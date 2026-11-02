//! Diff view of a Preview pane (capability `git-diff`): the selected file's working copy against
//! a committed revision, loaded on a worker, and the `preview.diff.*` commands. Blobs live only
//! as long as the pane shows the diff.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub use fm_git::diff::{Content, DiffModel, Outcome, Row, Tag};
use fm_git::{MAX_BLOB, Mark, Rev};
use fm_layout::{DiffMode, DiffSide, DiffState, PaneId, PaneKind};
use fm_vfs::{EntryKind, FmError, Scheme, Vfs, VfsUri};

use crate::file_cmds::object;
use crate::jobs::JobLane;
use crate::preview_cmds::Body;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

const MAX_REV: usize = 256;

type Stamp = Option<(SystemTime, u64)>;
type Key = (DiffSide, DiffSide, bool, Stamp);

/// Load state of one pane's diff.
#[derive(Debug, Clone)]
pub enum DiffStatus {
    Loading,
    Ready(Arc<Outcome>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct DiffView {
    key: Key,
    pub status: DiffStatus,
}

/// What a Preview pane's diff is about: the single selected file inside a git repository, or
/// two files (any backend) compared with each other.
#[derive(Debug, Clone)]
pub struct PaneDiff {
    pub path: String,
    /// The user's choice (Preview or Diff) for this selection; `None` follows the default.
    pub shown: Option<bool>,
    /// Whether the pane currently shows the diff instead of the preview.
    pub active: bool,
    /// The revision picker is open.
    pub picker: bool,
    /// Branches and tags for the picker, once listed.
    pub refs: Option<Arc<fm_git::refs::Refs>>,
    pub state: DiffState,
    stamp: Stamp,
}

impl PaneDiff {
    /// Two files compared with each other (no git revision involved).
    pub fn is_pair(&self) -> bool {
        !matches!(self.state.left, DiffSide::Rev { .. })
    }

    /// The revision the working copy is compared with.
    pub fn rev(&self) -> &str {
        match &self.state.left {
            DiffSide::Rev { rev, .. } => rev,
            _ => "HEAD",
        }
    }
}

type Done = Vec<(PaneId, Key, Result<Outcome, String>)>;
type RefsDone = Vec<(PaneId, Result<fm_git::refs::Refs, String>)>;

/// What a pane keeps while the selection moves between files (or to a non-diffable item).
#[derive(Clone)]
struct Sticky {
    shown: Option<bool>,
    repo: String,
    rev: String,
    mode: DiffMode,
    ignore_eol: bool,
}

/// A file pair chosen with "Compare with…".
#[derive(Clone)]
struct Pin {
    a: VfsUri,
    b: VfsUri,
    /// The selection the pair belongs to; set once the pane's selection reaches `a`.
    anchor: Option<Vec<VfsUri>>,
    since: std::time::Instant,
}

/// How long a pin waits for the preview to reach its file before it is dropped.
const PIN_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Diff state of every Preview pane plus the loaded results.
#[derive(Default)]
pub struct DiffHub {
    panes: HashMap<PaneId, PaneDiff>,
    sticky: HashMap<PaneId, Sticky>,
    views: HashMap<PaneId, DiffView>,
    /// Explicit file pairs ("Compare with…"), kept while the pane's selection stays `anchor`.
    pins: HashMap<PaneId, Pin>,
    pub(crate) finds: HashMap<PaneId, crate::diff_find_cmds::DiffFind>,
    done: Arc<Mutex<Done>>,
    refs_done: Arc<Mutex<RefsDone>>,
}

impl std::fmt::Debug for DiffHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffHub")
            .field("panes", &self.panes.len())
            .field("views", &self.views.len())
            .finish()
    }
}

fn load_side(side: &DiffSide) -> Result<Content, String> {
    match side {
        DiffSide::File(path) => {
            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Content::Missing),
                Err(e) => return Err(e.to_string()),
            };
            if meta.is_dir() {
                return Err(format!("{path} is a folder"));
            }
            if meta.len() > MAX_BLOB as u64 {
                return Err(format!("{path} is larger than {} MiB", MAX_BLOB >> 20));
            }
            std::fs::read(path)
                .map(Content::Bytes)
                .map_err(|e| e.to_string())
        }
        DiffSide::Rev { repo, rev, rel } => blob(repo, &Rev::Commit(rev.clone()), rel),
        DiffSide::Index { repo, rel } => blob(repo, &Rev::Index, rel),
        DiffSide::Uri(_) => Err("remote files are loaded through their backend".to_string()),
    }
}

