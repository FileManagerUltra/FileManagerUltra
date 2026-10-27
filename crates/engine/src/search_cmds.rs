//! Find Files: a recursive name / content search below a folder.
//!
//! The walk runs on its own thread (job lanes are shared with directory reads), never
//! follows symlinks, skips hidden items unless asked, and is capped in results, depth and
//! bytes read per file. Results stream into a mutex the results window reads each frame.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use fm_layout::PaneId;
use fm_vfs::{Entry, EntryKind, FmError, Scheme, Vfs, VfsUri};

use crate::file_cmds::{focused_dir, object, parse_uri};
use crate::glob::{MAX_PATTERN_LEN, NameTest};
use crate::registry::CommandSpec;
use crate::session::{Session, is_hidden};

/// Most results kept; the search stops there.
pub const MAX_RESULTS: usize = 10_000;
/// Deepest folder level searched.
pub const MAX_DEPTH: usize = 64;
/// Files larger than this are not searched for content.
pub const MAX_CONTENT_BYTES: u64 = 64 * 1024 * 1024;
/// Longest content needle.
pub const MAX_NEEDLE: usize = 1024;
const CHUNK: usize = 64 * 1024;

/// What to look for (also the form's last values).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub root: Option<VfsUri>,
    /// Substring, or a pattern with `* ? [..]` (empty: any name).
    pub name: String,
    /// Text the file must contain (empty: no content test). Case-insensitive ASCII.
    pub content: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchStatus {
    pub found: usize,
    pub dirs: u64,
    pub files: u64,
    pub running: bool,
    /// Stopped at [`MAX_RESULTS`] (or by the user).
    pub truncated: bool,
    pub stopped: bool,
    /// Folder being scanned.
    pub current: String,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct Shared {
    results: Vec<Entry>,
    status: SearchStatus,
}

#[derive(Debug)]
struct Run {
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The Find Files window: the form and the current run.
#[derive(Debug)]
pub struct Search {
    pub query: SearchQuery,
    /// Files pane results are revealed in.
    pub origin: PaneId,
    run: Option<Run>,
}

fn lock(m: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Search {
    pub fn status(&self) -> SearchStatus {
        self.run
            .as_ref()
            .map(|r| lock(&r.shared).status.clone())
            .unwrap_or_default()
    }

    /// Clone of result `i` (the window asks only for the rows it shows).
    pub fn result(&self, i: usize) -> Option<Entry> {
        self.run
            .as_ref()
            .and_then(|r| lock(&r.shared).results.get(i).cloned())
    }
}

fn contains_ci(vfs: &dyn Vfs, uri: &VfsUri, needle: &[u8], cancel: &AtomicBool) -> bool {
    let Ok(mut reader) = vfs.open_read(uri) else {
        return false;
    };
    let mut buf = vec![0u8; CHUNK + needle.len()];
    let mut carry = 0usize;
    let mut total = 0u64;
    let mut first = true;
    loop {
        if cancel.load(Ordering::Relaxed) || total > MAX_CONTENT_BYTES {
            return false;
        }
        let Some(window) = buf.get_mut(carry..) else {
            return false;
        };
        let n = match reader.read(window) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        total += n as u64;
        let filled = carry + n;
        let Some(hay) = buf.get_mut(..filled) else {
            return false;
        };
        // Binary files (a NUL early on) are skipped like grep does.
        if first && hay.iter().take(8192).any(|b| *b == 0) {
            return false;
        }
        first = false;
        hay.make_ascii_lowercase();
        if hay.windows(needle.len()).any(|w| w == needle) {
            return true;
        }
        carry = (needle.len() - 1).min(filled);
        buf.copy_within(filled - carry..filled, 0);
    }
}

struct Walker<'a> {
    vfs: &'a dyn Vfs,
    name: Option<NameTest>,
    needle: Vec<u8>,
    hidden: bool,
    shared: &'a Mutex<Shared>,
    cancel: &'a AtomicBool,
}

impl Walker<'_> {
    fn run(&self, root: &VfsUri) {
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if self.cancel.load(Ordering::Relaxed) {
                lock(self.shared).status.stopped = true;
                return;
            }
            lock(self.shared).status.current = crate::item_cmds::display_path(&dir);
            let Ok(mut children) = self.vfs.read_dir(&dir) else {
                continue;
            };
            children.sort_by(|a, b| a.name.cmp(&b.name));
            let mut found = Vec::new();
            let (mut files, mut dirs) = (0u64, 0u64);
            let first_pushed = stack.len();
            for child in children {
                if !self.hidden && is_hidden(&child) {
                    continue;
                }
                let is_dir = child.kind == EntryKind::Dir;
                if is_dir {
                    dirs += 1;
                    if depth + 1 < MAX_DEPTH {
                        stack.push((child.uri.clone(), depth + 1));
                    }
                } else {
                    files += 1;
                }
                if self.name.as_ref().is_some_and(|t| !t.matches(&child.name)) {
                    continue;
                }
                if !self.needle.is_empty()
                    && (child.kind != EntryKind::File
                        || !contains_ci(self.vfs, &child.uri, &self.needle, self.cancel))
                {
                    continue;
                }
                found.push(child);
            }
            // Popped from the end: reverse so sub-folders are searched in name order.
            if let Some(new) = stack.get_mut(first_pushed..) {
                new.reverse();
            }
            let mut shared = lock(self.shared);
            shared.status.files += files;
            shared.status.dirs += dirs;
            let room = MAX_RESULTS.saturating_sub(shared.results.len());
            if found.len() > room {
                found.truncate(room);
                shared.status.truncated = true;
            }
            shared.results.extend(found);
            shared.status.found = shared.results.len();
            if shared.status.truncated {
                return;
            }
        }
    }
}

