//! Live directory watching (capability `directory-cache`).
//!
//! One non-recursive `notify` watch per visible local directory. The OS callback
//! never queues events: it only marks the already-watched directory dirty in a
//! small shared set (bounded by [`MAX_WATCHES`]), so a flood of events costs no
//! extra memory. [`Watcher::drain`] debounces dirty directories; the session then
//! re-reads them through the job lanes and never patches the cache.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fm_vfs::VfsUri;
use notify::{EventKind, RecursiveMode, Watcher as _};

/// A directory is re-read at most this long after its first unprocessed event.
pub const DEBOUNCE: Duration = Duration::from_millis(100);
/// Cap on concurrent watches; further directories fall back to polling.
pub const MAX_WATCHES: usize = 64;
/// Event paths inspected per OS event (events carry one or two in practice).
const MAX_EVENT_PATHS: usize = 4;
const RETRY_INITIAL: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);

#[derive(Clone, Copy)]
struct Retry {
    at: Instant,
    delay: Duration,
}

impl Retry {
    fn after_failure(previous: Option<Self>, now: Instant) -> Self {
        let delay = previous.map_or(RETRY_INITIAL, |r| (r.delay * 2).min(RETRY_MAX));
        Self {
            at: now + delay,
            delay,
        }
    }
}

type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Shared {
    watched: HashSet<PathBuf>,
    dirty: HashSet<PathBuf>,
    rescan: bool,
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|p| p.into_inner())
}

