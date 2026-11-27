//! Archives as folders (change `add-archive-vfs`): the mount table and the
//! `archive.enter`, `archive.unmount`, `archive.password` and `archive.password.forget`
//! commands.
//!
//! Entering an archive registers a [`LazyArchive`] backend under `archive://<handle>/`
//! and navigates there; the archive is opened by the first directory read, on the
//! archive job lane. Which file a handle names is also kept in a process-wide table so
//! [`display`] can show `…/site.zip/www` wherever a location is shown.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

pub use fm_archive::LazyArchive;
use fm_archive::{FormatId, Secret};
use fm_vfs::{FmError, Scheme, VfsUri};

use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// The archive file (and, for items found inside blobs, offset and format) behind a handle.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ArchiveRef {
    pub container: VfsUri,
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// Handles this process knows, for display and re-mounting (bounded).
static REFS: LazyLock<RwLock<HashMap<String, ArchiveRef>>> = LazyLock::new(Default::default);

/// Most handles remembered; the oldest are dropped first, which only affects display.
const MAX_REFS: usize = 4096;

/// Longest password accepted (bytes).
pub(crate) const MAX_PASSWORD: usize = 1024;

/// Mounted archives of this session.
#[derive(Default)]
pub struct Mounts {
    lazies: HashMap<String, Arc<LazyArchive>>,
    /// The archive Extract To… is choosing a folder for.
    pub extract_pending: Option<VfsUri>,
    /// The last location each pane's tree was revealed for.
    revealed: HashMap<fm_layout::PaneId, VfsUri>,
    /// Archive files opened this session, oldest first: the tree keeps showing them as
    /// folders until they are closed (Close Archive).
    opened: Vec<VfsUri>,
    /// A job that stopped because an archive needs a password, waiting for one.
    pub password_ask: Option<PasswordAsk>,
    /// The selection Add to Archive… is choosing an archive for.
    pub add_pending: Option<Vec<VfsUri>>,
    /// The archive (root) whose staged changes Save as copy is choosing a folder for.
    pub save_as_pending: Option<VfsUri>,
    /// The archive (root) whose list of staged changes is shown (`archive.changes`).
    pub changes_shown: Option<VfsUri>,
    /// Archives (roots) to close once their save finishes (Close Archive › Save).
    close_after_save: Vec<VfsUri>,
    /// Opened archives nothing shows or uses, since when (idle release).
    idle_since: HashMap<String, std::time::Instant>,
    last_idle_check: Option<std::time::Instant>,
    /// Archives being opened that a pane has been inside meanwhile (`cancel_tick`).
    entered_while_opening: std::collections::HashSet<String>,
    /// Paces the copies of what an archive being read has found (`archive_fill`).
    pub(crate) fill: crate::archive_fill::Pacer,
}

/// An opened archive nothing uses for this long frees its memory (passwords stay).
pub const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(600);
/// At most this many archives stay opened; the longest-idle unused ones go first.
pub const MAX_OPENED_MOUNTS: usize = 16;
const IDLE_CHECK: std::time::Duration = std::time::Duration::from_secs(30);

/// Handles in use: panes inside them, previews of their files, running jobs on them, a
/// waiting password dialog, and every archive those sit inside.
fn handles_in_use(s: &Session) -> std::collections::HashSet<String> {
    let mut uris: Vec<VfsUri> = s
        .layout
        .states
        .values()
        .map(|st| st.location.clone())
        .collect();
    uris.extend(
        s.previews
            .slots
            .values()
            .flat_map(|slot| slot.subjects.iter().map(|e| e.uri.clone())),
    );
    uris.extend(
        s.fileops
            .jobs()
            .iter()
            .filter(|j| j.state.is_running())
            .flat_map(|j| j.touched_uris()),
    );
    uris.extend(s.archives.password_ask.iter().map(|a| a.uri.clone()));
    let mut used = std::collections::HashSet::new();
    for uri in uris {
        let mut at = uri;
        for _ in 0..=fm_archive::limits::MAX_NESTING {
            let Some(h) = at.archive_handle().map(str::to_string) else {
                break;
            };
            used.insert(h);
            match container_of(&at) {
                Some(r) => at = r.container,
                None => break,
            }
        }
    }
    used
}

/// Free archives nothing has used for [`IDLE_RELEASE`], and keep at most
/// [`MAX_OPENED_MOUNTS`] opened. Released archives open again on their next read, with
/// their passwords. Called from `poll`; does real work every [`IDLE_CHECK`].
pub(crate) fn idle_tick(s: &mut Session, now: std::time::Instant) {
    if s.archives
        .last_idle_check
        .is_some_and(|t| now.saturating_duration_since(t) < IDLE_CHECK)
    {
        return;
    }
    s.archives.last_idle_check = Some(now);
    let used = handles_in_use(s);
    let opened: Vec<String> = s
        .archives
        .lazies
        .iter()
        .filter(|(_, m)| m.opened().is_some())
        .map(|(h, _)| h.clone())
        .collect();
    s.archives
        .idle_since
        .retain(|h, _| opened.contains(h) && !used.contains(h));
    for h in &opened {
        if !used.contains(h) {
            s.archives.idle_since.entry(h.clone()).or_insert(now);
        }
    }
    let mut idle: Vec<(String, std::time::Instant)> = s
        .archives
        .idle_since
        .iter()
        .map(|(h, t)| (h.clone(), *t))
        .collect();
    idle.sort_by_key(|(_, t)| *t);
    let mut still_open = opened.len();
    for (h, since) in idle {
        let expired = now.saturating_duration_since(since) >= IDLE_RELEASE;
        if !expired && still_open <= MAX_OPENED_MOUNTS {
            continue;
        }
        if let Some(m) = s.archives.lazies.get(&h) {
            m.release();
        }
        s.archives.idle_since.remove(&h);
        still_open -= 1;
    }
}

/// Stop opening an archive the user has left: a pane was inside it while it was being
/// opened and now none is ("leave the folder to stop", `archive-browsing`). An archive
/// opened only for the folder tree (expanded there, never entered) keeps opening.
/// Nothing is stopped while a file job runs or the address bar is open (they may read
/// an archive no pane shows). A stopped archive opens again when it is entered again.
/// Called from `poll`; costs nothing while no archive is being opened.
pub(crate) fn cancel_tick(s: &mut Session) {
    let opening = s.archives.being_read();
    if opening.is_empty() {
        s.archives.entered_while_opening.clear();
        return;
    }
    let handles: Vec<String> = opening
        .iter()
        .filter_map(|m| m.root().archive_handle().map(str::to_string))
        .collect();
    s.archives
        .entered_while_opening
        .retain(|h| handles.contains(h));
    if s.path_edit.is_some() || s.fileops.jobs().iter().any(|j| j.state.is_running()) {
        return;
    }
    let used = handles_in_use(s);
    for (mount, h) in opening.iter().zip(&handles) {
        if used.contains(h) {
            s.archives.entered_while_opening.insert(h.clone());
        } else if s.archives.entered_while_opening.remove(h) {
            mount.cancel();
        }
    }
}

