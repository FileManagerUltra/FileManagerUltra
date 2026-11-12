//! Opening files with the system: `file.open` hands a local file to the default
//! application, or runs it when it is an executable and the caller confirmed.
//! Directories navigate instead. Nothing here goes through a shell.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fm_vfs::{Entry, EntryKind, FmError, Scheme, VfsUri};

use crate::backends::lane_for;
use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Unix,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) { Os::Windows } else { Os::Unix }
    }
}

/// What activating an entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    /// Navigate into it.
    Dir,
    /// Starts a program: running it needs an explicit confirmation.
    Executable,
    /// Handed to the default application.
    Document,
}

/// Extensions that make an executable bit meaningful. Removable and network drives often
/// mark every file executable, so the bit alone must not turn documents into prompts.
const UNIX_RUNNABLE: &[&str] = &[
    "", "sh", "bash", "zsh", "fish", "py", "pl", "rb", "php", "js", "appimage", "run", "bin",
    "out", "command",
];
const WINDOWS_RUNNABLE: &[&str] = &[
    "exe",
    "bat",
    "cmd",
    "com",
    "msi",
    "ps1",
    "vbs",
    "vbe",
    "wsf",
    "wsh",
    "scr",
    "lnk",
    "hta",
    "jar",
    "reg",
    "msc",
    "cpl",
    "url",
    "pif",
    "js",
    "jse",
    "appref-ms",
    "msp",
    "gadget",
    "ps2",
    "psc1",
    "msh",
    "inf",
    "application",
];

pub fn classify(entry: &Entry, os: Os) -> OpenKind {
    let ext = Path::new(&entry.name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if entry.is_dir_like() {
        // A macOS application bundle is a directory that launches.
        return if ext == "app" && os == Os::Unix && cfg!(target_os = "macos") {
            OpenKind::Executable
        } else {
            OpenKind::Dir
        };
    }
    if !entry.is_file() && !entry.is_symlink() {
        return OpenKind::Document;
    }
    let runnable = match os {
        Os::Windows => WINDOWS_RUNNABLE.contains(&ext.as_str()),
        Os::Unix => {
            entry.permissions.is_some_and(|m| m & 0o111 != 0)
                && UNIX_RUNNABLE.contains(&ext.as_str())
        }
    };
    if runnable {
        OpenKind::Executable
    } else {
        OpenKind::Document
    }
}

/// How the app should react to an activated `uri` (double click or Enter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenPlan {
    Navigate,
    Launch,
    ConfirmRun { name: String },
}

/// Decide from the cached listing, without touching the filesystem.
pub fn plan(s: &mut Session, uri: &VfsUri) -> OpenPlan {
    let Some(entry) = known_entry(s, uri) else {
        return OpenPlan::Navigate;
    };
    match classify(&entry, Os::current()) {
        OpenKind::Dir => OpenPlan::Navigate,
        OpenKind::Document => OpenPlan::Launch,
        OpenKind::Executable => OpenPlan::ConfirmRun { name: entry.name },
    }
}

/// What is already known about `uri` without I/O: its parent's cached listing, else the
/// items and subjects of a loaded preview (a folder or multi-file grid shows entries the
/// panes have not listed).
fn known_entry(s: &Session, uri: &VfsUri) -> Option<Entry> {
    use crate::preview_cmds::Body;
    let cached = uri
        .parent()
        .and_then(|p| s.dirs.entries(&p))
        .and_then(|list| list.iter().find(|e| e.uri == *uri).cloned());
    cached.or_else(|| {
        s.previews.slots.values().find_map(|slot| {
            let from_doc = |doc: &fm_preview::Preview| match doc {
                fm_preview::Preview::Dir(d) => d.items.iter().find(|e| e.uri == *uri).cloned(),
                _ => None,
            };
            slot.subjects
                .iter()
                .chain(slot.subject.iter())
                .find(|e| e.uri == *uri)
                .cloned()
                .or_else(|| match &slot.body {
                    Body::Static(doc) => from_doc(doc),
                    Body::Grid(cells) => cells.iter().find_map(|(e, doc)| {
                        (e.uri == *uri).then(|| e.clone()).or_else(|| from_doc(doc))
                    }),
                    _ => None,
                })
        })
    })
}