/// A side ready to load on a worker: plain data, or a backend plus the file to read from it.
enum Source {
    Side(DiffSide),
    Remote(Arc<dyn Vfs>, VfsUri),
}

fn load_source(source: &Source) -> Result<Content, String> {
    match source {
        Source::Side(side) => load_side(side),
        Source::Remote(vfs, uri) => {
            let entry = match vfs.stat(uri) {
                Ok(e) => e,
                Err(FmError::NotFound(_)) => return Ok(Content::Missing),
                Err(e) => return Err(e.to_string()),
            };
            if !matches!(entry.kind, EntryKind::File | EntryKind::Symlink { .. }) {
                return Err(format!("{} is not a file", entry.name));
            }
            if entry.size.is_some_and(|n| n > MAX_BLOB as u64) {
                return Err(format!(
                    "{} is larger than {} MiB",
                    entry.name,
                    MAX_BLOB >> 20
                ));
            }
            let mut bytes = Vec::new();
            vfs.open_read(uri)
                .map_err(|e| e.to_string())?
                .take(MAX_BLOB as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > MAX_BLOB {
                return Err(format!(
                    "{} is larger than {} MiB",
                    entry.name,
                    MAX_BLOB >> 20
                ));
            }
            Ok(Content::Bytes(bytes))
        }
    }
}

/// The side that reads `uri`: a plain path for local files, a URI for other backends.
fn side_for(uri: &VfsUri) -> Option<DiffSide> {
    match uri.scheme() {
        Scheme::File => uri.local_path_string().map(DiffSide::File),
        Scheme::Sftp | Scheme::Smb => Some(DiffSide::Uri(uri.to_string())),
        _ => None,
    }
}

fn blob(repo: &str, rev: &Rev, rel: &str) -> Result<Content, String> {
    match fm_git::blob(Path::new(repo), rev, rel) {
        Ok(Some(bytes)) => Ok(Content::Bytes(bytes)),
        Ok(None) => Ok(Content::Missing),
        Err(e) => Err(e.to_string()),
    }
}

fn stamp(path: &str) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Text-like content that a diff can replace without hiding anything useful.
fn diffable_by_default(body: &Body) -> bool {
    match body {
        Body::Static(p) => matches!(
            &**p,
            fm_preview::Preview::Text(_)
                | fm_preview::Preview::Structured(_)
                | fm_preview::Preview::Markdown(_)
        ),
        Body::Log(_) => true,
        _ => false,
    }
}

impl Session {
    /// The diff state of a Preview pane whose selection can be compared (one file in a repo).
    pub fn diffs_find(&self, pane: PaneId) -> Option<&crate::diff_find_cmds::DiffFind> {
        self.diffs.finds.get(&pane)
    }

    pub fn pane_diff(&self, pane: PaneId) -> Option<&PaneDiff> {
        self.diffs.panes.get(&pane)
    }

    /// The load state of a Preview pane's diff, starting the load when its inputs changed.
    pub fn diff_view(&mut self, pane: PaneId) -> Option<&DiffView> {
        let diff = self.diffs.panes.get(&pane).filter(|d| d.active)?;
        let key: Key = (
            diff.state.left.clone(),
            diff.state.right.clone(),
            diff.state.ignore_eol,
            diff.stamp,
        );
        if self.diffs.views.get(&pane).is_none_or(|v| v.key != key) {
            let (left, right, eol, _) = key.clone();
            let sources = self
                .diff_source(&left)
                .and_then(|l| self.diff_source(&right).map(|r| (l, r)));
            let status = match sources {
                Ok((left, right)) => {
                    let lane = [&left, &right]
                        .into_iter()
                        .find_map(|src| match src {
                            Source::Remote(_, uri) => Some(crate::backends::lane_for(uri.scheme())),
                            Source::Side(_) => None,
                        })
                        .unwrap_or(JobLane::Local);
                    let done = self.diffs.done.clone();
                    let job_key = key.clone();
                    self.jobs.submit(lane, move |_ctx| {
                        let result = load_source(&left).and_then(|l| {
                            let r = load_source(&right)?;
                            fm_git::diff::diff(&l, &r, eol).map_err(|e| e.to_string())
                        });
                        done.lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push((pane, job_key, result));
                        Ok(())
                    });
                    DiffStatus::Loading
                }
                Err(e) => DiffStatus::Failed(e),
            };
            self.diffs.views.insert(pane, DiffView { key, status });
        }
        self.diffs.views.get(&pane)
    }

    fn diff_source(&self, side: &DiffSide) -> Result<Source, String> {
        let DiffSide::Uri(raw) = side else {
            return Ok(Source::Side(side.clone()));
        };
        let uri = VfsUri::parse(raw).map_err(|_| format!("invalid location: {raw}"))?;
        let vfs = self
            .backends
            .get_for(&uri)
            .ok_or_else(|| "not connected to that location".to_string())?;
        Ok(Source::Remote(vfs, uri))
    }

    /// Re-derive every Preview pane's diff target from its selection, apply finished
    /// comparisons and drop the data of panes that stopped showing one.
    pub(crate) fn pump_diffs(&mut self) -> bool {
        let mut changed = self.sync_pane_diffs();
        let finished: Done =
            std::mem::take(&mut *self.diffs.done.lock().unwrap_or_else(|p| p.into_inner()));
        for (pane, key, result) in finished {
            if let Some(view) = self.diffs.views.get_mut(&pane)
                && view.key == key
            {
                view.status = match result {
                    Ok(outcome) => DiffStatus::Ready(Arc::new(outcome)),
                    Err(e) => DiffStatus::Failed(e),
                };
                changed = true;
            }
        }
        let listed: RefsDone = std::mem::take(
            &mut *self
                .diffs
                .refs_done
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        for (pane, result) in listed {
            if let Some(d) = self.diffs.panes.get_mut(&pane) {
                match result {
                    Ok(refs) => d.refs = Some(Arc::new(refs)),
                    Err(e) => {
                        d.picker = false;
                        self.notice = Some(e);
                    }
                }
                changed = true;
            }
        }
        let panes = &self.diffs.panes;
        self.diffs
            .views
            .retain(|pane, _| panes.get(pane).is_some_and(|d| d.active));
        changed
    }

    fn sync_pane_diffs(&mut self) -> bool {
        let previews: Vec<PaneId> = self
            .layout
            .states
            .iter()
            .filter(|(_, st)| st.kind == PaneKind::Preview)
            .map(|(id, _)| *id)
            .collect();
        let mut changed = false;
        self.diffs.panes.retain(|id, _| previews.contains(id));
        self.diffs.sticky.retain(|id, _| previews.contains(id));
        self.diffs.pins.retain(|id, _| previews.contains(id));
        for pane in previews {
            if let Some(d) = self.diffs.panes.get(&pane)
                && let DiffSide::Rev { repo, rev, .. } = &d.state.left
            {
                self.diffs.sticky.insert(
                    pane,
                    Sticky {
                        shown: d.shown.or(d.active.then_some(true)),
                        repo: repo.clone(),
                        rev: rev.clone(),
                        mode: d.state.mode,
                        ignore_eol: d.state.ignore_eol,
                    },
                );
            }
            if let Some((a, b)) = self.pair_target(pane) {
                changed |= self.sync_pair(pane, a, b);
                continue;
            }
            let subject = self
                .previews
                .slot(pane)
                .filter(|slot| slot.subjects.len() <= 1)
                .and_then(|slot| slot.subject.clone())
                .filter(|e| e.kind == EntryKind::File && e.uri.scheme() == Scheme::File);
            let path = subject
                .as_ref()
                .and_then(|e| e.uri.local_path_string())
                .filter(|_| self.prefs.git_enabled);
            let Some(path) = path else {
                changed |= self.diffs.panes.remove(&pane).is_some();
                continue;
            };
            if self
                .diffs
                .panes
                .get(&pane)
                .is_none_or(|d| d.path != path || d.is_pair())
            {
                self.diffs.panes.remove(&pane);
                let Some((repo, rel)) = locate(&path).ok() else {
                    continue;
                };
                let previous = self.diffs.sticky.get(&pane).cloned();
                // A revision only carries over within the same repository.
                let rev = previous
                    .as_ref()
                    .filter(|p| p.repo == repo)
                    .map_or_else(|| "HEAD".to_string(), |p| p.rev.clone());
                let mut state = DiffState::new(
                    DiffSide::Rev { repo, rev, rel },
                    DiffSide::File(path.clone()),
                );
                state.ignore_eol = previous.as_ref().map_or(cfg!(windows), |p| p.ignore_eol);
                state.mode = previous.as_ref().map_or(DiffMode::Split, |p| p.mode);
                let shown = previous.and_then(|p| p.shown);
                self.diffs.panes.insert(
                    pane,
                    PaneDiff {
                        path: path.clone(),
                        shown,
                        active: false,
                        picker: false,
                        refs: None,
                        state,
                        stamp: None,
                    },
                );
                changed = true;
            }
            let modified = subject
                .as_ref()
                .and_then(|e| e.uri.parent())
                .and_then(|dir| self.git_view(&dir))
                .and_then(|status| status.mark_abs(Path::new(&path)))
                .is_some_and(|m| {
                    matches!(
                        m,
                        Mark::Modified | Mark::Renamed | Mark::Added | Mark::Conflicted
                    )
                });
            let by_default = modified
                && self
                    .previews
                    .slot(pane)
                    .is_some_and(|slot| diffable_by_default(&slot.body));
            if let Some(d) = self.diffs.panes.get_mut(&pane) {
                let active = d.shown.unwrap_or(by_default);
                let stamp = if active { stamp(&path) } else { None };
                changed |= d.active != active || d.stamp != stamp;
                d.active = active;
                d.stamp = stamp;
            }
        }
        changed
    }

    /// The two files a pane compares with each other: an explicit "Compare with…" pair while
    /// its selection is unchanged, else exactly two selected files.
    fn pair_target(&mut self, pane: PaneId) -> Option<(VfsUri, VfsUri)> {
        let slot = self.previews.slot(pane)?;
        let selection: Vec<VfsUri> = if slot.subjects.is_empty() {
            slot.subject.iter().map(|e| e.uri.clone()).collect()
        } else {
            slot.subjects.iter().map(|e| e.uri.clone()).collect()
        };
        if let Some(pin) = self.diffs.pins.get_mut(&pane) {
            let armed = pin.anchor.is_some();
            if pin
                .anchor
                .as_ref()
                .is_some_and(|anchor| *anchor == selection)
            {
                return Some((pin.a.clone(), pin.b.clone()));
            }
            if !armed && selection == [pin.a.clone()] {
                pin.anchor = Some(selection);
                return Some((pin.a.clone(), pin.b.clone()));
            }
            if armed || pin.since.elapsed() > PIN_WAIT {
                self.diffs.pins.remove(&pane);
            }
        }
        let [a, b] = slot.subjects.as_slice() else {
            return None;
        };
        let comparable =
            |e: &fm_vfs::Entry| matches!(e.kind, EntryKind::File) && side_for(&e.uri).is_some();
        (comparable(a) && comparable(b)).then(|| (a.uri.clone(), b.uri.clone()))
    }

    /// Point a pane's diff at two files. `true` when something changed.
    fn sync_pair(&mut self, pane: PaneId, a: VfsUri, b: VfsUri) -> bool {
        let (Some(left), Some(right)) = (side_for(&a), side_for(&b)) else {
            return self.diffs.panes.remove(&pane).is_some();
        };
        let path = format!("{a}\n{b}");
        let pinned = self.diffs.pins.contains_key(&pane);
        let mut changed = false;
        if self.diffs.panes.get(&pane).is_none_or(|d| d.path != path) {
            let previous = self.diffs.sticky.get(&pane).cloned();
            let mut state = DiffState::new(left, right);
            state.ignore_eol = previous.as_ref().map_or(cfg!(windows), |p| p.ignore_eol);
            state.mode = previous.as_ref().map_or(DiffMode::Split, |p| p.mode);
            self.diffs.panes.insert(
                pane,
                PaneDiff {
                    path,
                    shown: pinned.then_some(true),
                    active: false,
                    picker: false,
                    refs: None,
                    state,
                    stamp: None,
                },
            );
            changed = true;
        }
        if let Some(d) = self.diffs.panes.get_mut(&pane) {
            let active = d.shown.unwrap_or(false);
            let stamp = if active {
                [&d.state.right, &d.state.left]
                    .into_iter()
                    .find_map(|side| match side {
                        DiffSide::File(path) => Some(stamp(path)),
                        _ => None,
                    })
                    .flatten()
            } else {
                None
            };
            changed |= d.active != active || d.stamp != stamp;
            d.active = active;
            d.stamp = stamp;
        }
        changed
    }

    /// Show `a` against `b` in `pane` until its selection moves away from `a`.
    pub(crate) fn pin_pair(&mut self, pane: PaneId, a: VfsUri, b: VfsUri) {
        self.diffs.panes.remove(&pane);
        self.diffs.pins.insert(
            pane,
            Pin {
                a,
                b,
                anchor: None,
                since: std::time::Instant::now(),
            },
        );
    }

    pub(crate) fn diff_busy(&self) -> bool {
        self.diffs
            .views
            .values()
            .any(|v| matches!(v.status, DiffStatus::Loading))
    }

    fn pane_diff_mut(&mut self, params: &Params) -> Result<(PaneId, &mut PaneDiff), FmError> {
        let pane = self.preview_pane(params)?;
        self.diffs
            .panes
            .get_mut(&pane)
            .map(|d| (pane, d))
            .ok_or_else(|| {
                FmError::InvalidParams(
                    "select one file inside a git repository, or two files".to_string(),
                )
            })
    }
}

fn rev_text(text: &str) -> Result<String, FmError> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_REV || text.starts_with('-') || text.contains('\0') {
        return Err(FmError::InvalidParams("invalid revision".to_string()));
    }
    Ok(text.to_string())
}

fn locate(path: &str) -> Result<(String, String), FmError> {
    let (root, rel) = fm_git::locate(Path::new(path))
        .ok_or_else(|| FmError::InvalidParams("file is not inside a git repository".into()))?;
    Ok((root.to_string_lossy().into_owned(), rel))
}

/// The Preview pane a command acts on has a comparable file.
fn has_target(s: &Session) -> bool {
    s.preview_pane(&Params::Null)
        .is_ok_and(|pane| s.diffs.panes.contains_key(&pane))
}

/// The pane's diff compares a file with a git revision (so a revision can be chosen).
fn has_rev(s: &Session) -> bool {
    s.preview_pane(&Params::Null)
        .is_ok_and(|pane| s.diffs.panes.get(&pane).is_some_and(|d| !d.is_pair()))
}

fn showing(s: &Session) -> bool {
    s.preview_pane(&Params::Null)
        .is_ok_and(|pane| s.diffs.panes.get(&pane).is_some_and(|d| d.active))
}

fn toggle() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.toggle",
        label: "Show Diff",
        menu: &["View", "Preview Diff"],
        shortcut: None,
        params: "{pane?: number, show?: boolean} (default: switch between preview and diff)",
        enabled: has_target,
        run: |s, p: &Params| {
            let wanted = match object(p)?.and_then(|m| m.get("show")) {
                None | Some(Params::Null) => None,
                Some(v) => Some(
                    v.as_bool()
                        .ok_or_else(|| FmError::InvalidParams("show must be a boolean".into()))?,
                ),
            };
            let (_, d) = s.pane_diff_mut(p)?;
            let show = wanted.unwrap_or(!d.active);
            d.shown = Some(show);
            d.active = show;
            Ok(())
        },
    }
}