/// An Extract or Open job stopped for a password: the dialog asks for it and the job
/// runs again once one is given (archive-browsing "Encrypted archives open with a
/// password").
#[derive(Clone, Debug)]
pub struct PasswordAsk {
    /// A location inside the archive (its root).
    pub uri: VfsUri,
    /// The last password given did not fit.
    pub wrong: bool,
    retry: crate::fileops::OpRequest,
}

impl PasswordAsk {
    pub(crate) fn new(uri: VfsUri, wrong: bool, retry: crate::fileops::OpRequest) -> Self {
        Self { uri, wrong, retry }
    }
}

/// Most archives the tree keeps as opened.
const MAX_OPENED: usize = 256;

impl std::fmt::Debug for Mounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mounts")
            .field("handles", &self.lazies.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Mounts {
    pub fn get(&self, handle: &str) -> Option<&Arc<LazyArchive>> {
        self.lazies.get(handle)
    }

    /// The opened archive that the archive file `file` is: that file itself, or any part
    /// of the same numbered split set (`x.7z.002` finds the set opened from `x.7z.001`).
    /// An opened mount is preferred over one that was only registered.
    pub fn of_file(&self, file: &VfsUri) -> Option<&Arc<LazyArchive>> {
        let dir = file.parent();
        let part_base = |name: &str| fm_archive::split::parse(name).map(|p| p.base);
        let want = file.file_name().and_then(part_base);
        let same = |m: &&Arc<LazyArchive>| {
            let c = m.container();
            c == file
                || (want.is_some()
                    && c.parent() == dir
                    && c.file_name().and_then(part_base) == want)
        };
        let mut found = self.lazies.values().filter(same);
        let first = found.next()?;
        if first.opened().is_some() {
            return Some(first);
        }
        found.find(|m| m.opened().is_some()).or(Some(first))
    }

    /// The archives being opened right now.
    pub(crate) fn being_read(&self) -> Vec<Arc<LazyArchive>> {
        self.lazies
            .values()
            .filter(|m| m.opening_for().is_some())
            .cloned()
            .collect()
    }

    /// The mount serving `uri`, if `uri` is inside an archive.
    pub fn for_uri(&self, uri: &VfsUri) -> Option<&Arc<LazyArchive>> {
        uri.archive_handle().and_then(|h| self.lazies.get(h))
    }

    /// Whether the entry at `uri` (inside an opened archive) is stored encrypted.
    pub fn is_encrypted(&self, uri: &VfsUri) -> bool {
        self.opened_fs(uri).is_some_and(|fs| fs.is_encrypted(uri))
    }

    /// The opened archive serving `uri`, for per-row lookups in one frame.
    pub fn opened_fs(&self, uri: &VfsUri) -> Option<Arc<fm_archive::ArchiveFs>> {
        self.for_uri(uri).and_then(|m| m.opened())
    }

    /// Why `uri` (inside an opened archive) cannot be taken out by a move, if it cannot:
    /// read-only archives offer copy only.
    pub fn move_blocked(&self, uri: &VfsUri) -> Option<&'static str> {
        match self.opened_fs(uri)?.writable() {
            Err(FmError::Unsupported(why)) => Some(why),
            _ => None,
        }
    }

    /// Whether the tree shows this archive file as a folder (opened and not closed since).
    pub fn is_opened(&self, container: &VfsUri) -> bool {
        self.opened.contains(container)
    }

    fn open(&mut self, container: VfsUri) {
        if !self.opened.contains(&container) {
            if self.opened.len() >= MAX_OPENED {
                self.opened.remove(0);
            }
            self.opened.push(container);
        }
    }
}

/// Whether `uri` lies inside the archive with `handle`, directly or in an archive within it.
fn within(uri: &VfsUri, handle: &str) -> bool {
    let mut at = uri.clone();
    for _ in 0..=fm_archive::limits::MAX_NESTING {
        match at.archive_handle() {
            Some(h) if h == handle => return true,
            Some(_) => match container_of(&at) {
                Some(r) => at = r.container,
                None => return false,
            },
            None => return false,
        }
    }
    false
}

/// The archive file behind an archive location (`None` for other locations).
pub fn container_of(uri: &VfsUri) -> Option<ArchiveRef> {
    let handle = uri.archive_handle()?;
    REFS.read().ok()?.get(handle).cloned()
}

/// Where `uri` really lives: outside archives itself, inside one (or nested ones) the
/// outermost archive file, so `sftp://host/a.zip` for a folder in an archive on a server.
pub fn outer_location(uri: &VfsUri) -> VfsUri {
    let mut at = uri.clone();
    for _ in 0..=fm_archive::limits::MAX_NESTING {
        match container_of(&at) {
            Some(r) => at = r.container,
            None => break,
        }
    }
    at
}

/// How many archives deep `uri` is (0 outside archives).
pub fn nesting(uri: &VfsUri) -> usize {
    let mut depth = 0;
    let mut at = uri.clone();
    while let Some(r) = container_of(&at) {
        depth += 1;
        if depth > fm_archive::limits::MAX_NESTING {
            break;
        }
        at = r.container;
    }
    depth
}

/// The display form of a location inside an archive: the archive file's display form,
/// then the inner path with the platform separator (`path-display`).
pub fn display(uri: &VfsUri, windows: bool) -> Option<String> {
    let r = container_of(uri)?;
    let base = if r.container.scheme() == Scheme::Archive {
        display(&r.container, windows).unwrap_or_else(|| r.container.to_string())
    } else if r.container.is_drive_list() {
        r.container.to_string()
    } else {
        fm_vfs::display_form(&r.container, windows)
    };
    let rest = uri.path_str().trim_start_matches('/');
    if rest.is_empty() {
        return Some(base);
    }
    let sep = if windows { "\\" } else { "/" };
    Some(format!("{base}{sep}{}", rest.replace('/', sep)))
}

/// Where Up goes from the root of an archive: the folder holding the archive file, with
/// the archive's name to put the cursor on.
pub fn up_from_root(uri: &VfsUri) -> Option<(VfsUri, String)> {
    if uri.path_str() != "/" {
        return None;
    }
    let r = container_of(uri)?;
    let name = r.container.file_name()?.to_string();
    Some((r.container.parent()?, name))
}