/// Canonical map key: Windows paths ignore case and the `\\?\` prefix.
fn key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        let text = path.to_string_lossy();
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        PathBuf::from(text.to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// Mounts whose native events are unreliable or absent: polled instead.
pub fn is_non_event_path(path: &str, network_mounts: &[String]) -> bool {
    if cfg!(windows) && path.starts_with(r"\\") && !path.starts_with(r"\\?\") {
        return true;
    }
    let candidate = key(Path::new(path));
    network_mounts
        .iter()
        .any(|mount| candidate.starts_with(key(Path::new(mount))))
}

/// Filesystem types that do not deliver local change notifications.
pub fn is_network_filesystem(fs: &str) -> bool {
    let fs = fs.to_ascii_lowercase();
    fs.starts_with("nfs")
        || fs.starts_with("cifs")
        || fs.starts_with("smb")
        || fs.starts_with("afp")
        || fs.starts_with("9p")
        || fs.starts_with("fuse.")
        || matches!(fs.as_str(), "afs" | "ceph" | "webdav" | "davfs" | "sshfs")
}

pub struct Watcher {
    inner: Option<notify::RecommendedWatcher>,
    started: bool,
    startup_error: Option<String>,
    startup_retry: Option<Retry>,
    shared: Arc<Mutex<Shared>>,
    waker: Arc<Mutex<Option<Waker>>>,
    dirs: HashMap<PathBuf, VfsUri>,
    retries: HashMap<VfsUri, Retry>,
    pending: HashMap<VfsUri, Instant>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher")
            .field("active", &self.inner.is_some())
            .field("startup_error", &self.startup_error.is_some())
            .field("watched", &self.dirs.len())
            .field("retries", &self.retries.len())
            .finish()
    }
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Watcher {
    pub fn new() -> Self {
        Self {
            inner: None,
            started: false,
            startup_error: None,
            startup_retry: None,
            shared: Arc::default(),
            waker: Arc::default(),
            dirs: HashMap::new(),
            retries: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// Create the OS watcher on first use, so idle sessions hold no OS handles.
    fn start(&mut self, now: Instant) {
        if self.started || self.startup_retry.is_some_and(|retry| now < retry.at) {
            return;
        }
        self.started = true;
        let cb_shared = self.shared.clone();
        let cb_waker = self.waker.clone();
        match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let marked = match res {
                Ok(event) => mark(&cb_shared, &event),
                // The backend lost events or failed: everything watched is suspect.
                Err(_) => {
                    lock(&cb_shared).rescan = true;
                    true
                }
            };
            if marked && let Some(wake) = cb_waker.lock().unwrap_or_else(|p| p.into_inner()).clone()
            {
                wake();
            }
        }) {
            Ok(watcher) => {
                self.inner = Some(watcher);
                self.startup_retry = None;
            }
            Err(err) => {
                self.started = false;
                if self.startup_retry.is_none() {
                    self.startup_error = Some(err.to_string());
                }
                self.startup_retry = Some(Retry::after_failure(self.startup_retry, now));
            }
        }
    }

    /// Turn OS watching off; visible folders are polled instead (also a test seam).
    pub fn disable(&mut self) {
        self.started = true;
        self.inner = None;
        self.startup_retry = None;
        self.retries.clear();
        self.pending.clear();
        self.dirs.clear();
        let mut shared = lock(&self.shared);
        shared.watched.clear();
        shared.dirty.clear();
        shared.rescan = false;
    }

    /// Called from the OS thread when a watched directory changes, so an idle UI wakes.
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.waker.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(wake));
    }

    /// Whether debounced work is waiting (the app must keep ticking).
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn watched(&self) -> usize {
        self.dirs.len()
    }

    /// Whether `uri` is covered by a live OS watch (so it needs no polling).
    pub fn covers(&self, uri: &VfsUri) -> bool {
        uri.local_path()
            .is_some_and(|p| self.dirs.contains_key(&key(&p)))
    }

    /// Stop trusting the watch of `uri` (its directory vanished) so the next
    /// [`Watcher::sync`] re-creates it or falls back to polling.
    pub fn forget(&mut self, uri: &VfsUri) {
        let Some(path) = uri.local_path() else { return };
        self.unwatch(&key(&path));
        self.pending.remove(uri);
    }

    /// Make the watched set equal `want` (the visible local directories). Returns a
    /// notice when a watch could not be created and polling takes over.
    pub fn sync(&mut self, want: &[VfsUri], network_mounts: &[String]) -> Option<String> {
        self.sync_at(want, network_mounts, Instant::now())
    }

    fn sync_at(
        &mut self,
        want: &[VfsUri],
        network_mounts: &[String],
        now: Instant,
    ) -> Option<String> {
        let wanted: HashMap<PathBuf, &VfsUri> = want
            .iter()
            .filter_map(|u| u.local_path().map(|p| (key(&p), u)))
            .collect();
        let stale: Vec<PathBuf> = self
            .dirs
            .keys()
            .filter(|k| !wanted.contains_key(*k))
            .cloned()
            .collect();
        for k in stale {
            self.unwatch(&k);
        }
        self.retries.retain(|u, _| want.contains(u));
        self.pending.retain(|u, _| want.contains(u));
        if !wanted.is_empty() {
            self.start(now);
        }
        let mut notice = None;
        for (k, uri) in wanted {
            if self.dirs.contains_key(&k) {
                continue;
            }
            let Some(path) = uri.local_path() else {
                continue;
            };
            if is_non_event_path(&path.to_string_lossy(), network_mounts)
                || self.dirs.len() >= MAX_WATCHES
                || self.retries.get(uri).is_some_and(|retry| now < retry.at)
            {
                continue;
            }
            let Some(inner) = self.inner.as_mut() else {
                if let Some(err) = self.startup_error.take() {
                    notice = Some(format!(
                        "Live refresh is unavailable ({err}); folders refresh periodically instead."
                    ));
                }
                continue;
            };
            lock(&self.shared).watched.insert(k.clone());
            match inner.watch(&path, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    self.retries.remove(uri);
                    self.dirs.insert(k, uri.clone());
                    // The cached listing may predate this watch (e.g. the pane's workspace
                    // was inactive), so re-read once now that changes are being tracked.
                    self.pending.entry(uri.clone()).or_insert(now);
                }
                Err(err) => {
                    lock(&self.shared).watched.remove(&k);
                    let previous = self.retries.get(uri).copied();
                    self.retries
                        .insert(uri.clone(), Retry::after_failure(previous, now));
                    // A vanished folder is not a watcher problem; the read reports it.
                    if previous.is_none() && !matches!(err.kind, notify::ErrorKind::PathNotFound) {
                        notice = Some(format!(
                            "Live refresh is unavailable for some folders ({err}); they refresh periodically instead."
                        ));
                    }
                }
            }
        }
        notice
    }

    /// Directories whose debounce window elapsed and need a re-read.
    pub fn drain(&mut self, now: Instant) -> Vec<VfsUri> {
        let (dirty, rescan) = {
            let mut shared = lock(&self.shared);
            (
                std::mem::take(&mut shared.dirty),
                std::mem::take(&mut shared.rescan),
            )
        };
        if rescan {
            for uri in self.dirs.values() {
                self.pending.entry(uri.clone()).or_insert(now);
            }
        }
        for k in dirty {
            if let Some(uri) = self.dirs.get(&k) {
                self.pending.entry(uri.clone()).or_insert(now);
            }
        }
        let due: Vec<VfsUri> = self
            .pending
            .iter()
            .filter(|(_, at)| now.saturating_duration_since(**at) >= DEBOUNCE)
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in &due {
            self.pending.remove(uri);
        }
        due
    }

    fn unwatch(&mut self, k: &Path) {
        if let (Some(uri), Some(inner)) = (self.dirs.remove(k), self.inner.as_mut())
            && let Some(path) = uri.local_path()
        {
            let _ = inner.unwatch(&path);
        }
        let mut shared = lock(&self.shared);
        shared.watched.remove(k);
        shared.dirty.remove(k);
    }
}