/// `preview.diff.picker {open?: boolean, pane?: number}`: the branch/tag picker. Lists the
/// repository's refs on a worker each time it opens.
fn picker() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.picker",
        label: "Choose Diff Revision",
        menu: &["View", "Preview Diff", "Choose Revision"],
        shortcut: None,
        params: "{pane?: number, open?: boolean} (default: toggle)",
        enabled: |s| showing(s) && has_rev(s),
        run: |s, p: &Params| {
            let open = match object(p)?.and_then(|m| m.get("open")) {
                None | Some(Params::Null) => None,
                Some(v) => Some(
                    v.as_bool()
                        .ok_or_else(|| FmError::InvalidParams("open must be a boolean".into()))?,
                ),
            };
            let (pane, d) = s.pane_diff_mut(p)?;
            if d.is_pair() {
                return Err(FmError::InvalidParams(
                    "a file comparison has no revision to choose".into(),
                ));
            }
            let open = open.unwrap_or(!d.picker);
            d.picker = open;
            let DiffSide::Rev { repo, .. } = &d.state.left else {
                return Ok(());
            };
            if open {
                let repo = repo.clone();
                let done = s.diffs.refs_done.clone();
                s.jobs.submit(JobLane::Local, move |_ctx| {
                    let result = fm_git::refs::list(Path::new(&repo)).map_err(|e| e.to_string());
                    done.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push((pane, result));
                    Ok(())
                });
            }
            Ok(())
        },
    }
}

