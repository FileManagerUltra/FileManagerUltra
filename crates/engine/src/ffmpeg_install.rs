//! App-managed ffmpeg and ffprobe: a pinned, hash-verified download into the app's data folder.
//!
//! The pin (URL, exact size, SHA-256, archive members) is compiled in. Nothing is unpacked
//! unless the downloaded bytes match it, only the two expected members are extracted (to
//! fixed names, never to archive-supplied paths), and files are renamed into place.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use fm_vfs::FmError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

/// Hard cap on any download, whatever the pin says.
pub const MAX_DOWNLOAD_BYTES: u64 = 192 * 1024 * 1024;
/// Hard cap on one extracted file.
pub const MAX_MEMBER_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MANIFEST: &str = "manifest.json";

/// An exact, reviewed download. Bump it only after checking the publisher's own checksum.
pub struct Pin {
    pub version: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// `(path inside the top-level folder of the archive, installed file name)`.
    pub members: &'static [(&'static str, &'static str)],
}

/// gyan.dev "essentials" 9.0.2 (GPL build, run as a separate process, never linked). The hash
/// equals the one gyan.dev publishes at /ffmpeg/builds/ffmpeg-release-essentials.zip.sha256.
pub const WINDOWS: Pin = Pin {
    version: "9.0.2",
    url: "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
    size: 114_768_076,
    sha256: "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
    members: &[
        ("bin/ffmpeg.exe", "ffmpeg.exe"),
        ("bin/ffprobe.exe", "ffprobe.exe"),
    ],
};

/// The pin for this platform; Linux relies on the distribution package and macOS on Homebrew.
pub fn pin() -> Option<&'static Pin> {
    cfg!(windows).then_some(&WINDOWS)
}

/// Where downloaded tools live: `<data dir>/tools`.
pub fn tools_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", fm_brand::APP_SLUG)
        .map(|dirs| dirs.data_dir().join("tools"))
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: String,
    archive_sha256: String,
    files: BTreeMap<String, String>,
}

/// The installed version if the manifest is intact and every file is still there.
pub fn installed_version(dir: &Path, pin: &Pin) -> Option<String> {
    let path = dir.join(MANIFEST);
    if std::fs::metadata(&path).ok()?.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    pin.members
        .iter()
        .all(|(_, name)| dir.join(name).is_file() && manifest.files.contains_key(*name))
        .then_some(manifest.version)
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn io(context: &str, error: std::io::Error) -> String {
    format!("{context}: {error}")
}

/// Stream `reader` into `out`, enforcing the size cap while reading and the hash afterwards.
fn download(
    mut reader: impl Read,
    pin: &Pin,
    out: &mut impl Write,
    progress: &AtomicU64,
) -> Result<(), String> {
    let cap = pin.size.min(MAX_DOWNLOAD_BYTES);
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| io("The download was interrupted", e))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > cap {
            return Err("The download is larger than expected and was rejected.".into());
        }
        let chunk = buffer.get(..n).unwrap_or_default();
        hasher.update(chunk);
        out.write_all(chunk)
            .map_err(|e| io("Cannot write the download", e))?;
        progress.store(total, Ordering::Relaxed);
    }
    if total != pin.size || !hex(hasher.finalize()).eq_ignore_ascii_case(pin.sha256) {
        return Err(
            "The download does not match the expected checksum and was rejected; nothing was installed."
                .into(),
        );
    }
    Ok(())
}

/// The installed name when `entry` is exactly `<top folder>/<member>`.
fn member_name(entry: &str, pin: &Pin) -> Option<&'static str> {
    if entry.contains(['\\', '\0']) {
        return None;
    }
    let (top, rest) = entry.split_once('/')?;
    if top.is_empty() || top == "." || top == ".." {
        return None;
    }
    pin.members
        .iter()
        .find(|(path, _)| *path == rest)
        .map(|(_, name)| *name)
}

struct Extracted {
    name: &'static str,
    file: NamedTempFile,
    sha256: String,
}