fn remember(handle: &str, r: ArchiveRef) {
    if let Ok(mut refs) = REFS.write() {
        if refs.get(handle) == Some(&r) {
            return;
        }
        if refs.len() >= MAX_REFS && !refs.contains_key(handle) {
            let victim = refs.keys().next().cloned();
            if let Some(v) = victim {
                refs.remove(&v);
            }
        }
        refs.insert(handle.to_string(), r);
        REFS_DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The handle table changed since it was last saved.
static REFS_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// File in the config directory that keeps the handle table across restarts, so restored
/// panes, tabs, history and bookmarks inside archives open again (tasks 2.2/4.1).
pub const REFS_FILE: &str = "archives.json";

/// Largest `archives.json` read.
const MAX_REFS_BYTES: u64 = 2 << 20;

#[derive(serde::Serialize)]
struct RefsFile {
    version: u32,
    refs: HashMap<String, ArchiveRef>,
}

/// The handle table as JSON when it changed since the last call (the app writes it).
pub fn refs_tick() -> Option<String> {
    if !REFS_DIRTY.swap(false, std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let refs = REFS.read().ok()?.clone();
    serde_json::to_string(&RefsFile { version: 1, refs }).ok()
}

/// `refs_tick` could not be written: try again on the next tick.
pub fn refs_retry() {
    REFS_DIRTY.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Write `archives.json` (owner-only, atomic).
pub fn save_refs_file(dir: &std::path::Path, text: &str) -> Result<(), FmError> {
    crate::prefs::write_atomic(dir, REFS_FILE, text.as_bytes())
}

/// Read `archives.json` from the config directory (size-capped) and load it.
pub fn load_refs_file(dir: &std::path::Path) -> usize {
    let path = dir.join(REFS_FILE);
    if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_REFS_BYTES) {
        return 0;
    }
    std::fs::read_to_string(path).map_or(0, |text| load_refs(&text))
}

/// Load a saved handle table. Hostile or stale input is dropped entry by entry: a handle
/// must be the hash of its own container (so a hand-edited file cannot point a location
/// at another file), containers must parse, and at most [`MAX_REFS`] are kept. Returns how
/// many entries were taken.
pub fn load_refs(text: &str) -> usize {
    #[derive(serde::Deserialize)]
    struct Loose {
        version: u32,
        refs: serde_json::Map<String, serde_json::Value>,
    }
    let Ok(file) = serde_json::from_str::<Loose>(text) else {
        return 0;
    };
    if file.version != 1 {
        return 0;
    }
    let Ok(mut refs) = REFS.write() else {
        return 0;
    };
    let mut taken = 0;
    // Entry by entry, so one bad entry costs only itself.
    let entries = file
        .refs
        .into_iter()
        .filter_map(|(h, v)| serde_json::from_value::<ArchiveRef>(v).ok().map(|r| (h, r)));
    for (handle, r) in entries {
        if taken >= MAX_REFS || refs.len() >= MAX_REFS {
            break;
        }
        let format = match r.format.as_deref() {
            None => None,
            Some(id) => match FormatId::from_id(id) {
                Some(f) => Some(f),
                None => continue,
            },
        };
        // The derived `Deserialize` skips `parse`'s checks: the container must survive a
        // display/parse round trip unchanged (normalised, within the caps).
        let normal = VfsUri::parse(&r.container.to_string()).ok();
        if fm_archive::handle_for(&r.container, r.offset, format) != handle
            || normal.as_ref() != Some(&r.container)
        {
            continue;
        }
        refs.entry(handle).or_insert(r);
        taken += 1;
    }
    taken
}

/// Register `container` as an archive and return its root location. Registering again
/// returns the same root and keeps the mount (and its passwords).
pub(crate) fn mount(s: &mut Session, container: &VfsUri) -> Result<VfsUri, FmError> {
    mount_as(s, container, 0, None)
}

pub(crate) fn mount_as(
    s: &mut Session,
    container: &VfsUri,
    offset: u64,
    format: Option<FormatId>,
) -> Result<VfsUri, FmError> {
    // Too deep: registered anyway, but every read fails with the reason, so the pane
    // shows it in place of a listing (`archive-browsing` "Nesting limit").
    let too_deep = nesting(container) >= fm_archive::limits::MAX_NESTING;
    let canonical = canonical_container(container);
    let container = if offset == 0 && format.is_none() {
        &canonical
    } else {
        container
    };
    let handle = fm_archive::handle_for(container, offset, format);
    let root = VfsUri::archive_root(&handle)?;
    remember(
        &handle,
        ArchiveRef {
            container: container.clone(),
            offset,
            format: format.map(|f| f.id().to_string()),
        },
    );
    if s.archives.lazies.contains_key(&handle) && s.backends.get_for(&root).is_some() {
        return Ok(root);
    }
    let container_vfs = s.backends.require(container)?;
    let mut lazy = LazyArchive::new(
        root.clone(),
        container.clone(),
        container_vfs,
        format,
        offset,
    );
    if too_deep {
        lazy = lazy.refusing(TOO_DEEP);
    }
    let lazy = Arc::new(lazy);
    s.backends.register_at(root.authority(), lazy.clone());
    s.archives.lazies.insert(handle, lazy);
    Ok(root)
}

/// Typed-path resolution through archives (`path-display`): read `uri` from the left
/// and continue inside every component that is an archive file, so the display form
/// `/home/me/site.zip/www` (or `C:\Users\me\site.zip\www`) comes back to `www` inside
/// `site.zip`, also for archives inside archives up to the nesting limit. Only registers
/// the archives (no I/O); the listing opens them on the job lanes as usual. A component
/// the parent's cached listing shows as a folder stays a folder. When an archive cannot
/// be registered (no connection yet), the location is returned as far as it got.
pub fn enter_typed(s: &mut Session, uri: VfsUri) -> VfsUri {
    let mut at = uri;
    for _ in 0..=fm_archive::limits::MAX_NESTING {
        let Some((container, rest)) = first_archive_in(s, &at) else {
            return at;
        };
        let inside = ensure_registered(s, &container)
            .and_then(|()| mount(s, &container))
            .and_then(|root| rest.iter().try_fold(root, |u, seg| u.join(seg)));
        match inside {
            Ok(inner) => at = inner,
            Err(_) => return at,
        }
    }
    at
}

/// The first component of `uri` named like an archive (and not a folder by the cached
/// listing of its parent): that archive file and the path segments after it.
fn first_archive_in(s: &Session, uri: &VfsUri) -> Option<(VfsUri, Vec<String>)> {
    let segments: Vec<&str> = uri
        .path_str()
        .split('/')
        .filter(|x| !x.is_empty())
        .collect();
    for (ix, segment) in segments.iter().enumerate() {
        if !is_archive_name(segment) {
            continue;
        }
        let prefix = uri
            .with_path(&format!("/{}", segments.get(..=ix)?.join("/")))
            .ok()?;
        let is_folder = prefix
            .parent()
            .and_then(|dir| s.dirs.entries(&dir))
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|e| e.name == *segment)
                    .map(|e| e.is_dir_like())
            })
            .unwrap_or(false);
        if is_folder {
            continue;
        }
        let rest = segments
            .get(ix + 1..)
            .unwrap_or_default()
            .iter()
            .map(|x| (*x).to_string())
            .collect();
        return Some((prefix, rest));
    }
    None
}