fn start(s: &mut Session, query: SearchQuery) -> Result<(), FmError> {
    let root = query
        .root
        .clone()
        .ok_or_else(|| FmError::InvalidParams("no folder to search".to_string()))?;
    if !matches!(root.scheme(), Scheme::File | Scheme::Sftp) {
        return Err(FmError::Unsupported(
            "Find Files works in local folders and on SFTP servers",
        ));
    }
    let vfs = s.backends.require(&root)?;
    let name = match query.name.trim() {
        "" => None,
        text => Some(NameTest::parse(text)?),
    };
    if query.content.len() > MAX_NEEDLE {
        return Err(FmError::InvalidParams(format!(
            "content text is limited to {MAX_NEEDLE} bytes"
        )));
    }
    if name.is_none() && query.content.is_empty() {
        return Err(FmError::InvalidParams(
            "enter a name or some text to look for".to_string(),
        ));
    }
    let needle = query.content.to_ascii_lowercase().into_bytes();
    let shared = Arc::new(Mutex::new(Shared::default()));
    lock(&shared).status.running = true;
    let cancel = Arc::new(AtomicBool::new(false));
    let (thread_shared, thread_cancel, hidden) = (shared.clone(), cancel.clone(), query.hidden);
    std::thread::Builder::new()
        .name("fm-search".into())
        .spawn(move || {
            let walker = Walker {
                vfs: vfs.as_ref(),
                name,
                needle,
                hidden,
                shared: &thread_shared,
                cancel: &thread_cancel,
            };
            walker.run(&root);
            let mut shared = lock(&thread_shared);
            shared.status.running = false;
            shared.status.current.clear();
        })
        .map_err(FmError::Io)?;
    let origin = s.layout.focused();
    let search = s.search.get_or_insert_with(|| Search {
        query: SearchQuery::default(),
        origin,
        run: None,
    });
    search.query = query;
    search.run = Some(Run { shared, cancel });
    Ok(())
}

fn text_param(
    map: Option<&crate::file_cmds::Obj>,
    key: &str,
    cap: usize,
) -> Result<String, FmError> {
    match map.and_then(|m| m.get(key)) {
        None => Ok(String::new()),
        Some(v) => {
            let t = v
                .as_str()
                .ok_or_else(|| FmError::InvalidParams(format!("{key} must be a string")))?;
            if t.len() > cap {
                return Err(FmError::InvalidParams(format!("{key} is too long")));
            }
            Ok(t.to_string())
        }
    }
}

