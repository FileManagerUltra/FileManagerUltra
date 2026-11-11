//! Find Files: a recursive name / content search below a folder.
//!
//! The walk runs on its own thread (job lanes are shared with directory reads), never
//! follows symlinks, skips hidden items unless asked, and is capped in results, depth and
//! bytes read per file. Results stream into a mutex the results window reads each frame.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

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

/// Recent searches kept in the preferences.
pub const MAX_HISTORY: usize = 20;
/// Longest folder kept in a remembered search.
const MAX_SAVED_ROOT: usize = 4096;

/// A remembered search: what the form held when it started.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedSearch {
    pub root: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub hidden: bool,
}

impl SavedSearch {
    /// Within the caps a search accepts, so a hand-edited file cannot smuggle more in.
    pub fn is_valid(&self) -> bool {
        self.root.len() <= MAX_SAVED_ROOT
            && VfsUri::parse(&self.root).is_ok()
            && self.name.len() <= MAX_PATTERN_LEN
            && self.content.len() <= MAX_NEEDLE
            && !(self.name.trim().is_empty() && self.content.is_empty())
    }

    /// One-line menu text: `"*.pdf" containing "invoice" in /home/alice`.
    pub fn label(&self) -> String {
        let clip = |text: &str| -> String {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            if text.chars().count() > 60 {
                format!("{}…", text.chars().take(60).collect::<String>())
            } else {
                text
            }
        };
        let mut parts = Vec::new();
        if !self.name.trim().is_empty() {
            parts.push(format!("\"{}\"", clip(self.name.trim())));
        }
        if !self.content.is_empty() {
            parts.push(format!("containing \"{}\"", clip(&self.content)));
        }
        let place = VfsUri::parse(&self.root)
            .map(|uri| crate::item_cmds::display_path(&uri))
            .unwrap_or_default();
        format!("{} in {}", parts.join(" "), clip(&place))
    }
}

impl Session {
    /// Remembered searches that are still valid, newest first.
    pub fn recent_searches(&self) -> Vec<SavedSearch> {
        self.prefs
            .search_history
            .iter()
            .filter(|saved| saved.is_valid())
            .take(MAX_HISTORY)
            .cloned()
            .collect()
    }

    fn remember_search(&mut self, query: &SearchQuery) {
        let Some(root) = &query.root else {
            return;
        };
        let saved = SavedSearch {
            root: root.to_string(),
            name: query.name.trim().to_string(),
            content: query.content.clone(),
            hidden: query.hidden,
        };
        if !saved.is_valid() {
            return;
        }
        let history = &mut self.prefs.search_history;
        history.retain(|old| *old != saved);
        history.insert(0, saved);
        history.truncate(MAX_HISTORY);
    }
}

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
    /// Hidden from view while the search goes on; the jobs panel shows it.
    pub minimized: bool,
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

/// Most threads a local search walks with.
const MAX_WALKERS: usize = 8;

/// Case-insensitive (ASCII) substring test over a stream, with SIMD search.
struct Needle {
    finder: memchr::memmem::Finder<'static>,
    len: usize,
}

impl Needle {
    fn new(text: &str) -> Option<Self> {
        let lower = text.to_ascii_lowercase().into_bytes();
        (!lower.is_empty()).then(|| Self {
            len: lower.len(),
            finder: memchr::memmem::Finder::new(&lower).into_owned(),
        })
    }

    /// Whether `reader` contains the needle; binary files (a NUL in the first 8 KiB) never
    /// match, like grep, and at most [`MAX_CONTENT_BYTES`] are read.
    fn in_reader(&self, mut reader: impl Read, cancel: &AtomicBool) -> bool {
        let mut buf = vec![0u8; CHUNK + self.len];
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
            if first && hay.iter().take(8192).any(|b| *b == 0) {
                return false;
            }
            first = false;
            // Only the new bytes need lower-casing; the carried tail already is.
            if let Some(fresh) = hay.get_mut(carry..) {
                fresh.make_ascii_lowercase();
            }
            if self.finder.find(hay).is_some() {
                return true;
            }
            carry = (self.len - 1).min(filled);
            buf.copy_within(filled - carry..filled, 0);
        }
    }
}