/// Make sure the archive behind `uri` is registered (after a restart, a restored pane or
/// history entry points at a handle this session has not mounted yet).
pub fn ensure_registered(s: &mut Session, uri: &VfsUri) -> Result<(), FmError> {
    let Some(handle) = uri.archive_handle() else {
        return Ok(());
    };
    if s.archives.lazies.contains_key(handle) {
        return Ok(());
    }
    let r = container_of(uri).ok_or_else(|| {
        FmError::NotFound("This archive location is no longer available".to_string())
    })?;
    ensure_registered(s, &r.container)?;
    let format = r.format.as_deref().and_then(FormatId::from_id);
    mount_as(s, &r.container, r.offset, format).map(|_| ())
}

/// The root location of `container` as an archive, without opening it: what the tree
/// shows as the archive's node, so it expands like a folder (`archive-browsing` "Archives
/// expand in the tree"). The handle is remembered so the root displays and mounts later.
pub fn tree_root(container: &VfsUri) -> Option<VfsUri> {
    let handle = fm_archive::handle_for(container, 0, None);
    let root = VfsUri::archive_root(&handle).ok()?;
    let known = REFS
        .read()
        .ok()
        .is_some_and(|refs| refs.contains_key(&handle));
    if !known {
        remember(
            &handle,
            ArchiveRef {
                container: container.clone(),
                offset: 0,
                format: None,
            },
        );
    }
    Some(root)
}

/// A location's parent as the tree nests it: an archive's root hangs under the folder
/// holding the archive file.
pub fn tree_parent(uri: &VfsUri) -> Option<VfsUri> {
    if uri.archive_handle().is_some() && uri.path_str() == "/" {
        return container_of(uri).and_then(|r| r.container.parent());
    }
    uri.parent()
}

/// The archive files a location is inside, innermost first (empty outside archives).
/// The tree shows only these archives (`archive-browsing` "The open archive shows in the
/// tree").
pub fn open_containers(location: &VfsUri) -> Vec<VfsUri> {
    let mut out = Vec::new();
    let mut at = location.clone();
    while let Some(r) = container_of(&at) {
        if out.len() >= fm_archive::limits::MAX_NESTING {
            break;
        }
        out.push(r.container.clone());
        at = r.container;
    }
    out
}

/// Keep each tree in step with its pane's archive: an archive a pane moves into is
/// remembered as opened (the tree shows it until it is closed) and the tree expands down
/// to the location (layout alone stops at the archive's root, which has no parent).
pub(crate) fn reveal_in_trees(s: &mut Session) -> bool {
    let mut changed = false;
    for pane in s.layout.panes() {
        let Some(location) = s.layout.state(pane).map(|p| p.location.clone()) else {
            continue;
        };
        if s.archives.revealed.get(&pane) == Some(&location) {
            continue;
        }
        s.archives.revealed.insert(pane, location.clone());
        if location.archive_handle().is_none() {
            continue;
        }
        for container in open_containers(&location).into_iter().rev() {
            s.archives.open(container);
        }
        if let Some(state) = s.layout.state_mut(pane) {
            state.reveal_location_via(&tree_parent);
            changed = true;
        }
    }
    changed
}

/// The archive file `uri` names: an archive location's archive, or an archive file itself.
fn container_for(uri: &VfsUri) -> Option<VfsUri> {
    if uri.archive_handle().is_none() && uri.file_name().is_some_and(is_archive_name) {
        return Some(canonical_container(uri));
    }
    container_of(uri).map(|r| r.container)
}

/// `Err` when the archive `handle` (or one inside it) has unsaved staged changes:
/// closing it would lose them.
fn refuse_if_dirty(s: &Session, handle: &str) -> Result<(), FmError> {
    let dirty = s.archives.lazies.iter().find(|(h, m)| {
        m.is_dirty()
            && (h.as_str() == handle
                || VfsUri::archive_root(h)
                    .ok()
                    .and_then(|root| container_of(&root))
                    .is_some_and(|r| within(&r.container, handle)))
    });
    match dirty {
        Some((_, m)) => Err(FmError::InvalidParams(format!(
            "{} has unsaved changes: save or discard them first",
            m.container().file_name().unwrap_or("the archive")
        ))),
        None => Ok(()),
    }
}

/// Close an archive: free its memory and passwords (and those of archives inside it), take
/// it out of the tree, and move panes inside it to the folder holding it.
pub(crate) fn close(s: &mut Session, container: &VfsUri) {
    let handle = fm_archive::handle_for(container, 0, None);
    let inner: Vec<String> = s
        .archives
        .lazies
        .keys()
        .filter(|h| {
            **h == handle
                || VfsUri::archive_root(h)
                    .ok()
                    .and_then(|root| container_of(&root))
                    .is_some_and(|r| within(&r.container, &handle))
        })
        .cloned()
        .collect();
    for h in &inner {
        if let Some(m) = s.archives.lazies.get(h) {
            m.unmount();
        }
    }
    // Its listings go too: opening it again reads the (possibly changed) file afresh.
    s.dirs.invalidate_where(|u| {
        u.archive_handle()
            .is_some_and(|h| inner.iter().any(|i| i == h))
    });
    s.archives
        .opened
        .retain(|c| c != container && !within(c, &handle));
    let reveal = container.file_name().map(str::to_string);
    let outside = container.parent();
    for pane in s.layout.panes() {
        let Some(state) = s.layout.state_mut(pane) else {
            continue;
        };
        let folded: Vec<VfsUri> = state
            .tree
            .expanded()
            .filter(|u| within(u, &handle))
            .cloned()
            .collect();
        for uri in &folded {
            state.tree.collapse(uri);
        }
        if state
            .tree
            .focused
            .as_ref()
            .is_some_and(|f| within(f, &handle))
        {
            state.tree.focused = outside.clone();
        }
        if within(&state.location, &handle)
            && let Some(dir) = outside.clone()
        {
            state.navigate(dir);
            state.listing.reveal = reveal.clone();
        }
    }
}

/// Whether a read failed because an archive needs a (different) password: the text of
/// `FmError::PasswordRequired` / `FmError::WrongPassword` as a listing shows it.
pub fn is_password_message(message: &str) -> bool {
    message.ends_with(" needs a password") || message.starts_with("wrong password for ")
}

/// Why archives nested deeper than [`fm_archive::limits::MAX_NESTING`] show no listing.
pub const TOO_DEEP: &str =
    "Archives nested more than 8 deep cannot be opened; copy this one out first";

/// A failed archive listing that gets its own pane state instead of a red line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveProblem {
    /// No 7-Zip or unrar found for a RAR archive.
    NeedsTool,
    /// Nested deeper than the nesting limit.
    TooDeep,
    /// The file's bytes do not match its name ("x.zip is not a zip archive").
    NotArchive,
}

