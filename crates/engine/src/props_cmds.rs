//! File properties: name, location, kind, size, dates and permissions of the selection,
//! a live folder-size count, and `file.chmod`.
//!
//! The folder walk runs on its own thread (never a job lane, which directory reads share)
//! and publishes running totals through a mutex the UI reads each frame. It never follows
//! symlinks, is capped in entries and depth, and stops when the dialog closes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use fm_vfs::{Entry, EntryKind, FmError, Scheme, Vfs, VfsUri};

use crate::file_cmds::{focused_dir, focused_selection, object, parse_sources};
use crate::fileops::OpRequest;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Entries visited before the count stops (shown as "more than").
pub const MAX_WALK_ENTRIES: u64 = 5_000_000;
/// Deepest folder level walked.
pub const MAX_WALK_DEPTH: usize = 256;

/// Running totals of the walk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropsInfo {
    /// `stat` of a single item whose parent listing is not cached.
    pub stat: Option<Entry>,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Folders that could not be read (permission denied, vanished…).
    pub unreadable: u64,
    pub done: bool,
    /// The walk hit [`MAX_WALK_ENTRIES`] or [`MAX_WALK_DEPTH`].
    pub truncated: bool,
    /// Why there is no folder count (e.g. SMB), or a stat error.
    pub note: Option<String>,
}

#[derive(Debug)]
pub struct Properties {
    pub items: Vec<VfsUri>,
    shared: Arc<Mutex<PropsInfo>>,
    cancel: Arc<AtomicBool>,
}

impl Properties {
    pub fn info(&self) -> PropsInfo {
        lock(&self.shared).clone()
    }
}

impl Drop for Properties {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn lock(m: &Mutex<PropsInfo>) -> std::sync::MutexGuard<'_, PropsInfo> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Session {
    /// The single item's entry: from its parent's cached listing (fresh after a chmod), else
    /// the walker's `stat`.
    pub fn properties_entry(&self) -> Option<Entry> {
        let p = self.properties.as_ref()?;
        let [uri] = p.items.as_slice() else {
            return None;
        };
        uri.parent()
            .and_then(|parent| self.dirs.entries(&parent))
            .and_then(|entries| entries.iter().find(|e| e.uri == *uri).cloned())
            .or_else(|| p.info().stat)
    }
}