fn contains_ci(vfs: &dyn Vfs, uri: &VfsUri, needle: &Needle, cancel: &AtomicBool) -> bool {
    vfs.open_read(uri)
        .is_ok_and(|reader| needle.in_reader(reader, cancel))
}

struct Walker<'a> {
    vfs: &'a dyn Vfs,
    name: Option<NameTest>,
    needle: Option<Needle>,
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
                    if depth + 1 < MAX_DEPTH && !is_pseudo_folder(&child.uri) {
                        stack.push((child.uri.clone(), depth + 1));
                    }
                } else {
                    files += 1;
                }
                if self.name.as_ref().is_some_and(|t| !t.matches(&child.name)) {
                    continue;
                }
                if let Some(needle) = &self.needle
                    && (child.kind != EntryKind::File
                        || !contains_ci(self.vfs, &child.uri, needle, self.cancel))
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

/// Folders waiting to be searched by the local walkers.
struct Queue {
    dirs: Vec<(VfsUri, PathBuf, usize)>,
    busy: usize,
    done: bool,
}

/// Local-disk walk on several threads. Entry types come from the directory listing
/// itself (no `stat` per entry); only matching entries are stat'ed for size and date.
struct LocalWalk<'a> {
    vfs: &'a dyn Vfs,
    name: Option<NameTest>,
    needle: Option<Needle>,
    hidden: bool,
    shared: &'a Mutex<Shared>,
    cancel: &'a AtomicBool,
    queue: Mutex<Queue>,
    wake: Condvar,
}

fn lock_queue(m: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl LocalWalk<'_> {
    fn run(&self, root: &VfsUri) {
        let mut seed = Vec::new();
        if root.is_drive_list() {
            // Windows "This PC": every drive is a starting folder.
            for drive in self.vfs.read_dir(root).unwrap_or_default() {
                if let Some(path) = drive.uri.local_path() {
                    seed.push((drive.uri, path, 1));
                }
            }
        } else if let Some(path) = root.local_path() {
            seed.push((root.clone(), path, 0));
        }
        seed.reverse();
        lock_queue(&self.queue).dirs = seed;
        let workers = std::thread::available_parallelism()
            .map_or(2, std::num::NonZero::get)
            .clamp(1, MAX_WALKERS);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| self.worker());
            }
        });
    }

    fn worker(&self) {
        loop {
            let (uri, path, depth) = {
                let mut queue = lock_queue(&self.queue);
                loop {
                    if queue.done {
                        return;
                    }
                    if let Some(item) = queue.dirs.pop() {
                        queue.busy += 1;
                        break item;
                    }
                    if queue.busy == 0 {
                        queue.done = true;
                        self.wake.notify_all();
                        return;
                    }
                    queue = self
                        .wake
                        .wait(queue)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            };
            let (mut subdirs, stop) = self.dir(&uri, &path, depth);
            let mut queue = lock_queue(&self.queue);
            queue.busy -= 1;
            // Popped from the end: reversed so sub-folders are searched in name order.
            subdirs.reverse();
            queue.dirs.append(&mut subdirs);
            queue.done |= stop;
            self.wake.notify_all();
        }
    }

    /// Search one folder; returns its sub-folders and whether the whole walk must stop.
    fn dir(
        &self,
        uri: &VfsUri,
        path: &std::path::Path,
        depth: usize,
    ) -> (Vec<(VfsUri, PathBuf, usize)>, bool) {
        if self.cancel.load(Ordering::Relaxed) {
            lock(self.shared).status.stopped = true;
            return (Vec::new(), true);
        }
        let Ok(listing) = std::fs::read_dir(path) else {
            return (Vec::new(), false);
        };
        let mut children: Vec<_> = listing.filter_map(Result::ok).collect();
        children.sort_by_key(std::fs::DirEntry::file_name);
        let mut subdirs = Vec::new();
        let mut found = Vec::new();
        let (mut files, mut dirs) = (0u64, 0u64);
        for child in children {
            let Ok(kind) = child.file_type() else {
                continue;
            };
            let name = child.file_name().to_string_lossy().into_owned();
            if !self.hidden && (name.starts_with('.') || os_hidden(&child)) {
                continue;
            }
            if kind.is_dir() {
                dirs += 1;
                if depth + 1 < MAX_DEPTH
                    && let Ok(child_uri) = uri.join(&name)
                    && !is_pseudo_folder(&child_uri)
                {
                    subdirs.push((child_uri, child.path(), depth + 1));
                }
            } else {
                files += 1;
            }
            if self.name.as_ref().is_some_and(|t| !t.matches(&name)) {
                continue;
            }
            if let Some(needle) = &self.needle
                && (!kind.is_file()
                    || !std::fs::File::open(child.path())
                        .is_ok_and(|file| needle.in_reader(file, self.cancel)))
            {
                continue;
            }
            if let Ok(entry) = uri.join(&name).and_then(|u| self.vfs.stat(&u)) {
                found.push(entry);
            }
        }
        let mut shared = lock(self.shared);
        shared.status.files += files;
        shared.status.dirs += dirs;
        shared.status.current = path.display().to_string();
        let room = MAX_RESULTS.saturating_sub(shared.results.len());
        if found.len() > room {
            found.truncate(room);
            shared.status.truncated = true;
        }
        shared.results.extend(found);
        shared.status.found = shared.results.len();
        (subdirs, shared.status.truncated)
    }
}