/// `search.open` — show the Find Files window for a folder (default: the focused one).
pub fn open() -> CommandSpec {
    CommandSpec {
        id: "search.open",
        label: "Find Files…",
        menu: &["File", "Find Files…"],
        shortcut: Some("Alt+F7"),
        params: "root?: uri",
        enabled: |s| focused_dir(s).is_some(),
        run: |s, params| {
            let map = object(params)?;
            let root = match map.and_then(|m| m.get("root")) {
                Some(v) => Some(parse_uri(v)?),
                None => focused_dir(s),
            };
            let origin = s.layout.focused();
            let origin_is_files = s.layout.state(origin).is_some_and(|p| p.kind.is_files());
            let search = s.search.get_or_insert_with(|| Search {
                query: SearchQuery::default(),
                origin,
                run: None,
            });
            if origin_is_files {
                search.origin = origin;
            }
            if root.is_some() {
                search.query.root = root;
            }
            Ok(())
        },
    }
}

/// `search.start` — search below `root` for names and/or content.
pub fn start_cmd() -> CommandSpec {
    CommandSpec {
        id: "search.start",
        label: "Search",
        menu: &["File", "Find Files…", "Search"],
        shortcut: None,
        params: "name|content: string (at least one), root?: uri, hidden?: bool",
        enabled: |s| s.search.is_some() || focused_dir(s).is_some(),
        run: |s, params| {
            let map = object(params)?;
            let root = match map.and_then(|m| m.get("root")) {
                Some(v) => Some(parse_uri(v)?),
                None => s
                    .search
                    .as_ref()
                    .and_then(|x| x.query.root.clone())
                    .or_else(|| focused_dir(s)),
            };
            let hidden = match map.and_then(|m| m.get("hidden")) {
                None => false,
                Some(v) => v
                    .as_bool()
                    .ok_or_else(|| FmError::InvalidParams("hidden must be a boolean".into()))?,
            };
            let query = SearchQuery {
                root,
                name: text_param(map, "name", MAX_PATTERN_LEN)?,
                content: text_param(map, "content", MAX_NEEDLE)?,
                hidden,
            };
            start(s, query)
        },
    }
}

pub fn stop() -> CommandSpec {
    CommandSpec {
        id: "search.stop",
        label: "Stop Search",
        menu: &["File", "Find Files…", "Stop"],
        shortcut: None,
        params: "none",
        enabled: |s| s.search.as_ref().is_some_and(|x| x.status().running),
        run: |s, params| {
            object(params)?;
            if let Some(run) = s.search.as_ref().and_then(|x| x.run.as_ref()) {
                run.cancel.store(true, Ordering::Relaxed);
            }
            Ok(())
        },
    }
}

pub fn close() -> CommandSpec {
    CommandSpec {
        id: "search.close",
        label: "Close Find Files",
        menu: &["File", "Find Files…", "Close"],
        shortcut: None,
        params: "none",
        enabled: |s| s.search.is_some(),
        run: |s, params| {
            object(params)?;
            s.search = None;
            Ok(())
        },
    }
}

