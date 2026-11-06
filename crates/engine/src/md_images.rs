//! Images of the Markdown preview (capability `preview-pane`, `security`).
//!
//! The UI never loads anything: it asks [`MdImageService::state`] what it may draw and the
//! `preview.md_images.want` command queues what is missing. Relative images are read through
//! the document's own backend on a local pool; `http`/`https` images (only with the
//! `preview_remote_images` preference) are fetched on a separate network pool so a slow
//! server never delays local files. Everything is bounded: source size, decoded size,
//! redirects, time, count and cache bytes. Remote hosts must resolve to public addresses.

use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, Sender};
use fm_preview::thumbs::Thumb;
use fm_preview::{Limits, Preview, image_prev};
use fm_vfs::{Entry, EntryKind, FmError, Vfs, VfsUri};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::preview_link_cmds::{Link, classify, resolve};
use crate::registry::{CommandSpec, Params};

/// Source bytes of one image.
pub const MAX_BYTES: usize = 8 << 20;
/// Decoded images kept in memory (least recently drawn go first).
pub const MEMORY_BYTES: usize = 256 << 20;
const MAX_ENTRIES: usize = 256;
const MAX_URLS: usize = 200;
const MAX_URL: usize = 2048;
const MAX_REDIRECTS: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(10);
const WORKERS: usize = 2;

type Key = (VfsUri, Option<SystemTime>, String);

/// What the preview may draw for one image.
#[derive(Clone, Debug)]
pub enum MdImageState {
    /// Remote image while the preference is off.
    Blocked,
    Pending,
    Ready(Arc<Thumb>),
    /// Why it cannot be shown (missing, too large, undecodable, refused address, ...).
    Failed(String),
}

#[derive(Debug)]
enum Slot {
    Pending,
    Ready(Arc<Thumb>),
    Failed(String),
}

#[derive(Debug)]
struct Item {
    slot: Slot,
    used: u64,
}

#[derive(Debug, Default)]
struct Store {
    map: HashMap<Key, Item>,
    bytes: usize,
    tick: u64,
}

impl Store {
    fn evict(&mut self, cap: usize) {
        while self.bytes > cap || self.map.len() > MAX_ENTRIES {
            let oldest = self
                .map
                .iter()
                .filter(|(_, i)| !matches!(i.slot, Slot::Pending))
                .min_by_key(|(_, i)| i.used)
                .map(|(k, _)| k.clone());
            let Some(key) = oldest else { break };
            if let Some(Item {
                slot: Slot::Ready(t),
                ..
            }) = self.map.remove(&key)
            {
                self.bytes = self.bytes.saturating_sub(t.bytes());
            }
        }
    }
}

enum Target {
    Local(VfsUri),
    Remote(String),
}

struct Job {
    key: Key,
    target: Target,
    vfs: Arc<dyn Vfs>,
}

struct Inner {
    store: Mutex<Store>,
    pending: AtomicUsize,
    done: AtomicU64,
    seen: AtomicU64,
    /// Tests reach a loopback server; production never sets it.
    allow_private: bool,
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn finish(&self, key: &Key, result: Result<Thumb, String>) {
        let mut st = self.lock();
        st.tick += 1;
        let used = st.tick;
        let slot = match result {
            Ok(t) => {
                st.bytes += t.bytes();
                Slot::Ready(Arc::new(t))
            }
            Err(why) => Slot::Failed(why),
        };
        st.map.insert(key.clone(), Item { slot, used });
        st.evict(MEMORY_BYTES);
        drop(st);
        self.done.fetch_add(1, Ordering::Release);
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }

    fn run(&self, job: &Job) {
        let loaded = catch_unwind(AssertUnwindSafe(|| match &job.target {
            Target::Local(uri) => load_local(&*job.vfs, uri),
            Target::Remote(url) => fetch(url, self.allow_private).and_then(|b| decode(&b)),
        }))
        .unwrap_or_else(|_| Err("The image could not be decoded".into()));
        self.finish(&job.key, loaded);
    }
}

fn worker(inner: &Inner, rx: &Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        inner.run(&job);
    }
}

#[derive(Default)]
struct Pools {
    local: Option<Sender<Job>>,
    net: Option<Sender<Job>>,
}

pub struct MdImageService {
    inner: Arc<Inner>,
    pools: Mutex<Pools>,
}