/// The OS hidden flag; dot-names are checked separately. Only Windows and macOS have one,
/// so only they pay for reading the entry's metadata.
fn os_hidden(child: &std::fs::DirEntry) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        child
            .metadata()
            .is_ok_and(|meta| fm_fs_local::os_hidden(&meta))
    } else {
        false
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
    let needle = Needle::new(&query.content);
    let shared = Arc::new(Mutex::new(Shared::default()));
    lock(&shared).status.running = true;
    let cancel = Arc::new(AtomicBool::new(false));
    let (thread_shared, thread_cancel, hidden) = (shared.clone(), cancel.clone(), query.hidden);
    std::thread::Builder::new()
        .name("fm-search".into())
        .spawn(move || {
            if vfs.is_native_local() && root.scheme() == Scheme::File {
                LocalWalk {
                    vfs: vfs.as_ref(),
                    name,
                    needle,
                    hidden,
                    shared: &thread_shared,
                    cancel: &thread_cancel,
                    queue: Mutex::new(Queue {
                        dirs: Vec::new(),
                        busy: 0,
                        done: false,
                    }),
                    wake: Condvar::new(),
                }
                .run(&root);
            } else {
                Walker {
                    vfs: vfs.as_ref(),
                    name,
                    needle,
                    hidden,
                    shared: &thread_shared,
                    cancel: &thread_cancel,
                }
                .run(&root);
            }
            let mut shared = lock(&thread_shared);
            shared.status.running = false;
            shared.status.current.clear();
        })
        .map_err(FmError::Io)?;
    let origin = s.layout.focused();
    let search = s.search.get_or_insert_with(|| Search {
        query: SearchQuery::default(),
        origin,
        minimized: false,
        run: None,
    });
    search.query = query.clone();
    search.run = Some(Run { shared, cancel });
    s.remember_search(&query);
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
        shortcut: Some("Ctrl+F"),
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
                minimized: false,
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

/// `search.open_everywhere` — Find Files over the whole computer: `/` on Linux and
/// macOS, every drive on Windows (system pseudo-folders are skipped by the walk).
pub fn open_everywhere() -> CommandSpec {
    CommandSpec {
        id: "search.open_everywhere",
        label: "Find Files Everywhere…",
        menu: &["File", "Find Files Everywhere…"],
        shortcut: Some("Ctrl+Shift+F"),
        params: "none",
        enabled: |_| true,
        run: |s, params| {
            object(params)?;
            let origin = s.layout.focused();
            let origin_is_files = s.layout.state(origin).is_some_and(|p| p.kind.is_files());
            let search = s.search.get_or_insert_with(|| Search {
                query: SearchQuery::default(),
                origin,
                minimized: false,
                run: None,
            });
            if origin_is_files {
                search.origin = origin;
            }
            search.query.root = Some(VfsUri::root_file());
            Ok(())
        },
    }
}

/// Kernel and system pseudo-folders that a whole-disk search must not walk: they are
/// endless, mirror the disk again, or block on reads.
fn is_pseudo_folder(uri: &VfsUri) -> bool {
    if uri.scheme() != Scheme::File {
        return false;
    }
    let path = uri.path_str();
    let linux = ["/proc", "/sys", "/dev", "/run"];
    let macos = [
        "/System/Volumes",
        "/dev",
        "/private/var/vm",
        "/Volumes/Recovery",
    ];
    if cfg!(target_os = "macos") {
        macos.contains(&path)
    } else if cfg!(windows) {
        false
    } else {
        linux.contains(&path)
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

/// `search.minimize` — keep the search running in the jobs panel, without its window.
pub fn minimize() -> CommandSpec {
    CommandSpec {
        id: "search.minimize",
        label: "Run Search in Background",
        menu: &["File", "Find Files…", "Minimize"],
        shortcut: None,
        params: "none",
        enabled: |s| s.search.as_ref().is_some_and(|x| !x.minimized),
        run: |s, params| {
            object(params)?;
            let search = s
                .search
                .as_mut()
                .ok_or_else(|| FmError::InvalidParams("no search is open".to_string()))?;
            search.minimized = true;
            Ok(())
        },
    }
}

/// `search.restore` — show the Find Files window of a minimised search again.
pub fn restore() -> CommandSpec {
    CommandSpec {
        id: "search.restore",
        label: "Show Search",
        menu: &["File", "Find Files…", "Show"],
        shortcut: None,
        params: "none",
        enabled: |s| s.search.as_ref().is_some_and(|x| x.minimized),
        run: |s, params| {
            object(params)?;
            let search = s
                .search
                .as_mut()
                .ok_or_else(|| FmError::InvalidParams("no search is open".to_string()))?;
            search.minimized = false;
            Ok(())
        },
    }
}

/// `search.history.run {index}` — run a remembered search again (newest is 0).
pub fn history_run() -> CommandSpec {
    CommandSpec {
        id: "search.history.run",
        label: "Run Recent Search",
        menu: &["File", "Find Files…", "Recent"],
        shortcut: None,
        params: "index: number (0 = newest)",
        enabled: |s| !s.recent_searches().is_empty(),
        run: |s, params| {
            let index = object(params)?
                .and_then(|m| m.get("index"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| FmError::InvalidParams("index must be a number".to_string()))?;
            let saved = s
                .recent_searches()
                .into_iter()
                .nth(index)
                .ok_or_else(|| FmError::InvalidParams("no such recent search".to_string()))?;
            let query = SearchQuery {
                root: Some(VfsUri::parse(&saved.root)?),
                name: saved.name,
                content: saved.content,
                hidden: saved.hidden,
            };
            start(s, query)?;
            if let Some(search) = s.search.as_mut() {
                search.minimized = false;
            }
            Ok(())
        },
    }
}

/// `search.history.clear` — forget every remembered search.
pub fn history_clear() -> CommandSpec {
    CommandSpec {
        id: "search.history.clear",
        label: "Clear Recent Searches",
        menu: &["File", "Find Files…", "Clear recent"],
        shortcut: None,
        params: "none",
        enabled: |s| !s.prefs.search_history.is_empty(),
        run: |s, params| {
            object(params)?;
            s.prefs.search_history.clear();
            Ok(())
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
    vec![
        open(),
        open_everywhere(),
        start_cmd(),
        minimize(),
        restore(),
        history_run(),
        history_clear(),
        stop(),
        close(),
        reveal(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Params;
    use serde_json::json;
    use std::time::{Duration, Instant};

    /// Run one walker over `root` and return the sorted result paths.
    fn walk(root: &VfsUri, name: &str, content: &str, hidden: bool, local: bool) -> Vec<String> {
        let vfs = fm_fs_local::LocalFs::default();
        let shared = Mutex::new(Shared::default());
        let cancel = AtomicBool::new(false);
        let name = (!name.is_empty()).then(|| NameTest::parse(name).expect("pattern"));
        let needle = Needle::new(content);
        if local {
            LocalWalk {
                vfs: &vfs,
                name,
                needle,
                hidden,
                shared: &shared,
                cancel: &cancel,
                queue: Mutex::new(Queue {
                    dirs: Vec::new(),
                    busy: 0,
                    done: false,
                }),
                wake: Condvar::new(),
            }
            .run(root);
        } else {
            Walker {
                vfs: &vfs,
                name,
                needle,
                hidden,
                shared: &shared,
                cancel: &cancel,
            }
            .run(root);
        }
        let mut paths: Vec<String> = lock(&shared)
            .results
            .iter()
            .map(|e| e.uri.to_string())
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn the_fast_local_walk_finds_exactly_what_the_backend_walk_finds() {
        let tmp = tempfile::tempdir().expect("tmp");
        let t = tmp.path();
        for dir in ["a/b/c", "a/.hidden/d", "e", "f g/ü"] {
            std::fs::create_dir_all(t.join(dir)).expect("dirs");
        }
        for (path, body) in [
            ("a/one.txt", &b"needle here"[..]),
            ("a/b/two.TXT", b"NeEdLe"),
            ("a/b/c/bin.txt", b"\0\0needle"),
            ("a/.hidden/d/three.txt", b"needle"),
            ("e/.dot.txt", b"needle"),
            ("f g/ü/four.txt", b"no match"),
            ("e/other.log", b"needle"),
        ] {
            std::fs::write(t.join(path), body).expect("write");
        }
        // A needle split across the chunk boundary.
        let mut big = vec![b'x'; CHUNK - 2];
        big.extend_from_slice(b"NEEDLE");
        std::fs::write(t.join("e/big.txt"), &big).expect("big");
        #[cfg(unix)]
        std::os::unix::fs::symlink(t, t.join("a/b/loop")).expect("symlink loop");
        let root = VfsUri::parse(t.to_str().expect("utf8")).expect("uri");
        for (name, content, hidden) in [
            ("*.txt", "", false),
            ("*.txt", "", true),
            ("", "needle", false),
            ("*.txt", "needle", true),
            ("ü", "", false),
            ("loop", "", false),
        ] {
            let fast = walk(&root, name, content, hidden, true);
            let slow = walk(&root, name, content, hidden, false);
            assert_eq!(fast, slow, "{name:?} {content:?} hidden={hidden}");
        }
        assert_eq!(walk(&root, "", "needle", false, true).len(), 4);
    }

    #[test]
    fn a_cancelled_fast_walk_stops() {
        let tmp = tempfile::tempdir().expect("tmp");
        for i in 0..50 {
            std::fs::create_dir_all(tmp.path().join(format!("d{i}/x"))).expect("dir");
        }
        let root = VfsUri::parse(tmp.path().to_str().expect("utf8")).expect("uri");
        let vfs = fm_fs_local::LocalFs::default();
        let shared = Mutex::new(Shared::default());
        let cancel = AtomicBool::new(true);
        LocalWalk {
            vfs: &vfs,
            name: Some(NameTest::parse("x").expect("pattern")),
            needle: None,
            hidden: false,
            shared: &shared,
            cancel: &cancel,
            queue: Mutex::new(Queue {
                dirs: Vec::new(),
                busy: 0,
                done: false,
            }),
            wake: Condvar::new(),
        }
        .run(&root);
        let shared = lock(&shared);
        assert!(shared.status.stopped);
        assert!(shared.results.is_empty());
    }

    #[test]
    fn searches_are_remembered_newest_first_without_duplicates_and_capped() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("a.pdf"), b"x").expect("w");
        let root = tmp.path().to_str().expect("utf8").to_string();
        let mut s = Session::new().expect("session");
        for i in 0..25 {
            s.dispatch(
                "search.start",
                &json!({"root": root, "name": format!("n{i}")}),
            )
            .expect("start");
            finish(&s);
        }
        let recent = s.recent_searches();
        assert_eq!(recent.len(), MAX_HISTORY);
        assert_eq!(recent[0].name, "n24");
        // Running an older one again moves it to the front instead of duplicating it.
        s.dispatch("search.start", &json!({"root": root, "name": "n10"}))
            .expect("again");
        finish(&s);
        let recent = s.recent_searches();
        assert_eq!(recent.len(), MAX_HISTORY);
        assert_eq!(recent[0].name, "n10");
        assert_eq!(recent.iter().filter(|r| r.name == "n10").count(), 1);
        // Survives a save and load.
        let encoded = toml::to_string(&s.prefs).expect("serialize");
        let decoded: crate::prefs::Prefs = toml::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded.search_history, s.prefs.search_history);
        // Run a remembered search: same query, window shown.
        s.dispatch("search.history.run", &json!({"index": 3}))
            .expect("run");
        let expected = recent[3].clone();
        let search = s.search.as_ref().expect("search");
        assert_eq!(search.query.name, expected.name);
        assert!(!search.minimized);
        finish(&s);
        s.dispatch("search.history.clear", &Params::Null)
            .expect("clear");
        assert!(s.recent_searches().is_empty());
        assert!(!s.command_enabled("search.history.run"));
    }

    #[test]
    fn hand_edited_history_entries_that_break_the_caps_are_ignored() {
        let mut s = Session::new().expect("session");
        let ok = SavedSearch {
            root: "/home".into(),
            name: "*.txt".into(),
            content: String::new(),
            hidden: false,
        };
        s.prefs.search_history = vec![
            SavedSearch {
                name: "x".repeat(MAX_PATTERN_LEN + 1),
                ..ok.clone()
            },
            SavedSearch {
                content: "y".repeat(MAX_NEEDLE + 1),
                ..ok.clone()
            },
            SavedSearch {
                root: "not a uri\0".into(),
                ..ok.clone()
            },
            SavedSearch {
                name: " ".into(),
                ..ok.clone()
            },
            ok.clone(),
        ];
        assert_eq!(s.recent_searches(), vec![ok.clone()]);
        assert!(ok.label().contains("\"*.txt\" in /home"));
        for params in [
            json!({"index": 1}),
            json!({"index": -1}),
            json!({"index": "0"}),
            json!({}),
            json!(null),
        ] {
            assert!(
                s.dispatch("search.history.run", &params).is_err(),
                "{params}"
            );
        }
    }

    #[test]
    fn a_minimised_search_keeps_running_and_comes_back() {
        let tmp = tempfile::tempdir().expect("tmp");
        for i in 0..200 {
            std::fs::create_dir_all(tmp.path().join(format!("d{i}"))).expect("dir");
            std::fs::write(tmp.path().join(format!("d{i}/hit.txt")), b"x").expect("w");
        }
        let root = tmp.path().to_str().expect("utf8").to_string();
        let mut s = Session::new().expect("session");
        assert!(s.dispatch("search.minimize", &Params::Null).is_err());
        s.dispatch("search.start", &json!({"root": root, "name": "hit.txt"}))
            .expect("start");
        s.dispatch("search.minimize", &Params::Null)
            .expect("minimize");
        assert!(s.search.as_ref().expect("search").minimized);
        assert!(!s.command_enabled("search.minimize"));
        assert_eq!(finish(&s).found, 200, "minimising does not stop the search");
        s.dispatch("search.restore", &Params::Null)
            .expect("restore");
        assert!(!s.search.as_ref().expect("search").minimized);
        assert!(s.dispatch("search.restore", &json!([1])).is_err());
    }

    #[test]
    fn everywhere_searches_the_whole_computer_without_pseudo_folders() {
        let mut s = Session::new().expect("session");
        s.dispatch("search.open_everywhere", &Params::Null)
            .expect("open");
        let root = s.search.as_ref().and_then(|x| x.query.root.clone());
        assert_eq!(root, Some(VfsUri::root_file()));
        assert!(s.dispatch("search.open_everywhere", &json!([1])).is_err());
        let pseudo = |p: &str| is_pseudo_folder(&VfsUri::parse(p).expect("uri"));
        if cfg!(target_os = "linux") {
            assert!(pseudo("/proc") && pseudo("/sys") && pseudo("/dev") && pseudo("/run"));
        }
        if cfg!(target_os = "macos") {
            assert!(pseudo("/System/Volumes") && pseudo("/dev"));
        }
        assert!(!pseudo("/home") && !pseudo("/proc/x") && !pseudo("/procs"));
    }

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