fn extract(archive: std::fs::File, pin: &Pin, dir: &Path) -> Result<Vec<Extracted>, String> {
    let mut zip = zip::ZipArchive::new(archive)
        .map_err(|e| format!("The download is not a readable archive: {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err("The archive has too many entries and was rejected.".into());
    }
    let mut found: Vec<Extracted> = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("The archive is damaged: {e}"))?;
        let is_symlink = entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170_000 == 0o120_000);
        if entry.is_dir() || is_symlink {
            continue;
        }
        let Some(name) = member_name(entry.name(), pin) else {
            continue;
        };
        if found.iter().any(|f| f.name == name) {
            return Err("The archive lists a tool twice and was rejected.".into());
        }
        if entry.size() > MAX_MEMBER_BYTES {
            return Err("A file in the archive is too large and was rejected.".into());
        }
        let mut file =
            NamedTempFile::new_in(dir).map_err(|e| io("Cannot create a temporary file", e))?;
        let mut hasher = Sha256::new();
        let mut limited = (&mut entry).take(MAX_MEMBER_BYTES + 1);
        let mut total = 0u64;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = limited
                .read(&mut buffer)
                .map_err(|e| io("Cannot unpack the archive", e))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_MEMBER_BYTES {
                return Err("A file in the archive is too large and was rejected.".into());
            }
            let chunk = buffer.get(..n).unwrap_or_default();
            hasher.update(chunk);
            file.write_all(chunk)
                .map_err(|e| io("Cannot write an unpacked file", e))?;
        }
        found.push(Extracted {
            name,
            file,
            sha256: hex(hasher.finalize()),
        });
    }
    if found.len() != pin.members.len() {
        return Err("The archive does not contain ffmpeg and ffprobe.".into());
    }
    Ok(found)
}

/// Download, verify, unpack and atomically install the pinned tools into `dir`.
/// `open` returns the response body for the pin's URL.
pub fn install_with(
    dir: &Path,
    pin: &Pin,
    progress: &AtomicU64,
    open: impl FnOnce(&str) -> Result<Box<dyn Read + Send>, String>,
) -> Result<(), String> {
    create_private_dir(dir).map_err(|e| io("Cannot create the tools folder", e))?;
    let mut archive =
        NamedTempFile::new_in(dir).map_err(|e| io("Cannot create a temporary file", e))?;
    progress.store(0, Ordering::Relaxed);
    download(open(pin.url)?, pin, &mut archive, progress)?;
    let reopened = archive
        .reopen()
        .map_err(|e| io("Cannot read the download", e))?;
    let extracted = extract(reopened, pin, dir)?;
    drop(archive);

    let mut files = BTreeMap::new();
    for item in extracted {
        make_executable(item.file.path()).map_err(|e| io("Cannot make the tool runnable", e))?;
        files.insert(item.name.to_string(), item.sha256);
        item.file.persist(dir.join(item.name)).map_err(|e| {
            format!(
                "Cannot replace {}: {}. Stop any playing video and try again.",
                item.name, e.error
            )
        })?;
    }
    let manifest = Manifest {
        version: pin.version.into(),
        archive_sha256: pin.sha256.into(),
        files,
    };
    let text = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("Cannot write the install record: {e}"))?;
    let mut record =
        NamedTempFile::new_in(dir).map_err(|e| io("Cannot create a temporary file", e))?;
    record
        .write_all(&text)
        .map_err(|e| io("Cannot write the install record", e))?;
    record
        .persist(dir.join(MANIFEST))
        .map_err(|e| io("Cannot write the install record", e.error))?;
    Ok(())
}

fn open_https(url: &str) -> Result<Box<dyn Read + Send>, String> {
    if !url.starts_with("https://") {
        return Err("Refusing a download that is not served over https.".into());
    }
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() != "https" {
                attempt.error("redirect is not https")
            } else {
                attempt.follow()
            }
        }))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30 * 60))
        .user_agent(format!(
            "{}/{}",
            fm_brand::APP_SLUG,
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|e| format!("Cannot create the download client: {e}"))?;
    let response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("Cannot download ffmpeg: {e}"))?;
    Ok(Box::new(response))
}

type InstallResult = Result<(), String>;

/// Session state for the managed install.
#[derive(Debug)]
pub struct Ffmpeg {
    dir: Option<PathBuf>,
    installed: Option<String>,
    pending: Option<Receiver<InstallResult>>,
    progress: Arc<AtomicU64>,
    pub error: Option<String>,
    /// A video could not play because ffmpeg is missing; the app shows the install dialog.
    pub prompt: bool,
}