impl std::fmt::Debug for MdImageService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdImageService")
            .field("pending", &self.inner.pending.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for MdImageService {
    fn default() -> Self {
        Self::new(false)
    }
}

/// Where an image URL points, or why it must not be loaded.
fn target_of(document: &VfsUri, url: &str) -> Result<Target, String> {
    if url.len() > MAX_URL {
        return Err("The image address is too long".into());
    }
    match classify(url) {
        Ok(Link::External(u))
            if u.len() > 7
                && !u
                    .get(..7)
                    .is_some_and(|s| s.eq_ignore_ascii_case("mailto:")) =>
        {
            Ok(Target::Remote(u.to_string()))
        }
        Ok(Link::External(_)) => Err("Only http, https and relative images are loaded".into()),
        Ok(Link::Relative(rel)) => resolve(document, rel)
            .map(Target::Local)
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

impl MdImageService {
    fn new(allow_private: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: Mutex::new(Store::default()),
                pending: AtomicUsize::new(0),
                done: AtomicU64::new(0),
                seen: AtomicU64::new(0),
                allow_private,
            }),
            pools: Mutex::new(Pools::default()),
        }
    }

    /// What may be drawn for `url`; `None` when it is loadable but was not requested yet.
    pub fn state(&self, document: &Entry, url: &str, allow_remote: bool) -> Option<MdImageState> {
        let target = match target_of(&document.uri, url) {
            Ok(t) => t,
            Err(why) => return Some(MdImageState::Failed(why)),
        };
        if matches!(target, Target::Remote(_)) && !allow_remote {
            return Some(MdImageState::Blocked);
        }
        let mut st = self.inner.lock();
        st.tick += 1;
        let tick = st.tick;
        let item = st
            .map
            .get_mut(&(document.uri.clone(), document.modified, url.to_string()))?;
        item.used = tick;
        Some(match &item.slot {
            Slot::Pending => MdImageState::Pending,
            Slot::Ready(t) => MdImageState::Ready(t.clone()),
            Slot::Failed(why) => MdImageState::Failed(why.clone()),
        })
    }

    /// Queue every loadable, not yet known image of `document`.
    pub fn want(
        &self,
        document: &Entry,
        vfs: &Arc<dyn Vfs>,
        urls: &[String],
        allow_remote: bool,
        backends: &crate::backends::VfsRegistry,
    ) {
        for url in urls.iter().take(MAX_URLS) {
            let Ok(target) = target_of(&document.uri, url) else {
                continue;
            };
            let (job_vfs, remote) = match &target {
                Target::Remote(_) if !allow_remote => continue,
                Target::Remote(_) => (vfs.clone(), true),
                Target::Local(uri) => match backends.require(uri) {
                    Ok(v) => (v, false),
                    Err(e) => {
                        let key = (document.uri.clone(), document.modified, url.clone());
                        let mut st = self.inner.lock();
                        st.tick += 1;
                        let used = st.tick;
                        st.map.insert(
                            key,
                            Item {
                                slot: Slot::Failed(e.to_string()),
                                used,
                            },
                        );
                        continue;
                    }
                },
            };
            let key = (document.uri.clone(), document.modified, url.clone());
            {
                let mut st = self.inner.lock();
                if st.map.contains_key(&key) {
                    continue;
                }
                st.tick += 1;
                let used = st.tick;
                st.map.insert(
                    key.clone(),
                    Item {
                        slot: Slot::Pending,
                        used,
                    },
                );
            }
            self.inner.pending.fetch_add(1, Ordering::AcqRel);
            let job = Job {
                key: key.clone(),
                target,
                vfs: job_vfs,
            };
            let sent = self.sender(remote).is_some_and(|tx| tx.send(job).is_ok());
            if !sent {
                self.inner
                    .finish(&key, Err("The image loader is not running".into()));
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.inner.pending.load(Ordering::Acquire) > 0
    }

    /// Whether any image finished since the last call (the UI should repaint).
    pub fn take_changed(&self) -> bool {
        let done = self.inner.done.load(Ordering::Acquire);
        self.inner.seen.swap(done, Ordering::AcqRel) != done
    }

    fn sender(&self, remote: bool) -> Option<Sender<Job>> {
        let mut pools = self.pools.lock().ok()?;
        let slot = if remote {
            &mut pools.net
        } else {
            &mut pools.local
        };
        if slot.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded::<Job>();
            for n in 0..WORKERS {
                let (inner, rx) = (self.inner.clone(), rx.clone());
                std::thread::Builder::new()
                    .name(format!(
                        "fm-mdimg-{}-{n}",
                        if remote { "net" } else { "local" }
                    ))
                    .spawn(move || worker(&inner, &rx))
                    .ok()?;
            }
            *slot = Some(tx);
        }
        slot.clone()
    }
}

fn limits() -> Limits {
    Limits {
        image_bytes: MAX_BYTES as u64,
        ..Limits::default()
    }
}

fn decode(bytes: &[u8]) -> Result<Thumb, String> {
    if fm_preview::svg::looks_like_svg(bytes) {
        return fm_preview::svg::rasterize(bytes, limits().image_edge);
    }
    let entry = Entry::new(
        VfsUri::parse("/image").map_err(|e| e.to_string())?,
        EntryKind::File,
    );
    let Preview::Image(doc) = image_prev::decode_still(&entry, bytes, &limits()) else {
        return Err("Not a supported image (png, jpeg, gif, webp, bmp, ico or svg)".into());
    };
    let (rgba, _) = doc
        .frames
        .into_iter()
        .next()
        .ok_or_else(|| "The image has no pixels".to_string())?;
    let thumb = Thumb {
        width: doc.width,
        height: doc.height,
        rgba,
    };
    thumb
        .valid()
        .then_some(thumb)
        .ok_or_else(|| "The image has no pixels".into())
}

fn load_local(vfs: &dyn Vfs, uri: &VfsUri) -> Result<Thumb, String> {
    let entry = vfs.stat(uri).map_err(|e| e.to_string())?;
    if !entry.is_file() {
        return Err("Not a file".into());
    }
    if entry.size.is_some_and(|s| s > MAX_BYTES as u64) {
        return Err("The image is larger than 8 MiB".into());
    }
    let mut buf = Vec::new();
    vfs.open_read(uri)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| FmError::from(e).to_string())?;
    if buf.len() > MAX_BYTES {
        return Err("The image is larger than 8 MiB".into());
    }
    decode(&buf)
}