/// `file.reveal` — show an item in a Files pane: open its folder and select it.
pub fn reveal() -> CommandSpec {
    CommandSpec {
        id: "file.reveal",
        label: "Show in Folder",
        menu: &["File", "Show in Folder"],
        shortcut: None,
        params: "uri: string",
        enabled: |_| true,
        run: |s, params| {
            let uri = object(params)?
                .and_then(|m| m.get("uri"))
                .ok_or_else(|| FmError::InvalidParams("uri is required".to_string()))
                .and_then(parse_uri)?;
            let parent = uri
                .parent()
                .ok_or_else(|| FmError::InvalidParams("a root has no folder".to_string()))?;
            let name = uri.file_name().unwrap_or_default().to_string();
            s.backends.require(&parent)?;
            let is_files =
                |s: &Session, p: PaneId| s.layout.state(p).is_some_and(|x| x.kind.is_files());
            let pane = s
                .search
                .as_ref()
                .map(|x| x.origin)
                .filter(|p| is_files(s, *p))
                .or_else(|| Some(s.layout.focused()).filter(|p| is_files(s, *p)))
                .or_else(|| s.layout.panes().into_iter().find(|p| is_files(s, *p)))
                .ok_or_else(|| FmError::NotFound("a Files pane".to_string()))?;
            s.layout.focus_pane(pane)?;
            if let Some(state) = s.layout.state_mut(pane) {
                if state.location != parent {
                    state.navigate(parent.clone());
                }
                state.tree.active = false;
                state.listing.reveal = Some(name);
            }
            s.note_visit(&parent);
            s.ensure_read(parent)?;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![open(), start_cmd(), stop(), close(), reveal()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Params;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn finish(s: &Session) -> SearchStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let st = s.search.as_ref().expect("search").status();
            if !st.running {
                return st;
            }
            assert!(Instant::now() < deadline, "search never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn names(s: &Session) -> Vec<String> {
        let search = s.search.as_ref().expect("search");
        (0..search.status().found)
            .filter_map(|i| search.result(i).map(|e| e.name))
            .collect()
    }

    #[test]
    fn finds_names_and_content_recursively_but_not_hidden_or_binary() {
        let tmp = tempfile::tempdir().expect("tmp");
        let t = tmp.path();
        std::fs::create_dir_all(t.join("a/b")).expect("dirs");
        std::fs::create_dir_all(t.join(".git")).expect("dirs");
        std::fs::write(t.join("notes.txt"), b"Hello World").expect("w");
        std::fs::write(t.join("a/b/deep.TXT"), b"...hello...").expect("w");
        std::fs::write(t.join(".git/config.txt"), b"hello").expect("w");
        std::fs::write(t.join("a/blob.bin"), b"\0\0hello").expect("w");
        // A needle split across the 64 KiB chunk boundary is still found.
        let mut big = vec![b'x'; CHUNK - 3];
        big.extend_from_slice(b"NEEDLE");
        std::fs::write(t.join("a/big.log"), &big).expect("w");
        let root = VfsUri::parse(t.to_str().expect("utf8")).expect("uri");
        let mut s = Session::new().expect("session");

        s.dispatch(
            "search.start",
            &json!({"root": root.as_str(), "name": "*.txt"}),
        )
        .expect("start");
        finish(&s);
        assert_eq!(names(&s), ["notes.txt", "deep.TXT"]);

        s.dispatch(
            "search.start",
            &json!({"root": root.as_str(), "content": "HELLO"}),
        )
        .expect("start");
        finish(&s);
        let mut found = names(&s);
        found.sort();
        assert_eq!(found, ["deep.TXT", "notes.txt"]);

        s.dispatch(
            "search.start",
            &json!({"root": root.as_str(), "content": "hello", "hidden": true}),
        )
        .expect("start");
        assert!(names_after(&s).contains(&"config.txt".to_string()));

        s.dispatch(
            "search.start",
            &json!({"root": root.as_str(), "content": "needle"}),
        )
        .expect("start");
        assert_eq!(names_after(&s), ["big.log"]);
        let st = s.search.as_ref().expect("search").status();
        assert!(st.dirs >= 2 && st.files >= 3 && !st.truncated);

        s.dispatch("search.close", &Params::Null).expect("close");
        assert!(s.search.is_none());
    }

    fn names_after(s: &Session) -> Vec<String> {
        finish(s);
        names(s)
    }

    #[test]
    fn reveal_opens_the_folder_and_selects_the_item() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("x.txt"), b"x").expect("w");
        let file = VfsUri::parse(tmp.path().join("x.txt").to_str().expect("utf8")).expect("uri");
        let mut s = Session::new().expect("session");
        s.dispatch("file.reveal", &json!({"uri": file.as_str()}))
            .expect("reveal");
        let pane = s.layout.focused();
        let state = s.layout.state(pane).expect("pane");
        assert_eq!(Some(&state.location), file.parent().as_ref());
        assert_eq!(state.listing.reveal.as_deref(), Some("x.txt"));
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new().expect("session");
        for (id, p) in [
            ("search.start", json!({"root": "/tmp"})),
            ("search.start", json!({"root": "/tmp", "name": 3})),
            (
                "search.start",
                json!({"root": "/tmp", "name": "x", "hidden": "y"}),
            ),
            (
                "search.start",
                json!({"root": "/tmp", "content": "x".repeat(MAX_NEEDLE + 1)}),
            ),
            (
                "search.start",
                json!({"root": "smb://nas/share", "name": "x"}),
            ),
            ("search.open", json!({"root": 1})),
            ("file.reveal", json!({})),
            ("file.reveal", json!({"uri": "/"})),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
    }
}