impl Default for Ffmpeg {
    fn default() -> Self {
        let dir = tools_dir();
        if let Some(dir) = &dir {
            fm_preview::video::set_managed_tools_dir(dir.clone());
        }
        let installed = pin()
            .zip(dir.as_deref())
            .and_then(|(pin, dir)| installed_version(dir, pin));
        Self {
            dir,
            installed,
            pending: None,
            progress: Arc::new(AtomicU64::new(0)),
            error: None,
            prompt: false,
        }
    }
}

impl Ffmpeg {
    /// Whether this platform has a pinned download at all.
    pub fn downloadable(&self) -> bool {
        pin().is_some() && self.dir.is_some()
    }

    /// The folder holding app-downloaded tools.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn installing(&self) -> bool {
        self.pending.is_some()
    }

    pub fn installed_version(&self) -> Option<&str> {
        self.installed.as_deref()
    }

    pub fn pinned_version(&self) -> Option<&'static str> {
        pin().map(|pin| pin.version)
    }

    pub fn update_available(&self) -> bool {
        match (self.installed.as_deref(), pin()) {
            (Some(installed), Some(pin)) => installed != pin.version,
            _ => false,
        }
    }

    /// Bytes received and the expected total.
    pub fn progress(&self) -> (u64, u64) {
        (
            self.progress.load(Ordering::Relaxed),
            pin().map_or(0, |pin| pin.size),
        )
    }

    pub fn start(&mut self) -> Result<(), FmError> {
        let (Some(pin), Some(dir)) = (pin(), self.dir.clone()) else {
            return Err(FmError::backend(
                "ffmpeg",
                "ffmpeg cannot be downloaded on this platform; install it with your package manager.",
            ));
        };
        if self.installing() {
            return Ok(());
        }
        let progress = self.progress.clone();
        let (tx, rx) = bounded(1);
        std::thread::Builder::new()
            .name("ffmpeg-install".into())
            .spawn(move || {
                let _ = tx.send(install_with(&dir, pin, &progress, open_https));
            })
            .map_err(|e| FmError::backend("ffmpeg", format!("Cannot start the download: {e}")))?;
        self.pending = Some(rx);
        self.error = None;
        Ok(())
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => Err("The download stopped unexpectedly.".into()),
        };
        self.pending = None;
        match result {
            Ok(()) => {
                self.installed = pin()
                    .zip(self.dir.as_deref())
                    .and_then(|(pin, dir)| installed_version(dir, pin));
                self.prompt = false;
            }
            Err(message) => self.error = Some(message),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .expect("start");
            writer.write_all(data).expect("write");
        }
        writer.finish().expect("finish").into_inner()
    }

    /// A pin that matches `bytes`.
    fn pin_for(bytes: &[u8]) -> Pin {
        let sha: &'static str = Box::leak(hex(Sha256::digest(bytes)).into_boxed_str());
        Pin {
            version: "1.0",
            url: "https://example.invalid/ffmpeg.zip",
            size: bytes.len() as u64,
            sha256: sha,
            members: &[("bin/ffmpeg", "ffmpeg"), ("bin/ffprobe", "ffprobe")],
        }
    }

    fn run(dir: &Path, pin: &Pin, served: Vec<u8>) -> Result<(), String> {
        install_with(dir, pin, &AtomicU64::new(0), |_| {
            Ok(Box::new(Cursor::new(served)))
        })
    }

    fn good() -> Vec<u8> {
        zip_with(&[
            ("ffmpeg-1.0/bin/ffmpeg", b"FFMPEG"),
            ("ffmpeg-1.0/bin/ffprobe", b"FFPROBE"),
            ("ffmpeg-1.0/README", b"ignored"),
        ])
    }

    fn only_manifest_or_nothing(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    }

    #[test]
    fn verified_archive_installs_only_the_expected_files() {
        let dir = tempfile::tempdir().expect("dir");
        let bytes = good();
        let pin = pin_for(&bytes);
        run(dir.path(), &pin, bytes).expect("install");
        assert_eq!(
            std::fs::read(dir.path().join("ffmpeg")).expect("a"),
            b"FFMPEG"
        );
        assert_eq!(
            std::fs::read(dir.path().join("ffprobe")).expect("b"),
            b"FFPROBE"
        );
        assert!(!dir.path().join("README").exists());
        assert_eq!(installed_version(dir.path(), &pin).as_deref(), Some("1.0"));
        // No temporary leftovers beyond the three installed files.
        assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 3);
    }

    #[test]
    fn a_wrong_hash_installs_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        let bytes = good();
        let pin = pin_for(&bytes);
        let mut tampered = bytes.clone();
        if let Some(byte) = tampered.get_mut(10) {
            *byte ^= 0xff;
        }
        let error = run(dir.path(), &pin, tampered).expect_err("rejected");
        assert!(error.contains("checksum"), "{error}");
        assert!(only_manifest_or_nothing(dir.path()));
    }

    #[test]
    fn an_oversized_download_is_cut_off() {
        let dir = tempfile::tempdir().expect("dir");
        let bytes = good();
        let pin = pin_for(&bytes);
        let mut long = bytes;
        long.extend(std::iter::repeat_n(0u8, 4096));
        let error = run(dir.path(), &pin, long).expect_err("rejected");
        assert!(error.contains("larger"), "{error}");
        assert!(only_manifest_or_nothing(dir.path()));
    }

    #[test]
    fn hostile_members_are_ignored_or_rejected() {
        for entries in [
            // Wrong depth, traversal and absolute names never match a member.
            vec![
                ("../bin/ffmpeg", &b"x"[..]),
                ("/bin/ffmpeg", b"x"),
                ("a/b/bin/ffmpeg", b"x"),
                ("a\\bin\\ffmpeg", b"x"),
                ("a/bin/ffprobe", b"y"),
            ],
            // A duplicate tool.
            vec![
                ("a/bin/ffmpeg", b"x"),
                ("b/bin/ffmpeg", b"x"),
                ("a/bin/ffprobe", b"y"),
            ],
            // A missing tool.
            vec![("a/bin/ffmpeg", b"x")],
        ] {
            let dir = tempfile::tempdir().expect("dir");
            let bytes = zip_with(&entries);
            let pin = pin_for(&bytes);
            assert!(run(dir.path(), &pin, bytes).is_err());
            assert!(only_manifest_or_nothing(dir.path()));
        }
    }

    #[test]
    fn reinstalling_replaces_the_old_files() {
        let dir = tempfile::tempdir().expect("dir");
        let old = zip_with(&[("v/bin/ffmpeg", b"OLD"), ("v/bin/ffprobe", b"OLD")]);
        run(dir.path(), &pin_for(&old), old).expect("first");
        let new = good();
        let pin = pin_for(&new);
        run(dir.path(), &pin, new).expect("second");
        assert_eq!(
            std::fs::read(dir.path().join("ffmpeg")).expect("read"),
            b"FFMPEG"
        );
    }

    #[test]
    fn a_missing_file_makes_the_install_look_absent() {
        let dir = tempfile::tempdir().expect("dir");
        let bytes = good();
        let pin = pin_for(&bytes);
        run(dir.path(), &pin, bytes).expect("install");
        std::fs::remove_file(dir.path().join("ffprobe")).expect("remove");
        assert_eq!(installed_version(dir.path(), &pin), None);
    }

    #[test]
    fn non_https_sources_are_refused() {
        assert!(open_https("http://example.invalid/x.zip").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_tools_folder_and_binaries_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("dir");
        let dir = root.path().join("tools");
        let bytes = good();
        run(&dir, &pin_for(&bytes), bytes).expect("install");
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("ffmpeg")), 0o700);
    }

    #[test]
    fn the_shipped_pin_is_well_formed() {
        assert!(WINDOWS.url.starts_with("https://"));
        assert_eq!(WINDOWS.sha256.len(), 64);
        assert!(WINDOWS.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        const { assert!(WINDOWS.size <= MAX_DOWNLOAD_BYTES) };
    }

    /// `FM_FFMPEG_ARCHIVE=/path/to/the/pinned.zip cargo test -p fm-engine -- --ignored real_pin`
    #[test]
    #[ignore = "needs the real 115 MB archive"]
    fn real_pin_archive_installs() {
        let Ok(path) = std::env::var("FM_FFMPEG_ARCHIVE") else {
            return;
        };
        let dir = tempfile::tempdir().expect("dir");
        install_with(dir.path(), &WINDOWS, &AtomicU64::new(0), |_| {
            Ok(Box::new(
                std::fs::File::open(path).map_err(|e| e.to_string())?,
            ))
        })
        .expect("install");
        assert_eq!(
            installed_version(dir.path(), &WINDOWS).as_deref(),
            Some("9.0.2")
        );
        assert!(
            dir.path()
                .join("ffprobe.exe")
                .metadata()
                .expect("meta")
                .len()
                > 1_000_000
        );
    }
}