/// Count one top-level item and, for a folder, everything below it.
fn walk(
    vfs: &dyn Vfs,
    root: &VfsUri,
    single: bool,
    shared: &Mutex<PropsInfo>,
    cancel: &AtomicBool,
) {
    let entry = match vfs.stat(root) {
        Ok(e) => e,
        Err(e) => {
            lock(shared).note = Some(e.to_string());
            return;
        }
    };
    {
        let mut info = lock(shared);
        if single {
            info.stat = Some(entry.clone());
        }
        match entry.kind {
            EntryKind::Dir => info.dirs += 1,
            _ => {
                info.files += 1;
                info.bytes += entry.size.unwrap_or(0);
            }
        }
    }
    if entry.kind != EntryKind::Dir {
        return;
    }
    let mut stack = vec![(root.clone(), 0usize)];
    let mut seen: u64 = 0;
    while let Some((dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let children = match vfs.read_dir(&dir) {
            Ok(c) => c,
            Err(_) => {
                lock(shared).unreadable += 1;
                continue;
            }
        };
        let (mut files, mut dirs, mut bytes) = (0u64, 0u64, 0u64);
        for child in children {
            seen += 1;
            match child.kind {
                EntryKind::Dir => {
                    dirs += 1;
                    if depth + 1 < MAX_WALK_DEPTH {
                        stack.push((child.uri, depth + 1));
                    } else {
                        lock(shared).truncated = true;
                    }
                }
                _ => {
                    files += 1;
                    bytes += child.size.unwrap_or(0);
                }
            }
        }
        let mut info = lock(shared);
        info.files += files;
        info.dirs += dirs;
        info.bytes += bytes;
        if seen >= MAX_WALK_ENTRIES {
            info.truncated = true;
            return;
        }
    }
}

fn start(s: &mut Session, items: Vec<VfsUri>) -> Result<(), FmError> {
    let shared = Arc::new(Mutex::new(PropsInfo::default()));
    let cancel = Arc::new(AtomicBool::new(false));
    // SMB handles stay on the SMB lane; there the dialog shows the cached listing only.
    let walkable: Vec<(Arc<dyn Vfs>, VfsUri)> = items
        .iter()
        .filter(|u| matches!(u.scheme(), Scheme::File | Scheme::Sftp))
        .filter_map(|u| s.backends.get_for(u).map(|vfs| (vfs, u.clone())))
        .collect();
    if walkable.len() < items.len() {
        let mut info = lock(&shared);
        info.note = Some("Folder contents are not counted on this kind of location".into());
        info.done = walkable.is_empty();
    }
    if !walkable.is_empty() {
        let single = items.len() == 1;
        let (thread_shared, thread_cancel) = (shared.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("fm-props".into())
            .spawn(move || {
                for (vfs, uri) in walkable {
                    if thread_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    walk(vfs.as_ref(), &uri, single, &thread_shared, &thread_cancel);
                }
                lock(&thread_shared).done = true;
            })
            .map_err(FmError::Io)?;
    }
    s.properties = Some(Properties {
        items,
        shared,
        cancel,
    });
    Ok(())
}

fn targets(s: &Session, params: &Params) -> Result<Vec<VfsUri>, FmError> {
    match object(params)?.and_then(|m| m.get("sources")) {
        Some(v) => parse_sources(v),
        None => {
            let sel = focused_selection(s);
            Ok(if sel.is_empty() {
                focused_dir(s).into_iter().collect()
            } else {
                sel
            })
        }
    }
}

/// `file.properties` — open the Properties dialog for the selection (or the folder).
pub fn properties() -> CommandSpec {
    CommandSpec {
        id: "file.properties",
        label: "Properties",
        menu: &["File", "Properties"],
        shortcut: Some("Alt+Enter"),
        params: "sources?: [uri] (default: the selection or else the folder)",
        enabled: |s| !focused_selection(s).is_empty() || focused_dir(s).is_some(),
        run: |s, params| {
            let items = targets(s, params)?;
            if items.is_empty() {
                return Err(FmError::InvalidParams("nothing to show".to_string()));
            }
            s.properties = None;
            start(s, items)
        },
    }
}

pub fn properties_close() -> CommandSpec {
    CommandSpec {
        id: "file.properties.close",
        label: "Close Properties",
        menu: &["File", "Close Properties"],
        shortcut: None,
        params: "none",
        enabled: |s| s.properties.is_some(),
        run: |s, params| {
            object(params)?;
            s.properties = None;
            Ok(())
        },
    }
}

/// Parse `mode` as a number (`493`) or an octal string (`"755"`, `"0o755"`).
pub fn parse_mode(value: &Params) -> Result<u32, FmError> {
    let bad = || FmError::InvalidParams("mode must be 0..=7777 (octal)".to_string());
    let mode = match value {
        Params::Number(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(bad)?,
        Params::String(text) => {
            let digits = text.trim().trim_start_matches("0o");
            if digits.is_empty() || digits.len() > 6 {
                return Err(bad());
            }
            u32::from_str_radix(digits, 8).map_err(|_| bad())?
        }
        _ => return Err(bad()),
    };
    if mode > 0o7777 {
        return Err(bad());
    }
    Ok(mode)
}

/// `file.chmod` — set the permission bits of the selection (not recursive).
pub fn chmod() -> CommandSpec {
    CommandSpec {
        id: "file.chmod",
        label: "Set Permissions",
        menu: &["File", "Set Permissions"],
        shortcut: None,
        params: "mode: octal string or number, sources?: [uri]",
        enabled: |s| s.properties.is_some() || !focused_selection(s).is_empty(),
        run: |s, params| {
            let map = object(params)?;
            let mode = parse_mode(
                map.and_then(|m| m.get("mode"))
                    .ok_or_else(|| FmError::InvalidParams("mode is required".to_string()))?,
            )?;
            let sources = match map.and_then(|m| m.get("sources")) {
                Some(v) => parse_sources(v)?,
                None => match &s.properties {
                    Some(p) => p.items.clone(),
                    None => focused_selection(s),
                },
            };
            if sources.is_empty() {
                return Err(FmError::InvalidParams("nothing selected".to_string()));
            }
            s.submit_op(OpRequest::Chmod { sources, mode }).map(|_| ())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![properties(), properties_close(), chmod()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn settle(s: &mut Session, until: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !until(s) {
            assert!(Instant::now() < deadline, "timed out");
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn folder_sizes_are_counted_without_following_links() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("a/b")).expect("dirs");
        std::fs::write(root.join("x.bin"), [0u8; 100]).expect("w");
        std::fs::write(root.join("a/b/y.bin"), [0u8; 23]).expect("w");
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path(), root.join("loop")).expect("link");
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(root.to_str().expect("utf8")).expect("uri");
        s.dispatch("file.properties", &json!({"sources": [uri.as_str()]}))
            .expect("open");
        settle(&mut s, |s| {
            s.properties.as_ref().is_some_and(|p| p.info().done)
        });
        let info = s.properties.as_ref().expect("open").info();
        assert_eq!(info.dirs, 3, "root, a, b");
        assert_eq!(info.bytes, 123);
        assert_eq!(
            info.files,
            if cfg!(unix) { 3 } else { 2 },
            "the link counts, its target not"
        );
        assert_eq!(s.properties_entry().map(|e| e.kind), Some(EntryKind::Dir));
        s.dispatch("file.properties.close", &Params::Null)
            .expect("close");
        assert!(s.properties.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn chmod_sets_the_mode_of_the_dialog_items() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("run.sh");
        std::fs::write(&file, b"x").expect("w");
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(file.to_str().expect("utf8")).expect("uri");
        s.dispatch("file.properties", &json!({"sources": [uri.as_str()]}))
            .expect("open");
        s.dispatch("file.chmod", &json!({"mode": "750"}))
            .expect("chmod");
        let mode = || std::fs::metadata(&file).map(|m| m.permissions().mode() & 0o7777);
        settle(&mut s, |_| mode().ok() == Some(0o750));
    }

    #[test]
    fn modes_parse_strictly() {
        assert_eq!(parse_mode(&json!("755")).ok(), Some(0o755));
        assert_eq!(parse_mode(&json!("0o644")).ok(), Some(0o644));
        assert_eq!(parse_mode(&json!(0o600)).ok(), Some(0o600));
        for bad in [
            json!("8"),
            json!("77777"),
            json!(-1),
            json!(0o10000),
            json!(""),
            json!(null),
        ] {
            assert!(parse_mode(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new().expect("session");
        for (id, p) in [
            ("file.properties", json!({"sources": 1})),
            ("file.properties", json!({"sources": []})),
            ("file.chmod", json!({})),
            ("file.chmod", json!({"mode": "999", "sources": ["/x"]})),
            ("file.chmod", json!({"mode": "644", "sources": []})),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
    }
}