fn step(forward: bool) -> impl Fn(&mut Session, &Params) -> Result<(), FmError> {
    move |s, p| {
        let (pane, _) = s.pane_diff_mut(p)?;
        let n = match s.diff_view(pane).map(|v| &v.status) {
            Some(DiffStatus::Ready(o)) => match o.as_ref() {
                Outcome::Text(m) => m.changes.len(),
                Outcome::Binary { .. } => 0,
            },
            _ => 0,
        };
        let (_, d) = s.pane_diff_mut(p)?;
        if n == 0 {
            return Ok(());
        }
        let st = &mut d.state;
        st.jumps = st.jumps.wrapping_add(1);
        st.current = if forward {
            (st.current + 1).min(n - 1)
        } else {
            st.current.saturating_sub(1).min(n - 1)
        };
        Ok(())
    }
}

fn next() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.next",
        label: "Next Change",
        menu: &["View", "Preview Diff", "Next Change"],
        shortcut: Some("F7"),
        params: "{pane?: number}",
        enabled: showing,
        run: |s, p: &Params| step(true)(s, p),
    }
}

fn prev() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.prev",
        label: "Previous Change",
        menu: &["View", "Preview Diff", "Previous Change"],
        shortcut: Some("Shift+F7"),
        params: "{pane?: number}",
        enabled: showing,
        run: |s, p: &Params| step(false)(s, p),
    }
}