/// Mark the watched directories an OS event touches. Returns whether any was marked.
/// Events for unknown paths are ignored; nothing is queued.
fn mark(shared: &Mutex<Shared>, event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    let mut guard = lock(shared);
    if event.need_rescan() || event.paths.len() > MAX_EVENT_PATHS {
        guard.rescan = true;
        return true;
    }
    let mut marked = false;
    for path in event.paths.iter().take(MAX_EVENT_PATHS) {
        let own = key(path);
        let parent = path.parent().map(key);
        for candidate in std::iter::once(own).chain(parent) {
            if guard.watched.contains(&candidate) {
                guard.dirty.insert(candidate);
                marked = true;
            }
        }
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(path: &std::path::Path) -> VfsUri {
        VfsUri::parse(&path.to_string_lossy()).expect("local uri")
    }

    fn wait_due(w: &mut Watcher, uri: &VfsUri) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if w.drain(Instant::now()).contains(uri) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn create_in_watched_dir_becomes_due() {
        let dir = tempfile::tempdir().expect("tmp");
        let u = uri(dir.path());
        let mut w = Watcher::new();
        assert!(w.sync(std::slice::from_ref(&u), &[]).is_none());
        assert!(w.covers(&u));
        std::fs::write(dir.path().join("a.txt"), b"x").expect("write");
        assert!(wait_due(&mut w, &u));
    }

    #[test]
    fn unwatched_dirs_do_not_fire_and_sync_unwatches() {
        let dir = tempfile::tempdir().expect("tmp");
        let u = uri(dir.path());
        let mut w = Watcher::new();
        w.sync(std::slice::from_ref(&u), &[]);
        w.sync(&[], &[]);
        assert_eq!(w.watched(), 0);
        std::fs::write(dir.path().join("a.txt"), b"x").expect("write");
        std::thread::sleep(Duration::from_millis(300));
        assert!(w.drain(Instant::now()).is_empty());
    }

    #[test]
    fn burst_collapses_to_one_pending_entry() {
        let dir = tempfile::tempdir().expect("tmp");
        let u = uri(dir.path());
        let mut w = Watcher::new();
        w.sync(std::slice::from_ref(&u), &[]);
        for i in 0..2000 {
            std::fs::write(dir.path().join(format!("f{i}")), b"").expect("write");
        }
        std::thread::sleep(Duration::from_millis(300));
        w.drain(Instant::now());
        assert!(w.pending.len() <= 1);
        assert!(lock(&w.shared).dirty.len() <= MAX_WATCHES);
    }

    #[test]
    fn watch_count_is_capped_and_falls_back() {
        let root = tempfile::tempdir().expect("tmp");
        let uris: Vec<VfsUri> = (0..MAX_WATCHES + 3)
            .map(|i| {
                let p = root.path().join(format!("d{i}"));
                std::fs::create_dir(&p).expect("mkdir");
                uri(&p)
            })
            .collect();
        let mut w = Watcher::new();
        w.sync(&uris, &[]);
        assert!(w.watched() <= MAX_WATCHES);
        assert_eq!(w.watched(), MAX_WATCHES);
        assert_eq!(uris.iter().filter(|u| !w.covers(u)).count(), 3);
    }

    #[test]
    fn network_mounts_fall_back_to_polling() {
        let dir = tempfile::tempdir().expect("tmp");
        let u = uri(dir.path());
        let mut w = Watcher::new();
        let mount = dir.path().to_string_lossy().into_owned();
        w.sync(std::slice::from_ref(&u), &[mount]);
        assert!(!w.covers(&u));
        assert!(
            w.retries.is_empty(),
            "network polling is not a failed watch"
        );
        w.sync(std::slice::from_ref(&u), &[]);
        assert!(w.covers(&u), "reconsider when mount classification changes");
    }

    #[test]
    fn missing_dir_falls_back_without_notice() {
        let dir = tempfile::tempdir().expect("tmp");
        let gone = dir.path().join("gone");
        let u = uri(&gone);
        let mut w = Watcher::new();
        assert!(w.sync(std::slice::from_ref(&u), &[]).is_none());
        assert!(!w.covers(&u));
    }

    #[test]
    fn oversized_events_rescan_all_watches_even_when_relevant_path_is_last() {
        let mut w = Watcher::new();
        let a = VfsUri::parse("/watched/a").expect("uri");
        let b = VfsUri::parse("/watched/b").expect("uri");
        w.dirs.insert(PathBuf::from("/watched/a"), a.clone());
        w.dirs.insert(PathBuf::from("/watched/b"), b.clone());
        let mut event = notify::Event::new(EventKind::Create(notify::event::CreateKind::File));
        for i in 0..MAX_EVENT_PATHS {
            event = event.add_path(PathBuf::from(format!("/unrelated/{i}")));
        }
        event = event.add_path(PathBuf::from("/watched/b/new.txt"));
        assert!(mark(&w.shared, &event));
        assert!(lock(&w.shared).rescan);
        assert!(lock(&w.shared).dirty.is_empty());
        let now = Instant::now();
        assert!(w.drain(now).is_empty());
        let due = w.drain(now + DEBOUNCE);
        assert_eq!(due.len(), 2);
        assert!(due.contains(&a) && due.contains(&b));
        assert!(w.drain(now + DEBOUNCE * 2).is_empty());
    }

    #[test]
    fn missing_directory_retries_with_backoff_and_recovers_without_navigation() {
        let root = tempfile::tempdir().expect("tmp");
        let path = root.path().join("recover");
        let u = uri(&path);
        let mut w = Watcher::new();
        let now = Instant::now();
        assert!(w.sync_at(std::slice::from_ref(&u), &[], now).is_none());
        let first = *w.retries.get(&u).expect("retry");
        assert_eq!(first.at, now + RETRY_INITIAL);
        assert!(w.sync_at(std::slice::from_ref(&u), &[], first.at).is_none());
        let second = *w.retries.get(&u).expect("retry");
        assert_eq!(second.delay, RETRY_INITIAL * 2);
        std::fs::create_dir(&path).expect("recreate");
        w.sync_at(std::slice::from_ref(&u), &[], first.at);
        assert!(!w.covers(&u), "must not retry every frame");
        w.sync_at(std::slice::from_ref(&u), &[], second.at);
        assert!(w.covers(&u));
        assert!(!w.retries.contains_key(&u));
        std::fs::write(path.join("new.txt"), b"x").expect("write");
        assert!(wait_due(&mut w, &u));
        w.sync_at(&[], &[], second.at);
        assert!(w.retries.is_empty());
        assert!(!w.covers(&u));
    }

    #[test]
    fn retry_delay_is_exponential_and_capped() {
        let now = Instant::now();
        let mut retry = Retry::after_failure(None, now);
        for seconds in [1, 2, 4, 8, 16, 32, 60, 60] {
            assert_eq!(retry.delay, Duration::from_secs(seconds));
            assert_eq!(retry.at, now + retry.delay);
            retry = Retry::after_failure(Some(retry), now);
        }
    }

    #[test]
    fn leaving_a_failed_watch_discards_its_backoff() {
        let root = tempfile::tempdir().expect("tmp");
        let u = uri(&root.path().join("missing"));
        let mut w = Watcher::new();
        let now = Instant::now();
        w.sync_at(std::slice::from_ref(&u), &[], now);
        assert!(w.retries.contains_key(&u));
        w.sync_at(&[], &[], now);
        assert!(w.retries.is_empty());
        std::fs::create_dir(root.path().join("missing")).expect("recreate");
        w.sync_at(std::slice::from_ref(&u), &[], now);
        assert!(w.covers(&u), "returning starts a fresh attempt");
    }

    #[test]
    fn watcher_startup_waits_for_backoff_but_disabled_watching_never_restarts() {
        let root = tempfile::tempdir().expect("tmp");
        let u = uri(root.path());
        let now = Instant::now();
        let mut w = Watcher::new();
        let retry = Retry::after_failure(None, now);
        w.startup_retry = Some(retry);
        w.sync_at(std::slice::from_ref(&u), &[], now);
        assert!(w.inner.is_none());
        w.sync_at(std::slice::from_ref(&u), &[], retry.at);
        assert!(w.covers(&u));
        assert!(w.startup_retry.is_none());
        w.disable();
        w.sync_at(std::slice::from_ref(&u), &[], retry.at + RETRY_MAX);
        assert!(w.inner.is_none());
        assert!(!w.covers(&u));
    }

    #[test]
    fn watch_cap_fallback_recovers_when_a_slot_is_freed() {
        let root = tempfile::tempdir().expect("tmp");
        let uris: Vec<_> = (0..=MAX_WATCHES)
            .map(|i| {
                let path = root.path().join(format!("dir{i}"));
                std::fs::create_dir(&path).expect("mkdir");
                uri(&path)
            })
            .collect();
        let mut w = Watcher::new();
        w.sync(&uris, &[]);
        let uncovered = uris
            .iter()
            .find(|u| !w.covers(u))
            .expect("over cap")
            .clone();
        let removed = uris.iter().find(|u| w.covers(u)).expect("covered");
        let remaining: Vec<_> = uris.iter().filter(|u| *u != removed).cloned().collect();
        w.sync(&remaining, &[]);
        assert!(w.covers(&uncovered));
        assert_eq!(w.watched(), MAX_WATCHES);
    }

    #[test]
    fn hostile_event_paths_are_ignored() {
        let shared = Mutex::new(Shared::default());
        let event = notify::Event::new(EventKind::Create(notify::event::CreateKind::File))
            .add_path(PathBuf::from("/nowhere/at/all"))
            .add_path(PathBuf::new());
        assert!(!mark(&shared, &event));
        assert!(lock(&shared).dirty.is_empty());
    }

    #[test]
    fn network_filesystem_names() {
        assert!(is_network_filesystem("nfs4"));
        assert!(is_network_filesystem("cifs"));
        assert!(is_network_filesystem("fuse.sshfs"));
        assert!(!is_network_filesystem("ext4"));
    }
}