pub(crate) fn local_path(uri: &VfsUri) -> Result<PathBuf, FmError> {
    if uri.scheme() != Scheme::File {
        return Err(FmError::Unsupported(
            "only local files can be opened; copy it to a local folder first",
        ));
    }
    let path = uri
        .local_path()
        .ok_or_else(|| FmError::invalid_uri(uri.to_string()))?;
    if !path.is_absolute() {
        return Err(FmError::invalid_uri(uri.to_string()));
    }
    Ok(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Default,
    Run,
}

/// Start `path`; returns once the program has been spawned.
fn launch(path: &Path, mode: Mode) -> Result<(), FmError> {
    if mode == Mode::Run && !cfg!(windows) {
        return spawn_direct(path);
    }
    opener::open(path).map_err(|e| FmError::backend("open", e.to_string()))
}

#[cfg(unix)]
fn spawn_direct(path: &Path) -> Result<(), FmError> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group: quitting the file manager must not kill it.
        .process_group(0);
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| FmError::backend("open", format!("cannot run it: {e}")))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(not(unix))]
fn spawn_direct(path: &Path) -> Result<(), FmError> {
    opener::open(path).map_err(|e| FmError::backend("open", e.to_string()))
}

/// `file.open {uri, mode?: "default"|"run", confirm?: bool}` — a directory navigates the
/// focused pane; a file opens in the default application. Executables only start with
/// `mode: "run", confirm: true`; `mode: "default"` opens them as documents instead.
pub fn open_file() -> CommandSpec {
    CommandSpec {
        id: "file.open",
        label: "Open File",
        menu: &["File", "Open"],
        shortcut: None,
        params: "{uri: string, mode?: \"default\"|\"run\", confirm?: bool}",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let map = object(params)?
                .ok_or_else(|| FmError::InvalidParams("expected {uri: string}".to_string()))?;
            let raw = map
                .get("uri")
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("uri must be a string".to_string()))?;
            let uri = VfsUri::parse(raw).map_err(|_| FmError::invalid_uri(raw.to_string()))?;
            let mode = match map.get("mode").and_then(Params::as_str) {
                None | Some("default") => Mode::Default,
                Some("run") => Mode::Run,
                Some(_) => {
                    return Err(FmError::InvalidParams(
                        "mode must be default or run".to_string(),
                    ));
                }
            };
            let confirmed = map
                .get("confirm")
                .and_then(Params::as_bool)
                .unwrap_or(false);

            let vfs = s.backends.require(&uri)?;
            let entry = vfs.stat(&uri)?;
            let kind = classify(&entry, Os::current());
            if kind == OpenKind::Dir {
                return s.dispatch("nav.open", &Params::String(uri.to_string()));
            }
            if matches!(entry.kind, EntryKind::Other) {
                return Err(FmError::Unsupported("special files are never opened"));
            }
            if mode == Mode::Run && !confirmed {
                return Err(FmError::InvalidParams(
                    "running a program requires confirm: true".to_string(),
                ));
            }
            if mode == Mode::Default && kind == OpenKind::Executable && !confirmed {
                // Opening a program "as a document" is the same hand-off to the OS
                // handler, which could start it; make the caller say so explicitly.
                return Err(FmError::InvalidParams(
                    "this file is a program: pass mode \"run\" with confirm: true, or confirm: true to open it with its default application".to_string(),
                ));
            }
            if uri.scheme() != Scheme::File {
                if mode == Mode::Run {
                    return Err(FmError::Unsupported(
                        "remote programs are never run; copy it to a local folder first",
                    ));
                }
                let name = safe_file_name(&entry.name);
                s.submit_op(crate::fileops::OpRequest::OpenRemote {
                    uri,
                    name,
                    size: entry.size,
                })?;
                return Ok(());
            }
            let path = local_path(&uri)?;
            s.jobs
                .submit(lane_for(uri.scheme()), move |_ctx| launch(&path, mode));
            Ok(())
        },
    }
}

/// Largest remote file fetched just to open it.
const MAX_OPEN_DOWNLOAD: u64 = 1 << 30;