/// Classify a failed listing's message at `location` (`None`: show the message as is).
pub fn problem(location: &VfsUri, message: &str) -> Option<ArchiveProblem> {
    if location.scheme() != Scheme::Archive {
        return None;
    }
    if message.contains("RAR archives need 7-Zip or unrar") {
        Some(ArchiveProblem::NeedsTool)
    } else if message.contains(TOO_DEEP) {
        Some(ArchiveProblem::TooDeep)
    } else if message.contains(" is not a") && message.contains("archive") {
        Some(ArchiveProblem::NotArchive)
    } else {
        None
    }
}

/// `Err` with the reason when added archive name patterns (`cbz=zip, epub=zip`) do not
/// parse (Preferences › Files › Archives).
pub fn check_patterns(text: &str) -> Result<(), String> {
    fm_archive::format::parse_patterns(text).map(|_| ())
}

/// Archives inside archives open up to this many levels deep.
pub const MAX_NESTING: usize = fm_archive::limits::MAX_NESTING;

/// Bytes the temporary copies of all open archives hold now (their shared limit is the
/// spool cap).
pub fn temporary_copies_bytes() -> u64 {
    fm_archive::spool::reserved_bytes()
}

/// The spool cap range Preferences accepts, in GiB.
pub const SPOOL_CAP_GIB: std::ops::RangeInclusive<u64> = fm_archive::limits::SPOOL_CAP_GIB;

/// What to install for RAR archives on this system ("install 7-Zip from 7-zip.org").
pub fn rar_install_hint() -> &'static str {
    fm_archive::format::rar::install_hint()
}

/// Why the focused pane's folder takes no changes, when it is inside a read-only archive
/// (ISO, RAR, spanned zip, a save in progress …). Write commands are disabled there and
/// show this as their tooltip.
pub fn focused_read_only(s: &Session) -> Option<&'static str> {
    let location = &s.layout.state(s.layout.focused())?.location;
    if location.scheme() != Scheme::Archive {
        return None;
    }
    s.archives.move_blocked(location)
}

/// Command ids that change the folder they act in; disabled in read-only archives.
pub const WRITE_COMMANDS: &[&str] = &[
    "file.mkdir",
    "file.newfile",
    "file.rename",
    "file.delete",
    "file.delete_permanent",
    "file.duplicate",
    "file.clip_cut",
    "file.clip_paste",
    "file.move",
];

/// Whether a file with this name opens as a folder when activated.
pub fn is_archive_name(name: &str) -> bool {
    // Every format reads (RAR through an installed 7-Zip or unrar; the pane says what to
    // install when none is found). "Open with Default App" still lets the system mount an
    // ISO.
    let readable = |_: FormatId| true;
    if fm_archive::by_name(name).is_some_and(readable) {
        return true;
    }
    // Any part of a split archive opens the whole set (design D16).
    match fm_archive::split::parse(name) {
        Some(p) if p.kind == fm_archive::split::SplitKind::ZipSpanned => true,
        Some(p) => fm_archive::by_name(&p.base).is_some_and(readable),
        None => false,
    }
}

/// Whether the archive file `name` can take changes, judged by its name alone (an opened
/// archive answers for itself): a format the app writes, or numbered parts of one.
pub fn is_writable_archive_name(name: &str) -> bool {
    let base = match fm_archive::split::parse(name) {
        Some(p) if p.kind == fm_archive::split::SplitKind::Numbered => p.base,
        Some(_) => return false,
        None => name.to_string(),
    };
    fm_archive::by_name(&base).is_some_and(|f| fm_archive::write::WRITABLE.contains(&f))
}

/// Whether `uri` lies inside a RAR (its password goes to the 7-Zip or unrar process).
pub fn is_rar_location(uri: &VfsUri) -> bool {
    container_of(uri)
        .and_then(|r| r.container.file_name().map(str::to_string))
        .is_some_and(|n| fm_archive::by_name(&n) == Some(FormatId::Rar))
}

/// A split archive's part after the first (`.002`…, `.z01`…): shown with its part
/// number and not as its own archive in the tree.
pub fn is_later_part(name: &str) -> bool {
    fm_archive::split::is_later_part(name)
}

/// "part 3" for a split archive's part (the listing's type column).
pub fn part_label(name: &str) -> Option<String> {
    fm_archive::split::part_label(name)
}

/// The file that stands for a split archive's whole set (its first numbered part, or the
/// `.zip` of a spanned set); any other file unchanged. Every part shares one mount.
pub fn canonical_container(uri: &VfsUri) -> VfsUri {
    uri.file_name()
        .and_then(fm_archive::split::canonical)
        .and_then(|name| uri.parent()?.join(&name).ok())
        .unwrap_or_else(|| uri.clone())
}

pub(crate) fn uri_of(params: &Params) -> Result<Option<VfsUri>, FmError> {
    let raw = match params {
        Params::Null => None,
        Params::String(s) => Some(s.as_str()),
        Params::Object(map) => match map.get("uri") {
            None | Some(Params::Null) => None,
            Some(Params::String(s)) => Some(s.as_str()),
            Some(_) => return Err(FmError::InvalidParams("uri must be a string".into())),
        },
        _ => {
            return Err(FmError::InvalidParams(
                "expected {uri?: string}".to_string(),
            ));
        }
    };
    raw.map(|r| VfsUri::parse(r).map_err(|_| FmError::invalid_uri(r.to_string())))
        .transpose()
}

/// The archive a command acts on: its `uri` param, else the focused pane's location.
pub(crate) fn target_mount(s: &Session, params: &Params) -> Result<Arc<LazyArchive>, FmError> {
    let uri = match uri_of(params)? {
        Some(u) => u,
        None => s
            .layout
            .state(s.layout.focused())
            .map(|p| p.location.clone())
            .ok_or_else(|| FmError::NotFound("focused pane".into()))?,
    };
    s.archives
        .for_uri(&uri)
        .cloned()
        .ok_or_else(|| FmError::InvalidParams("not inside an archive".into()))
}

/// The archive at `root` was saved, or its staged changes discarded: forget what was
/// listed from it and read again what shows it.
/// The archive at `root` was saved, or its staged changes discarded: read it again, and
/// close it when Close Archive › Save was waiting for this save and it succeeded.
pub(crate) fn after_save(s: &mut Session, root: &VfsUri) {
    if let Some(at) = s.archives.close_after_save.iter().position(|r| r == root) {
        s.archives.close_after_save.remove(at);
        // Saved: close it now, without first reading it again (a read still running
        // after the close would open it again). A failed or cancelled save keeps the
        // changes, and the archive stays open and is read again as usual.
        if s.archives.for_uri(root).is_some_and(|m| !m.is_dirty()) {
            if let Err(err) = s.dispatch("archive.unmount", &Params::String(root.to_string())) {
                s.notice = Some(err.to_string());
            }
            return;
        }
    }
    refresh_after_save(s, root);
}

