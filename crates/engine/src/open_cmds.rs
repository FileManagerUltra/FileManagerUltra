//! Opening files with the system: `file.open` hands a local file to the default
//! application, or runs it when it is an executable and the caller confirmed.
//! Directories navigate instead. Nothing here goes through a shell.

use std::path::{Path, PathBuf};

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
    "exe", "bat", "cmd", "com", "msi", "ps1", "vbs", "wsf", "scr", "lnk",
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
    let Some(entry) = uri
        .parent()
        .and_then(|p| s.dirs.entries(&p))
        .and_then(|list| list.iter().find(|e| e.uri == *uri).cloned())
    else {
        return OpenPlan::Navigate;
    };
    match classify(&entry, Os::current()) {
        OpenKind::Dir => OpenPlan::Navigate,
        OpenKind::Document => OpenPlan::Launch,
        OpenKind::Executable => OpenPlan::ConfirmRun { name: entry.name },
    }
}

fn local_path(uri: &VfsUri) -> Result<PathBuf, FmError> {
    if uri.scheme() != Scheme::File {
        return Err(FmError::Unsupported(
            "only local files can be opened; copy it to a local folder first",
        ));
    }
    let path = PathBuf::from(uri.path_str());
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
                return s.dispatch("nav.open", &Params::String(uri.path_str().to_string()));
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
            let path = local_path(&uri)?;
            s.jobs
                .submit(lane_for(uri.scheme()), move |_ctx| launch(&path, mode));
            Ok(())
        },
    }
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
            .path_str()
            .to_string();
        assert_eq!(loc, dir.path().to_string_lossy());
    }
}