/// A remote name reduced to one harmless path component.
pub(crate) fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.chars().take(200).collect()
    }
}

/// A new, uniquely named, owner-only folder in the system temp dir.
fn private_temp_dir() -> Result<PathBuf, FmError> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = format!(
        "fm-open-{}-{}-{nanos}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(name);
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        let mut builder = builder;
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    builder.create(&dir).map_err(FmError::Io)?;
    Ok(dir)
}

/// Remove the temp folders made by `file.open` on remote files. `own_only` (on quit) removes
/// only this process's; at startup everything older than a day goes, so crashes and viewers
/// still open from a recent session are left alone.
pub fn sweep_open_temp(own_only: bool) {
    let own = format!("fm-open-{}-", std::process::id());
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("fm-open-") {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(24 * 3600));
        if (own_only && name.starts_with(&own)) || (!own_only && stale) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Hand a downloaded copy to the default application.
pub(crate) fn launch_default(path: &Path) -> Result<(), FmError> {
    launch(path, Mode::Default)
}

/// Copy a remote file into a fresh private (0700) temp folder and return its path. The
/// copy is not executable, so the OS hands it to a viewer rather than running it.
pub(crate) fn download_to_temp(
    vfs: &dyn fm_vfs::Vfs,
    uri: &VfsUri,
    name: &str,
    size: Option<u64>,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, FmError> {
    use std::io::{Read, Write};

    if size.is_some_and(|n| n > MAX_OPEN_DOWNLOAD) {
        return Err(FmError::Unsupported(
            "file is too large to open directly; copy it to a local folder first",
        ));
    }
    let dir = private_temp_dir()?;
    let path = dir.join(name);
    let mut reader = vfs.open_read(uri)?.take(MAX_OPEN_DOWNLOAD);
    let mut out = std::fs::File::create(&path).map_err(FmError::Io)?;
    let total = size.unwrap_or(0);
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        if cancelled() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(FmError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(FmError::Io)?;
        if n == 0 {
            break;
        }
        out.write_all(buf.get(..n).unwrap_or_default())
            .map_err(FmError::Io)?;
        done += n as u64;
        progress(done, total);
    }
    out.flush().map_err(FmError::Io)?;
    // `take` stops silently at the cap; a file that long with no known size is cut short.
    if size.is_none() && done >= MAX_OPEN_DOWNLOAD {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(FmError::Unsupported(
            "file is too large to open directly; copy it to a local folder first",
        ));
    }
    Ok(path)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![open_file()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind, mode: Option<u32>) -> Entry {
        let uri = VfsUri::parse(&format!("/t/{name}")).expect("uri");
        let mut e = Entry::new(uri, kind);
        e.permissions = mode;
        e
    }

    #[test]
    fn remote_names_become_one_safe_component() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name(".."), "file");
        assert_eq!(safe_file_name("a\x1bb\\c"), "a_b_c");
    }

    #[test]
    fn a_remote_file_is_fetched_into_a_private_temp_folder() {
        use std::sync::mpsc;
        let mem = fm_testkit::MemFs::new();
        mem.put_str("/r/doc.txt", b"remote bytes").expect("put");
        let uri = VfsUri::parse("/r/doc.txt").expect("uri");
        let (tx, rx) = mpsc::channel();
        let jobs = crate::jobs::JobQueue::new();
        jobs.submit(crate::jobs::JobLane::Local, move |ctx| {
            let r = download_to_temp(
                &mem,
                &uri,
                "doc.txt",
                Some(12),
                &|| ctx.is_cancelled(),
                &mut |_, _| {},
            );
            let _ = tx.send(r);
            Ok(())
        });
        let path = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("job ran")
            .expect("download");
        assert_eq!(std::fs::read(&path).expect("read"), b"remote bytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = path.parent().expect("dir");
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode();
            assert_eq!(mode(dir) & 0o077, 0, "folder is owner-only");
            assert_eq!(mode(&path) & 0o111, 0, "copy is not executable");
        }
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn classification_per_os() {
        let f = |n, m| classify(&entry(n, EntryKind::File, m), Os::Unix);
        assert_eq!(f("run.sh", Some(0o755)), OpenKind::Executable);
        assert_eq!(f("tool", Some(0o755)), OpenKind::Executable);
        assert_eq!(f("x.AppImage", Some(0o700)), OpenKind::Executable);
        assert_eq!(f("run.sh", Some(0o644)), OpenKind::Document);
        assert_eq!(f("photo.jpg", Some(0o777)), OpenKind::Document);
        assert_eq!(f("notes.txt", None), OpenKind::Document);
        let w = |n| classify(&entry(n, EntryKind::File, None), Os::Windows);
        assert_eq!(w("Setup.EXE"), OpenKind::Executable);
        assert_eq!(w("a.ps1"), OpenKind::Executable);
        assert_eq!(w("x.hta"), OpenKind::Executable);
        assert_eq!(w("x.JAR"), OpenKind::Executable);
        assert_eq!(w("x.url"), OpenKind::Executable);
        assert_eq!(w("a.txt"), OpenKind::Document);
        assert_eq!(
            classify(&entry("d", EntryKind::Dir, Some(0o755)), Os::Unix),
            OpenKind::Dir
        );
        assert_eq!(
            classify(&entry("p", EntryKind::Other, None), Os::Unix),
            OpenKind::Document
        );
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = Session::new().expect("session");
        for p in [
            serde_json::json!(null),
            serde_json::json!("x"),
            serde_json::json!({}),
            serde_json::json!({"uri": 5}),
            serde_json::json!({"uri": "relative/path"}),
            serde_json::json!({"uri": "/nonexistent-fm-test", "mode": "nope"}),
            serde_json::json!({"uri": "/nonexistent-fm-test"}),
            serde_json::json!({"uri": "sftp://h/x"}),
        ] {
            assert!(s.dispatch("file.open", &p).is_err(), "{p}");
        }
    }

    #[test]
    fn files_shown_only_in_a_preview_open_instead_of_navigating() {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("a.txt"), "x").expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let mut s = Session::new().expect("session");
        let folder = VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
        let vfs = s.backends.require(&folder).expect("vfs");
        let subject = vfs.stat(&folder).expect("stat");
        let doc = fm_preview::dir::summarize(
            vfs.as_ref(),
            &subject,
            &fm_preview::Limits::default(),
            false,
        )
        .expect("summary");
        let mut slot = crate::preview_cmds::PreviewSlot::new();
        slot.subject = Some(subject);
        slot.body =
            crate::preview_cmds::Body::Static(std::sync::Arc::new(fm_preview::Preview::Dir(doc)));
        s.previews.slots.insert(1, slot);

        let file = folder.join("a.txt").expect("join");
        assert!(s.dirs.entries(&folder).is_none(), "not listed by any pane");
        assert_eq!(plan(&mut s, &file), OpenPlan::Launch);
        let sub = folder.join("sub").expect("join");
        assert_eq!(plan(&mut s, &sub), OpenPlan::Navigate);
    }

    #[cfg(unix)]
    #[test]
    fn executables_need_confirmation_and_then_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tmp");
        let marker = dir.path().join("ran");
        let script = dir.path().join("go.sh");
        std::fs::write(&script, "#!/bin/sh\ntouch ran\n").expect("write");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let mut s = Session::new().expect("session");
        let uri = script.to_string_lossy().to_string();

        for p in [
            serde_json::json!({"uri": uri}),
            serde_json::json!({"uri": uri, "mode": "run"}),
            serde_json::json!({"uri": uri, "mode": "run", "confirm": false}),
        ] {
            assert!(s.dispatch("file.open", &p).is_err(), "{p}");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!marker.exists(), "nothing may run without confirmation");

        s.dispatch(
            "file.open",
            &serde_json::json!({"uri": uri, "mode": "run", "confirm": true}),
        )
        .expect("run");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < end {
            s.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "the script ran in its own folder");
    }

    #[test]
    fn directories_navigate() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        s.dispatch(
            "file.open",
            &serde_json::json!({"uri": dir.path().to_string_lossy()}),
        )
        .expect("navigates");
        let loc = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .local_path()
            .expect("local");
        assert_eq!(loc, dir.path());
    }
}