/// What Close Archive on `uri` (an archive location or archive file) would lose: the
/// opened archive's root, file name and number of staged changes, when it has any. The
/// UI then asks Save, Discard or Cancel instead of closing.
pub fn unsaved_for_close(s: &Session, uri: &VfsUri) -> Option<(VfsUri, String, usize)> {
    let mount = if uri.archive_handle().is_some() {
        s.archives.for_uri(uri)
    } else {
        s.archives.of_file(uri)
    }?;
    let fs = mount.opened().filter(|fs| fs.is_dirty())?;
    let name = mount
        .container()
        .file_name()
        .unwrap_or("the archive")
        .to_string();
    Some((mount.root().clone(), name, fs.changes().len()))
}

fn refresh_after_save(s: &mut Session, root: &VfsUri) {
    let Some(mount) = s.archives.for_uri(root).cloned() else {
        return;
    };
    if let Some(handle) = root.archive_handle() {
        s.dirs
            .invalidate_where(|u| u.archive_handle() == Some(handle));
    }
    // Panes in a folder that existed only as a staged change go up to what remains (the
    // opened archive answers from memory; a reopened one keeps every saved folder).
    if let Some(fs) = mount.opened() {
        use fm_vfs::Vfs as _;
        for pane in s.layout.panes() {
            let Some(state) = s.layout.state_mut(pane) else {
                continue;
            };
            if state.location.archive_handle() != root.archive_handle() {
                continue;
            }
            let mut at = state.location.clone();
            while fs.stat(&at).is_err() {
                match at.parent() {
                    Some(p) => at = p,
                    None => break,
                }
            }
            if at != state.location {
                state.navigate(at);
            }
        }
    }
    reload(s, &mount);
    // The saved file's own folder shows a new size and time.
    if let Some(dir) = mount.container().parent()
        && !matches!(s.dirs.state(&dir), crate::DirState::Absent)
    {
        let _ = s.refresh_read(dir);
    }
}

/// Handles of the opened archives with unsaved staged changes.
pub fn dirty_handles(s: &Session) -> Vec<String> {
    s.archives
        .lazies
        .iter()
        .filter(|(_, m)| m.is_dirty())
        .map(|(h, _)| h.clone())
        .collect()
}

/// Unsaved staged changes of every opened archive: `(root, archive file, count)`.
pub fn dirty_archives(s: &Session) -> Vec<(VfsUri, VfsUri, usize)> {
    let mut out: Vec<(VfsUri, VfsUri, usize)> = s
        .archives
        .lazies
        .values()
        .filter_map(|m| {
            let fs = m.opened().filter(|fs| fs.is_dirty())?;
            Some((m.root().clone(), m.container().clone(), fs.changes().len()))
        })
        .collect();
    out.sort_by_key(|(_, c, _)| c.to_string());
    out
}

/// Read again whatever shows the archive (after a password was given or forgotten).
fn reload(s: &mut Session, mount: &LazyArchive) {
    let handle = mount.root().archive_handle().map(str::to_string);
    let shown: Vec<VfsUri> = s
        .layout
        .panes()
        .into_iter()
        .filter_map(|id| s.layout.state(id))
        .map(|p| p.location.clone())
        .filter(|l| l.archive_handle().map(str::to_string) == handle)
        .collect();
    let mut shown = shown;
    shown.sort_by_key(ToString::to_string);
    // Two panes on one folder must re-read it once: a second invalidate would drop the
    // read the first one started.
    shown.dedup();
    for uri in shown {
        let _ = s.refresh_read(uri);
    }
    // Previews of files inside it render again (in place, no spinner).
    for slot in s.previews.slots.values_mut() {
        let inside = slot
            .subjects
            .iter()
            .any(|e| e.uri.archive_handle().map(str::to_string) == handle);
        if inside {
            slot.stale = true;
        }
    }
}

/// `archive.enter {uri}` — browse into an archive file in the focused pane.
pub fn enter() -> CommandSpec {
    CommandSpec {
        id: "archive.enter",
        label: "Browse Inside",
        menu: &["File", "Archive", "Browse Inside"],
        shortcut: None,
        params: "{uri: string} — an archive file",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let uri = uri_of(params)?
                .ok_or_else(|| FmError::InvalidParams("expected {uri: string}".into()))?;
            let name = uri
                .file_name()
                .ok_or_else(|| FmError::InvalidParams("not a file".into()))?;
            if !is_archive_name(name) {
                return Err(FmError::Unsupported("this file is not an archive"));
            }
            ensure_registered(s, &uri)?;
            let root = mount(s, &uri)?;
            s.dispatch("nav.open", &Params::String(root.to_string()))
        },
    }
}

/// `archive.unmount {uri?}` — close an archive (and forget its passwords).
pub fn unmount() -> CommandSpec {
    CommandSpec {
        id: "archive.unmount",
        label: "Close Archive",
        menu: &["File", "Archive", "Close Archive"],
        shortcut: None,
        params: "{uri?: string, unsaved?: \"save\"|\"discard\"} — an archive location or archive file; default the focused pane's archive. With staged changes it refuses unless `unsaved` says what to do with them: save (it closes when the save finishes) or discard",
        enabled: |s| {
            s.layout
                .state(s.layout.focused())
                .is_some_and(|p| p.location.scheme() == Scheme::Archive)
        },
        run: |s, params: &Params| {
            let uri = match uri_of(params)? {
                Some(u) => u,
                None => s
                    .layout
                    .state(s.layout.focused())
                    .map(|p| p.location.clone())
                    .ok_or_else(|| FmError::NotFound("focused pane".into()))?,
            };
            let unsaved = match params.as_object().and_then(|m| m.get("unsaved")) {
                None | Some(Params::Null) => None,
                Some(Params::String(v)) if v == "save" || v == "discard" => Some(v.clone()),
                Some(_) => {
                    return Err(FmError::InvalidParams(
                        "unsaved must be \"save\" or \"discard\"".into(),
                    ));
                }
            };
            let container = container_for(&uri)
                .ok_or_else(|| FmError::InvalidParams("not an archive".into()))?;
            if let Some(choice) = unsaved
                && let Some((root, _, _)) = unsaved_for_close(s, &uri)
            {
                if choice == "save" {
                    // Closes in after_save once the save job succeeds.
                    s.dispatch(
                        "archive.save",
                        &serde_json::json!({ "uri": root.to_string() }),
                    )?;
                    if !s.archives.close_after_save.contains(&root) {
                        s.archives.close_after_save.push(root);
                    }
                    return Ok(());
                }
                s.dispatch(
                    "archive.discard",
                    &serde_json::json!({ "uri": root.to_string(), "confirm": true }),
                )?;
            }
            refuse_if_dirty(s, &fm_archive::handle_for(&container, 0, None))?;
            close(s, &container);
            if let Some(dir) = container.parent() {
                let _ = s.ensure_read(dir);
            }
            Ok(())
        },
    }
}