/// Loopback, private, link-local, shared, multicast, unspecified and reserved addresses.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v6.segments();
            let embedded_v4 = s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0; 4];
            if embedded_v4 {
                let [a, b] = s[6].to_be_bytes();
                let [c, d] = s[7].to_be_bytes();
                return public_v4(Ipv4Addr::new(a, b, c, d));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || s[0] & 0xfe00 == 0xfc00
                || s[0] & 0xffc0 == 0xfe80
                || v6 == Ipv6Addr::LOCALHOST)
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0))
}

/// Literal addresses skip DNS, so they are checked here; names are checked by [`Guard`].
fn check_url(url: &reqwest::Url, allow_private: bool) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https images are loaded".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Image addresses with credentials are refused".into());
    }
    let host = url.host_str().ok_or("The image address has no host")?;
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<IpAddr>()
        && !allow_private
        && !is_public(ip)
    {
        return Err("The image host is not a public address".into());
    }
    Ok(())
}

/// Resolver that only ever hands out public addresses, so the connection cannot be pointed
/// at the local network, not even through DNS rebinding.
struct Guard {
    allow_private: bool,
}

impl Resolve for Guard {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let found: Vec<SocketAddr> = (host.as_str(), 0)
                .to_socket_addrs()?
                .filter(|a| allow_private || is_public(a.ip()))
                .collect();
            if found.is_empty() {
                return Err("The image host is not a public address".into());
            }
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

fn fetch(url: &str, allow_private: bool) -> Result<Vec<u8>, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "The image address is not valid")?;
    check_url(&parsed, allow_private)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(fm_brand::APP_NAME)
        .dns_resolver(Arc::new(Guard { allow_private }))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("Too many redirects");
            }
            match check_url(attempt.url(), allow_private) {
                Ok(()) => attempt.follow(),
                Err(why) => attempt.error(why),
            }
        }))
        .build()
        .map_err(|_| "The network client could not start")?;
    let response = client
        .get(parsed)
        .send()
        .map_err(|e| format!("Could not load the image: {}", short(&e)))?;
    if !response.status().is_success() {
        return Err(format!(
            "The server answered {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err("The image is larger than 8 MiB".into());
    }
    let mut buf = Vec::new();
    response
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Could not load the image: {e}"))?;
    if buf.len() > MAX_BYTES {
        return Err("The image is larger than 8 MiB".into());
    }
    Ok(buf)
}

/// Error text without the URL (it may hold a token in its query).
fn short(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_redirect() {
        "refused redirect".into()
    } else if e.is_connect() {
        "cannot connect".into()
    } else {
        "request failed".into()
    }
}

fn urls_param(params: &Params) -> Result<Vec<String>, FmError> {
    let bad = || FmError::InvalidParams("urls must be an array of at most 200 strings".into());
    let list = params
        .as_object()
        .and_then(|m| m.get("urls"))
        .and_then(|v| v.as_array())
        .ok_or_else(bad)?;
    if list.len() > MAX_URLS {
        return Err(bad());
    }
    list.iter()
        .map(|v| v.as_str().map(str::to_string).ok_or_else(bad))
        .collect()
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.md_images.want",
        label: "Load Markdown Images",
        menu: &["Preview", "Load Markdown Images"],
        shortcut: None,
        params: "{urls: [string] (at most 200), pane?: number}: queue the images of the previewed Markdown file; relative paths, and http/https only when remote images are enabled",
        enabled: |_| true,
        run: |s, params| {
            let urls = urls_param(params)?;
            let pane = s.preview_pane(params)?;
            let document = s
                .preview_slot(pane)
                .and_then(|slot| slot.subject.clone())
                .ok_or_else(|| FmError::InvalidParams("Nothing is previewed".into()))?;
            let vfs = s.backends.require(&document.uri)?;
            s.md_images.want(
                &document,
                &vfs,
                &urls,
                s.prefs.preview_remote_images,
                &s.backends,
            );
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::from_pixel(w, h, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("png");
        out
    }

    fn doc_in(dir: &std::path::Path) -> Entry {
        let p = dir.join("README.md");
        std::fs::write(&p, "x").expect("doc");
        Entry::new(
            VfsUri::parse(&p.to_string_lossy()).expect("uri"),
            EntryKind::File,
        )
    }

    fn registry() -> crate::backends::VfsRegistry {
        crate::backends::VfsRegistry::with_local()
    }

    fn settle(svc: &MdImageService) {
        let end = std::time::Instant::now() + Duration::from_secs(20);
        while svc.is_busy() {
            assert!(std::time::Instant::now() < end, "images never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn want(svc: &MdImageService, doc: &Entry, urls: &[&str], remote: bool) {
        let reg = registry();
        let vfs = reg.require(&doc.uri).expect("vfs");
        let urls: Vec<String> = urls.iter().map(|u| (*u).to_string()).collect();
        svc.want(doc, &vfs, &urls, remote, &reg);
        settle(svc);
    }

    fn server(body: Vec<u8>, status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let Ok(mut stream) = stream else { continue };
                let mut req = [0u8; 1024];
                let _ = std::io::Read::read(&mut stream, &mut req);
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}/i.png")
    }

    #[test]
    fn public_address_rules() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
            "64:ff9b::7f00:1",
        ] {
            assert!(!is_public(bad.parse().expect("ip")), "{bad}");
        }
        for good in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(good.parse().expect("ip")), "{good}");
        }
    }

    #[test]
    fn literal_internal_hosts_and_credentials_are_refused() {
        for url in [
            "http://127.0.0.1/a.png",
            "http://[::1]/a.png",
            "http://169.254.169.254/latest",
            "http://2130706433/a.png",
            "http://192.168.0.1:8080/a.png",
            "https://user:pw@example.com/a.png",
            "ftp://example.com/a.png",
        ] {
            let parsed = reqwest::Url::parse(url).expect("url");
            assert!(check_url(&parsed, false).is_err(), "{url}");
        }
        let ok = reqwest::Url::parse("https://example.com/a.png").expect("url");
        assert!(check_url(&ok, false).is_ok());
    }

    #[test]
    fn urls_resolve_relative_or_remote_only() {
        let doc = VfsUri::parse("/tmp/docs/README.md").expect("uri");
        assert!(matches!(target_of(&doc, "img/a.png"), Ok(Target::Local(_))));
        assert!(matches!(
            target_of(&doc, "https://h/a.png"),
            Ok(Target::Remote(_))
        ));
        for bad in [
            "/etc/passwd",
            "file:///etc/passwd",
            "data:image/png;base64,AAAA",
            "//host/a.png",
            "mailto:a@b.c",
            "javascript:alert(1)",
            "#frag",
            "../../../../../../../../x.png",
        ] {
            assert!(target_of(&doc, bad).is_err(), "{bad}");
        }
        assert!(target_of(&doc, &"a".repeat(MAX_URL + 1)).is_err());
    }

    #[test]
    fn local_images_load_and_failures_are_reported() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir(dir.path().join("img")).expect("dir");
        std::fs::write(dir.path().join("img/a.png"), png(40, 20)).expect("png");
        std::fs::write(dir.path().join("bad.png"), b"not an image").expect("bad");
        let doc = doc_in(dir.path());
        let svc = MdImageService::default();
        assert!(svc.state(&doc, "img/a.png", false).is_none());
        want(
            &svc,
            &doc,
            &["img/a.png", "bad.png", "missing.png", "img/%61.png"],
            false,
        );
        let Some(MdImageState::Ready(t)) = svc.state(&doc, "img/a.png", false) else {
            panic!("ready")
        };
        assert_eq!((t.width, t.height), (40, 20));
        assert!(matches!(
            svc.state(&doc, "img/%61.png", false),
            Some(MdImageState::Ready(_))
        ));
        assert!(matches!(
            svc.state(&doc, "bad.png", false),
            Some(MdImageState::Failed(_))
        ));
        assert!(matches!(
            svc.state(&doc, "missing.png", false),
            Some(MdImageState::Failed(_))
        ));
        assert!(svc.take_changed());
    }

    #[test]
    fn oversized_sources_are_refused() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("big.png"), vec![0u8; MAX_BYTES + 1]).expect("big");
        let doc = doc_in(dir.path());
        let svc = MdImageService::default();
        want(&svc, &doc, &["big.png"], false);
        assert!(matches!(
            svc.state(&doc, "big.png", false),
            Some(MdImageState::Failed(_))
        ));
    }

    #[test]
    fn remote_images_are_blocked_without_the_preference_and_never_requested() {
        let dir = tempfile::tempdir().expect("tmp");
        let doc = doc_in(dir.path());
        let svc = MdImageService::new(true);
        let url = server(png(8, 8), "200 OK");
        want(&svc, &doc, &[&url], false);
        assert!(matches!(
            svc.state(&doc, &url, false),
            Some(MdImageState::Blocked)
        ));
        assert!(!svc.is_busy());
        // Nothing was stored: enabling the preference later loads it.
        assert!(svc.state(&doc, &url, true).is_none());
        want(&svc, &doc, &[&url], true);
        assert!(matches!(
            svc.state(&doc, &url, true),
            Some(MdImageState::Ready(_))
        ));
    }

    #[test]
    fn production_service_refuses_loopback_servers() {
        let dir = tempfile::tempdir().expect("tmp");
        let doc = doc_in(dir.path());
        let svc = MdImageService::default();
        let url = server(png(8, 8), "200 OK");
        want(&svc, &doc, &[&url], true);
        let Some(MdImageState::Failed(why)) = svc.state(&doc, &url, true) else {
            panic!("refused")
        };
        assert!(why.contains("not a public address"), "{why}");
    }

    #[test]
    fn remote_errors_and_oversize_do_not_crash() {
        let dir = tempfile::tempdir().expect("tmp");
        let doc = doc_in(dir.path());
        let svc = MdImageService::new(true);
        let missing = server(b"nope".to_vec(), "404 Not Found");
        let text = server(b"<html>".to_vec(), "200 OK");
        let big = server(vec![0u8; MAX_BYTES + 10], "200 OK");
        want(&svc, &doc, &[&missing, &text, &big], true);
        for url in [&missing, &text, &big] {
            assert!(
                matches!(svc.state(&doc, url, true), Some(MdImageState::Failed(_))),
                "{url}"
            );
        }
    }

    #[test]
    fn cache_is_bounded_by_bytes() {
        let mut st = Store::default();
        for n in 0..4u64 {
            let t = Arc::new(Thumb {
                width: 1000,
                height: 1000,
                rgba: vec![0; 4_000_000],
            });
            st.bytes += t.bytes();
            st.tick += 1;
            st.map.insert(
                (VfsUri::parse("/d").expect("uri"), None, n.to_string()),
                Item {
                    slot: Slot::Ready(t),
                    used: st.tick,
                },
            );
        }
        st.evict(9_000_000);
        assert_eq!(st.map.len(), 2);
        assert!(st.bytes <= 9_000_000);
    }

    #[test]
    fn want_command_rejects_malformed_params() {
        let mut s = crate::Session::default();
        let spec = &specs()[0];
        for bad in [
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!({"urls": "a.png"}),
            serde_json::json!({"urls": [1]}),
            serde_json::json!({"urls": vec!["a"; MAX_URLS + 1]}),
            serde_json::json!({"urls": ["a.png"]}),
        ] {
            assert!((spec.run)(&mut s, &bad).is_err(), "{bad}");
        }
    }
}