fn mode() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.mode",
        label: "Diff Layout",
        menu: &["View", "Preview Diff", "Toggle Unified"],
        shortcut: None,
        params: "{pane?: number, mode?: \"split\"|\"unified\"} (default: toggle)",
        enabled: showing,
        run: |s, p: &Params| {
            let wanted = match object(p)?.and_then(|m| m.get("mode")) {
                None | Some(Params::Null) => None,
                Some(v) => Some(match v.as_str() {
                    Some("split") => DiffMode::Split,
                    Some("unified") => DiffMode::Unified,
                    _ => {
                        return Err(FmError::InvalidParams(
                            "mode must be split or unified".to_string(),
                        ));
                    }
                }),
            };
            let (_, d) = s.pane_diff_mut(p)?;
            d.state.mode = wanted.unwrap_or(match d.state.mode {
                DiffMode::Split => DiffMode::Unified,
                DiffMode::Unified => DiffMode::Split,
            });
            Ok(())
        },
    }
}

fn eol() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.eol",
        label: "Ignore Line Endings",
        menu: &["View", "Preview Diff", "Ignore Line Endings"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: showing,
        run: |s, p: &Params| {
            let (_, d) = s.pane_diff_mut(p)?;
            d.state.ignore_eol = !d.state.ignore_eol;
            Ok(())
        },
    }
}