/// `archive.password {uri?, password}` — add a password for an encrypted archive and
/// read again what needed it. The password stays in memory with the mount only.
pub fn password() -> CommandSpec {
    CommandSpec {
        id: "archive.password",
        label: "Archive Password",
        menu: &["File", "Archive", "Enter Password…"],
        shortcut: None,
        params: "{uri?: string, password: string}",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let map = params
                .as_object()
                .ok_or_else(|| FmError::InvalidParams("expected {password: string}".into()))?;
            let pw = map
                .get("password")
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("password must be a string".into()))?;
            if pw.is_empty() || pw.len() > MAX_PASSWORD {
                return Err(FmError::InvalidParams(format!(
                    "password must be 1 to {MAX_PASSWORD} bytes"
                )));
            }
            let mount = target_mount(s, params)?;
            if mount.passwords().gave_up() {
                let name = mount.container().file_name().unwrap_or("archive");
                return Err(fm_archive::format::gave_up_error(name));
            }
            mount.passwords().add(Secret::new(pw));
            reload(s, &mount);
            // A job that stopped for this archive's password runs again.
            let same = |a: &PasswordAsk| a.uri.archive_handle() == mount.root().archive_handle();
            if let Some(ask) = s.archives.password_ask.take_if(|a| same(a)) {
                s.submit_op(ask.retry)?;
            }
            Ok(())
        },
    }
}

/// `archive.password.cancel` — close the password dialog of a stopped job.
pub fn cancel_password() -> CommandSpec {
    CommandSpec {
        id: "archive.password.cancel",
        label: "Cancel Password",
        menu: &["File", "Archive", "Cancel Password"],
        shortcut: None,
        params: "none",
        enabled: |s| s.archives.password_ask.is_some(),
        run: |s, params: &Params| {
            if !params.is_null() && !params.as_object().is_some_and(|m| m.is_empty()) {
                return Err(FmError::InvalidParams("expected no parameters".into()));
            }
            s.archives
                .password_ask
                .take()
                .map(|_| ())
                .ok_or_else(|| FmError::InvalidParams("no password is being asked".into()))
        },
    }
}

/// `archive.password.forget {uri?}` — drop the passwords given for an archive.
pub fn forget_password() -> CommandSpec {
    CommandSpec {
        id: "archive.password.forget",
        label: "Forget Password",
        menu: &["File", "Archive", "Forget Password"],
        shortcut: None,
        params: "{uri?: string} — default the focused pane's archive",
        enabled: |s| {
            s.layout
                .state(s.layout.focused())
                .and_then(|p| s.archives.for_uri(&p.location))
                .is_some_and(|m| crate::archive_edit_cmds::enabled_on("archive.password.forget", m))
        },
        run: |s, params: &Params| {
            let mount = target_mount(s, params)?;
            if let Some(h) = mount.root().archive_handle() {
                refuse_if_dirty(s, h)?;
            }
            mount.unmount();
            reload(s, &mount);
            Ok(())
        },
    }
}