fn reload() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.reload",
        label: "Reload Diff",
        menu: &["View", "Preview Diff", "Reload"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: showing,
        run: |s, p: &Params| {
            let (pane, _) = s.pane_diff_mut(p)?;
            s.diffs.views.remove(&pane);
            Ok(())
        },
    }
}

/// `preview.diff.rev {rev, pane?}`: compare the working copy with another revision
/// (branch, tag, commit or an expression such as `HEAD~3`).
fn rev() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.rev",
        label: "Set Diff Revision",
        menu: &["View", "Preview Diff", "Set Revision"],
        shortcut: None,
        params: "{pane?: number, rev: string}",
        enabled: has_rev,
        run: |s, p: &Params| {
            let rev = rev_text(
                object(p)?
                    .and_then(|m| m.get("rev"))
                    .and_then(Params::as_str)
                    .ok_or_else(|| FmError::InvalidParams("rev required".to_string()))?,
            )?;
            let (_, d) = s.pane_diff_mut(p)?;
            let DiffSide::Rev { rev: r, .. } = &mut d.state.left else {
                return Err(FmError::InvalidParams(
                    "a file comparison has no revision to set".into(),
                ));
            };
            *r = rev;
            d.state.current = 0;
            d.picker = false;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        toggle(),
        picker(),
        next(),
        prev(),
        mode(),
        eol(),
        reload(),
        rev(),
    ]
}