/// `archive.open_app {uri}` — open an archive file with its application instead of
/// browsing inside (the context menu's "Open with Default App").
pub fn open_app() -> CommandSpec {
    CommandSpec {
        id: "archive.open_app",
        label: "Open with Default App",
        menu: &["File", "Archive", "Open with Default App"],
        shortcut: None,
        params: "{uri: string} — an archive file",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let uri = uri_of(params)?
                .ok_or_else(|| FmError::InvalidParams("expected {uri: string}".into()))?;
            s.dispatch(
                "file.open",
                &serde_json::json!({ "uri": uri.to_string(), "browse": false }),
            )
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        enter(),
        open_app(),
        unmount(),
        password(),
        cancel_password(),
        forget_password(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write as _;

    #[test]
    fn failed_archive_listings_are_classified_for_their_pane_state() {
        let root = VfsUri::archive_root(&fm_archive::handle_for(
            &VfsUri::parse("/tmp/x.rar").expect("uri"),
            0,
            None,
        ))
        .expect("root");
        let tool = fm_archive::format::rar::needs_tool_error().to_string();
        assert_eq!(problem(&root, &tool), Some(ArchiveProblem::NeedsTool));
        let deep = FmError::backend("archive", TOO_DEEP).to_string();
        assert_eq!(problem(&root, &deep), Some(ArchiveProblem::TooDeep));
        assert_eq!(
            problem(&root, "backend archive: notes.zip is not a zip archive"),
            Some(ArchiveProblem::NotArchive)
        );
        assert_eq!(problem(&root, "i/o error: gone"), None);
        let folder = VfsUri::parse("/tmp").expect("uri");
        assert_eq!(problem(&folder, &tool), None, "only inside archives");
    }

    pub(crate) fn zip_bytes(password: Option<&str>) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut out);
            let mut opts = zip::write::SimpleFileOptions::default();
            if let Some(pw) = password {
                opts = opts.with_aes_encryption(zip::AesMode::Aes256, pw);
            }
            w.add_directory("www/", zip::write::SimpleFileOptions::default())
                .expect("dir");
            w.start_file("www/index.html", opts).expect("file");
            w.write_all(b"<h1>hi</h1>").expect("w");
            w.finish().expect("finish");
        }
        out.into_inner()
    }

    fn session_with(name: &str, bytes: &[u8]) -> (Session, VfsUri) {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write");
        let s = Session::new().expect("session");
        let uri = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
        std::mem::forget(dir);
        (s, uri)
    }

    fn wait_listing(s: &mut Session, uri: &VfsUri) -> crate::DirState {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            s.poll();
            let state = s.dirs.state(uri);
            if !matches!(
                state,
                crate::DirState::Loading { .. } | crate::DirState::Absent
            ) {
                return state;
            }
            assert!(std::time::Instant::now() < deadline, "listing timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn every_archive_format_and_part_is_browsable_by_name() {
        for name in [
            "a.zip",
            "a.tar.gz",
            "a.7z",
            "disc.iso",
            "data.rar",
            "data.part2.rar",
            "data.r00",
            "b.7z.002",
            "site.z01",
        ] {
            assert!(is_archive_name(name), "{name}");
        }
        for name in ["notes.txt", "movie.mkv.001", "x.part2.txt"] {
            assert!(!is_archive_name(name), "{name}");
        }
    }

    #[test]
    fn idle_archives_free_their_memory_but_keep_passwords() {
        let (mut s, zip) = session_with("idle.zip", &zip_bytes(Some("pw")));
        s.dispatch("archive.enter", &json!({"uri": zip.to_string()}))
            .expect("enter");
        let root = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone();
        wait_listing(&mut s, &root);
        s.dispatch("archive.password", &json!({"password": "pw"}))
            .expect("password");
        let mount = s.archives.for_uri(&root).cloned().expect("mount");
        assert!(mount.opened().is_some());
        let t0 = std::time::Instant::now();
        idle_tick(&mut s, t0);
        idle_tick(&mut s, t0 + IDLE_RELEASE * 2);
        assert!(mount.opened().is_some(), "a pane is inside it: kept");

        let outside = zip.parent().expect("dir");
        s.dispatch("nav.open", &Params::String(outside.to_string()))
            .expect("leave");
        idle_tick(&mut s, t0 + IDLE_RELEASE * 3);
        assert!(mount.opened().is_some(), "idle only just started");
        idle_tick(
            &mut s,
            t0 + IDLE_RELEASE * 3 + std::time::Duration::from_secs(30),
        );
        assert!(mount.opened().is_some(), "not idle long enough");
        idle_tick(&mut s, t0 + IDLE_RELEASE * 5);
        assert!(mount.opened().is_none(), "released after ten idle minutes");
        assert!(!mount.passwords().is_empty(), "passwords stay");
        // The next read opens it again, password and all.
        let page = root
            .join("www")
            .and_then(|w| w.join("index.html"))
            .expect("page");
        let vfs = s.backends.require(&page).expect("backend");
        let mut text = String::new();
        std::io::Read::read_to_string(&mut vfs.open_read(&page).expect("open"), &mut text)
            .expect("read");
        assert_eq!(text, "<h1>hi</h1>");
        assert!(mount.opened().is_some(), "opened again");
    }

    #[test]
    fn a_location_inside_an_archive_is_revealed_in_the_tree() {
        let tmp = tempfile::tempdir().expect("tmp");
        let zip = tmp.path().join("site.zip");
        std::fs::write(&zip, zip_bytes(None)).expect("zip");
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        let tree_root = VfsUri::parse(tmp.path().to_str().expect("utf8")).expect("uri");
        if let Some(state) = s.layout.state_mut(pane) {
            state.tree = fm_layout::TreeState::new(tree_root.clone());
        }
        let container = VfsUri::parse(zip.to_str().expect("utf8")).expect("uri");
        let root = tree_root_of(&container);
        let inner = root.join("docs").expect("inner");
        if let Some(state) = s.layout.state_mut(pane) {
            state.location = inner.clone();
        }
        assert!(reveal_in_trees(&mut s));
        let state = s.layout.state(pane).expect("pane");
        assert!(state.tree.is_expanded(&root), "the archive's row opens");
        assert_eq!(state.tree.focused.as_ref(), Some(&inner));
        assert_eq!(tree_parent(&root), Some(tree_root.clone()));
        assert_eq!(open_containers(&inner), std::slice::from_ref(&container));
        assert!(!reveal_in_trees(&mut s), "only once per location");
        assert!(s.archives.is_opened(&container));
        // Leaving keeps it in the tree…
        if let Some(state) = s.layout.state_mut(pane) {
            state.location = tree_root.clone();
        }
        reveal_in_trees(&mut s);
        assert!(s.archives.is_opened(&container));
        assert!(open_containers(&tree_root).is_empty());
        // …until it is closed, which also takes a pane inside it back out.
        if let Some(state) = s.layout.state_mut(pane) {
            state.location = inner;
        }
        reveal_in_trees(&mut s);
        s.dispatch("archive.unmount", &json!({ "uri": root.to_string() }))
            .expect("close");
        assert!(!s.archives.is_opened(&container));
        let state = s.layout.state(pane).expect("pane");
        assert_eq!(state.location, tree_root);
        assert!(!state.tree.is_expanded(&root));
        assert_eq!(state.listing.reveal.as_deref(), Some("site.zip"));
        assert!(
            s.dispatch("archive.unmount", &json!({ "uri": tree_root.to_string() }))
                .is_err()
        );
    }

    fn tree_root_of(container: &VfsUri) -> VfsUri {
        tree_root(container).expect("root")
    }

    #[test]
    fn enter_lists_the_archive_and_up_returns_to_its_folder() {
        let (mut s, zip) = session_with("site.zip", &zip_bytes(None));
        s.dispatch("archive.enter", &json!({"uri": zip.to_string()}))
            .expect("enter");
        let loc = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone();
        assert_eq!(loc.scheme(), Scheme::Archive);
        let crate::DirState::Ready { entries, .. } = wait_listing(&mut s, &loc) else {
            panic!("not listed");
        };
        assert_eq!(entries.len(), 1);
        let shown = crate::item_cmds::display_path(&loc.join("www").expect("www"));
        assert!(
            shown.ends_with("site.zip/www") || shown.ends_with("site.zip\\www"),
            "{shown}"
        );
        s.dispatch("nav.up", &Params::Null).expect("up");
        let back = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone();
        assert_eq!(Some(back), zip.parent());
    }

    #[test]
    fn a_password_unlocks_encrypted_members() {
        let (mut s, zip) = session_with("secret.zip", &zip_bytes(Some("pw")));
        s.dispatch("archive.enter", &json!({"uri": zip.to_string()}))
            .expect("enter");
        let root = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone();
        wait_listing(&mut s, &root);
        let page = root
            .join("www")
            .and_then(|w| w.join("index.html"))
            .expect("page");
        let vfs = s.backends.require(&page).expect("backend");
        assert!(matches!(
            vfs.open_read(&page),
            Err(FmError::PasswordRequired(_))
        ));
        assert!(
            s.dispatch("archive.password", &json!({"password": ""}))
                .is_err(),
            "empty password"
        );
        s.dispatch("archive.password", &json!({"password": "pw"}))
            .expect("password");
        let mut text = String::new();
        std::io::Read::read_to_string(&mut vfs.open_read(&page).expect("open"), &mut text)
            .expect("read");
        assert_eq!(text, "<h1>hi</h1>");
        s.dispatch("archive.password.forget", &Params::Null)
            .expect("forget");
        assert!(matches!(
            vfs.open_read(&page),
            Err(FmError::PasswordRequired(_))
        ));
    }

    #[test]
    fn bad_params_are_errors() {
        let (mut s, _) = session_with("x.zip", &zip_bytes(None));
        for bad in [
            Params::Null,
            json!({"uri": 5}),
            json!({"uri": "/etc"}),
            json!({"uri": "/tmp/notes.txt"}),
            json!({"uri": "x".repeat(10_000)}),
        ] {
            assert!(s.dispatch("archive.enter", &bad).is_err(), "{bad}");
        }
        assert!(
            s.dispatch("archive.password", &json!({"password": 3}))
                .is_err()
        );
        assert!(
            s.dispatch("archive.password", &json!({"password": "x".repeat(2000)}))
                .is_err()
        );
        assert!(s.dispatch("archive.unmount", &Params::Null).is_err());
    }
}
