//! Diff panes (capability `git-diff`): loading both sides on a worker, the aligned model,
//! and the `diff.*` commands. Blobs live only as long as the Diff pane shows them.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub use fm_git::diff::{Content, DiffModel, Outcome, Row, Tag};
use fm_git::{MAX_BLOB, Rev};
use fm_layout::{Axis, DiffMode, DiffSide, DiffState, PaneId, PaneKind};
use fm_vfs::{FmError, Scheme, VfsUri};

use crate::file_cmds::{focused_selection, object};
use crate::jobs::JobLane;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

const MAX_PATH: usize = 4096;
const MAX_REV: usize = 256;

type Key = (DiffSide, DiffSide, bool);

/// Load state of one Diff pane.
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

type Done = Vec<(PaneId, Key, Result<Outcome, String>)>;

/// All Diff panes' loaded results plus the "select for compare" memory.
#[derive(Default)]
pub struct DiffHub {
    views: HashMap<PaneId, DiffView>,
    done: Arc<Mutex<Done>>,
    /// File picked with "Select for Compare".
    pub mark: Option<String>,
}

impl std::fmt::Debug for DiffHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffHub")
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
    }
}

fn blob(repo: &str, rev: &Rev, rel: &str) -> Result<Content, String> {
    match fm_git::blob(Path::new(repo), rev, rel) {
        Ok(Some(bytes)) => Ok(Content::Bytes(bytes)),
        Ok(None) => Ok(Content::Missing),
        Err(e) => Err(e.to_string()),
    }
}

impl Session {
    /// The load state for a Diff pane, starting the load when the pane's comparison changed.
    pub fn diff_view(&mut self, pane: PaneId) -> Option<&DiffView> {
        let state = self.layout.state(pane)?;
        if state.kind != PaneKind::Diff {
            return None;
        }
        let diff = state.diff.as_ref()?;
        let key: Key = (diff.left.clone(), diff.right.clone(), diff.ignore_eol);
        if self.diffs.views.get(&pane).is_none_or(|v| v.key != key) {
            let (left, right, eol) = key.clone();
            let done = self.diffs.done.clone();
            let job_key = key.clone();
            self.jobs.submit(JobLane::Local, move |_ctx| {
                let result = load_side(&left).and_then(|l| {
                    let r = load_side(&right)?;
                    fm_git::diff::diff(&l, &r, eol).map_err(|e| e.to_string())
                });
                done.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((pane, job_key, result));
                Ok(())
            });
            self.diffs.views.insert(
                pane,
                DiffView {
                    key,
                    status: DiffStatus::Loading,
                },
            );
        }
        self.diffs.views.get(&pane)
    }

    /// Apply finished comparisons and drop the data of panes that stopped showing one.
    pub(crate) fn pump_diffs(&mut self) -> bool {
        let finished: Done =
            std::mem::take(&mut *self.diffs.done.lock().unwrap_or_else(|p| p.into_inner()));
        let mut changed = false;
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
        let layout = &self.layout;
        self.diffs.views.retain(|pane, _| {
            layout
                .state(*pane)
                .is_some_and(|st| st.kind == PaneKind::Diff && st.diff.is_some())
        });
        changed
    }

    pub(crate) fn diff_busy(&self) -> bool {
        self.diffs
            .views
            .values()
            .any(|v| matches!(v.status, DiffStatus::Loading))
    }

    fn diff_state_mut(&mut self, pane: PaneId) -> Result<&mut DiffState, FmError> {
        self.layout
            .state_mut(pane)
            .filter(|st| st.kind == PaneKind::Diff)
            .and_then(|st| st.diff.as_mut())
            .ok_or_else(|| FmError::InvalidParams("not a Diff pane".to_string()))
    }

    /// Show `diff` in the focused Diff pane, or in a new pane next to the focused one.
    pub fn open_diff(&mut self, diff: DiffState) -> Result<PaneId, FmError> {
        let focused = self.layout.focused();
        let reuse = self.pane_kind(focused) == Some(PaneKind::Diff);
        let pane = if reuse {
            focused
        } else {
            self.layout.split(focused, Axis::Vertical, 0.5)?
        };
        if let Some(st) = self.layout.state_mut(pane) {
            st.kind = PaneKind::Diff;
            st.diff = Some(diff);
        }
        self.diffs.views.remove(&pane);
        self.layout.focus_pane(pane)?;
        Ok(pane)
    }
}

fn abs_path(text: &str) -> Result<String, FmError> {
    if text.is_empty() || text.len() > MAX_PATH || text.contains('\0') {
        return Err(FmError::InvalidParams("invalid path".to_string()));
    }
    if !Path::new(text).is_absolute() {
        return Err(FmError::InvalidParams("path must be absolute".to_string()));
    }
    Ok(text.to_string())
}

fn rev_text(text: &str) -> Result<String, FmError> {
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

/// `"/abs/file"`, `{path, rev}` (committed) or `{path, source: "index"}` (staged).
fn parse_side(value: &Params) -> Result<DiffSide, FmError> {
    if let Some(text) = value.as_str() {
        return Ok(DiffSide::File(abs_path(text)?));
    }
    let map = value
        .as_object()
        .ok_or_else(|| FmError::InvalidParams("a side is a path or {path, rev}".to_string()))?;
    let path = abs_path(
        map.get("path")
            .and_then(Params::as_str)
            .ok_or_else(|| FmError::InvalidParams("path required".to_string()))?,
    )?;
    if map.get("source").and_then(Params::as_str) == Some("index") {
        let (repo, rel) = locate(&path)?;
        return Ok(DiffSide::Index { repo, rel });
    }
    match map.get("rev").and_then(Params::as_str) {
        Some(rev) => {
            let (repo, rel) = locate(&path)?;
            Ok(DiffSide::Rev {
                repo,
                rev: rev_text(rev)?,
                rel,
            })
        }
        None => Ok(DiffSide::File(path)),
    }
}

fn local_file(uri: &VfsUri) -> Option<String> {
    if uri.scheme() != Scheme::File {
        return None;
    }
    uri.local_path_string()
}

/// The single selected local file of the focused pane.
fn selected_file(s: &Session) -> Option<String> {
    let sel = focused_selection(s);
    match sel.as_slice() {
        [one] => local_file(one).filter(|p| Path::new(p).is_file()),
        _ => None,
    }
}

fn in_repo(path: &str) -> bool {
    fm_git::locate(Path::new(path)).is_some()
}

fn focused_diff(s: &Session) -> bool {
    s.focused_kind() == Some(PaneKind::Diff)
}

fn open_cmd() -> CommandSpec {
    CommandSpec {
        id: "diff.open",
        label: "Compare",
        menu: &["View", "Compare"],
        shortcut: None,
        params: "{left: side, right: side} | {path: string, rev?: string}; side = \"/abs/file\" | {path, rev} | {path, source: \"index\"}",
        enabled: |_s| true,
        run: |s, p: &Params| {
            let map = object(p)?.ok_or_else(|| {
                FmError::InvalidParams("left and right, or path, required".into())
            })?;
            let (left, right) = match (map.get("left"), map.get("right"), map.get("path")) {
                (Some(l), Some(r), _) => (parse_side(l)?, parse_side(r)?),
                (_, _, Some(path)) => {
                    let path = abs_path(path.as_str().ok_or_else(|| {
                        FmError::InvalidParams("path must be a string".to_string())
                    })?)?;
                    let rev = match map.get("rev") {
                        None | Some(Params::Null) => "HEAD".to_string(),
                        Some(v) => rev_text(v.as_str().ok_or_else(|| {
                            FmError::InvalidParams("rev must be a string".to_string())
                        })?)?,
                    };
                    let (repo, rel) = locate(&path)?;
                    (DiffSide::Rev { repo, rev, rel }, DiffSide::File(path))
                }
                _ => {
                    return Err(FmError::InvalidParams(
                        "left and right, or path, required".to_string(),
                    ));
                }
            };
            s.open_diff(DiffState::new(left, right)).map(|_| ())
        },
    }
}

fn compare_head() -> CommandSpec {
    CommandSpec {
        id: "diff.compare.head",
        label: "Compare with HEAD",
        menu: &["View", "Compare with HEAD"],
        shortcut: None,
        params: "{path?: string} (default: the selected file)",
        enabled: |s| s.prefs.git_enabled && selected_file(s).as_deref().is_some_and(in_repo),
        run: |s, p: &Params| {
            let path =
                match object(p)?.and_then(|m| m.get("path")) {
                    Some(v) => abs_path(v.as_str().ok_or_else(|| {
                        FmError::InvalidParams("path must be a string".to_string())
                    })?)?,
                    None => selected_file(s)
                        .ok_or_else(|| FmError::InvalidParams("select one file".to_string()))?,
                };
            let (repo, rel) = locate(&path)?;
            s.open_diff(DiffState::new(
                DiffSide::Rev {
                    repo,
                    rev: "HEAD".to_string(),
                    rel,
                },
                DiffSide::File(path),
            ))
            .map(|_| ())
        },
    }
}

fn mark_cmd() -> CommandSpec {
    CommandSpec {
        id: "diff.mark",
        label: "Select for Compare",
        menu: &["View", "Select for Compare"],
        shortcut: None,
        params: "{path?: string} (default: the selected file)",
        enabled: |s| selected_file(s).is_some(),
        run: |s, p: &Params| {
            let path =
                match object(p)?.and_then(|m| m.get("path")) {
                    Some(v) => abs_path(v.as_str().ok_or_else(|| {
                        FmError::InvalidParams("path must be a string".to_string())
                    })?)?,
                    None => selected_file(s)
                        .ok_or_else(|| FmError::InvalidParams("select one file".to_string()))?,
                };
            s.diffs.mark = Some(path);
            s.info = Some((
                "Selected for compare; pick another file and choose Compare with Selected"
                    .to_string(),
                std::time::Instant::now(),
            ));
            Ok(())
        },
    }
}

fn compare_marked() -> CommandSpec {
    CommandSpec {
        id: "diff.compare.marked",
        label: "Compare with Selected",
        menu: &["View", "Compare with Selected"],
        shortcut: None,
        params: "{path?: string} (default: the selected file)",
        enabled: |s| s.diffs.mark.is_some() && selected_file(s).is_some(),
        run: |s, p: &Params| {
            let left = s
                .diffs
                .mark
                .clone()
                .ok_or_else(|| FmError::InvalidParams("no file selected for compare".into()))?;
            let right =
                match object(p)?.and_then(|m| m.get("path")) {
                    Some(v) => abs_path(v.as_str().ok_or_else(|| {
                        FmError::InvalidParams("path must be a string".to_string())
                    })?)?,
                    None => selected_file(s)
                        .ok_or_else(|| FmError::InvalidParams("select one file".to_string()))?,
                };
            s.open_diff(DiffState::new(DiffSide::File(left), DiffSide::File(right)))
                .map(|_| ())
        },
    }
}

fn compare_selected() -> CommandSpec {
    CommandSpec {
        id: "diff.compare.selected",
        label: "Compare Two Selected Files",
        menu: &["View", "Compare Selected Files"],
        shortcut: None,
        params: "none",
        enabled: |s| two_files(s).is_some(),
        run: |s, _p: &Params| {
            let (a, b) = two_files(s)
                .ok_or_else(|| FmError::InvalidParams("select exactly two files".to_string()))?;
            s.open_diff(DiffState::new(DiffSide::File(a), DiffSide::File(b)))
                .map(|_| ())
        },
    }
}

fn two_files(s: &Session) -> Option<(String, String)> {
    match focused_selection(s).as_slice() {
        [a, b] => {
            let (a, b) = (local_file(a)?, local_file(b)?);
            (Path::new(&a).is_file() && Path::new(&b).is_file()).then_some((a, b))
        }
        _ => None,
    }
}

fn swap() -> CommandSpec {
    CommandSpec {
        id: "diff.swap",
        label: "Swap Sides",
        menu: &["View", "Diff", "Swap Sides"],
        shortcut: None,
        params: "none",
        enabled: focused_diff,
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            let d = s.diff_state_mut(pane)?;
            std::mem::swap(&mut d.left, &mut d.right);
            d.current = 0;
            d.scroll = 0.0;
            Ok(())
        },
    }
}

fn change_count(s: &mut Session, pane: PaneId) -> usize {
    match s.diff_view(pane).map(|v| &v.status) {
        Some(DiffStatus::Ready(o)) => match o.as_ref() {
            Outcome::Text(m) => m.changes.len(),
            Outcome::Binary { .. } => 0,
        },
        _ => 0,
    }
}

fn step(forward: bool) -> impl Fn(&mut Session, &Params) -> Result<(), FmError> {
    move |s, _p| {
        let pane = s.layout.focused();
        let n = change_count(s, pane);
        let d = s.diff_state_mut(pane)?;
        if n == 0 {
            return Ok(());
        }
        d.current = if forward {
            (d.current + 1).min(n - 1)
        } else {
            d.current.saturating_sub(1).min(n - 1)
        };
        Ok(())
    }
}

fn next() -> CommandSpec {
    CommandSpec {
        id: "diff.next",
        label: "Next Change",
        menu: &["View", "Diff", "Next Change"],
        shortcut: None,
        params: "none",
        enabled: focused_diff,
        run: |s, p: &Params| step(true)(s, p),
    }
}

fn prev() -> CommandSpec {
    CommandSpec {
        id: "diff.prev",
        label: "Previous Change",
        menu: &["View", "Diff", "Previous Change"],
        shortcut: None,
        params: "none",
        enabled: focused_diff,
        run: |s, p: &Params| step(false)(s, p),
    }
}

fn mode() -> CommandSpec {
    CommandSpec {
        id: "diff.mode",
        label: "Diff Layout",
        menu: &["View", "Diff", "Toggle Unified"],
        shortcut: None,
        params: "{mode?: \"split\"|\"unified\"} (default: toggle)",
        enabled: focused_diff,
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
            let pane = s.layout.focused();
            let d = s.diff_state_mut(pane)?;
            d.mode = wanted.unwrap_or(match d.mode {
                DiffMode::Split => DiffMode::Unified,
                DiffMode::Unified => DiffMode::Split,
            });
            Ok(())
        },
    }
}

fn eol() -> CommandSpec {
    CommandSpec {
        id: "diff.eol.toggle",
        label: "Ignore Line Endings",
        menu: &["View", "Diff", "Ignore Line Endings"],
        shortcut: None,
        params: "none",
        enabled: focused_diff,
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            let d = s.diff_state_mut(pane)?;
            d.ignore_eol = !d.ignore_eol;
            Ok(())
        },
    }
}

fn reload() -> CommandSpec {
    CommandSpec {
        id: "diff.reload",
        label: "Reload Comparison",
        menu: &["View", "Diff", "Reload"],
        shortcut: None,
        params: "none",
        enabled: focused_diff,
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            s.diff_state_mut(pane)?;
            s.diffs.views.remove(&pane);
            Ok(())
        },
    }
}

/// `diff.rev {side: "left"|"right", rev}` — point a committed side at another revision.
fn rev() -> CommandSpec {
    CommandSpec {
        id: "diff.rev",
        label: "Set Diff Revision",
        menu: &["View", "Diff", "Set Revision"],
        shortcut: None,
        params: "{side: \"left\"|\"right\", rev: string}",
        enabled: focused_diff,
        run: |s, p: &Params| {
            let map = object(p)?
                .ok_or_else(|| FmError::InvalidParams("side and rev required".to_string()))?;
            let side = map.get("side").and_then(Params::as_str);
            let rev = rev_text(
                map.get("rev")
                    .and_then(Params::as_str)
                    .ok_or_else(|| FmError::InvalidParams("rev required".to_string()))?,
            )?;
            let pane = s.layout.focused();
            let d = s.diff_state_mut(pane)?;
            let target = match side {
                Some("left") => &mut d.left,
                Some("right") => &mut d.right,
                _ => {
                    return Err(FmError::InvalidParams(
                        "side must be left or right".to_string(),
                    ));
                }
            };
            match target {
                DiffSide::Rev { rev: r, .. } => *r = rev,
                DiffSide::File(path) => {
                    let (repo, rel) = locate(path)?;
                    *target = DiffSide::Rev { repo, rev, rel };
                }
                DiffSide::Index { repo, rel } => {
                    *target = DiffSide::Rev {
                        repo: std::mem::take(repo),
                        rev,
                        rel: std::mem::take(rel),
                    };
                }
            }
            d.current = 0;
            Ok(())
        },
    }
}

/// `diff.side {side: "left"|"right", path, pane?}` — replace one side with a local file.
fn side_cmd() -> CommandSpec {
    CommandSpec {
        id: "diff.side",
        label: "Set Diff Side",
        menu: &["View", "Diff", "Set Side"],
        shortcut: None,
        params: "{side: \"left\"|\"right\", path: string, pane?: number}",
        enabled: focused_diff,
        run: |s, p: &Params| {
            let map = object(p)?
                .ok_or_else(|| FmError::InvalidParams("side and path required".to_string()))?;
            let path = abs_path(
                map.get("path")
                    .and_then(Params::as_str)
                    .ok_or_else(|| FmError::InvalidParams("path required".to_string()))?,
            )?;
            let left = match map.get("side").and_then(Params::as_str) {
                Some("left") => true,
                Some("right") => false,
                _ => {
                    return Err(FmError::InvalidParams(
                        "side must be left or right".to_string(),
                    ));
                }
            };
            let pane = match map.get("pane") {
                None | Some(Params::Null) => s.layout.focused(),
                Some(v) => v
                    .as_u64()
                    .ok_or_else(|| FmError::InvalidParams("invalid pane".to_string()))?,
            };
            let d = s.diff_state_mut(pane)?;
            *(if left { &mut d.left } else { &mut d.right }) = DiffSide::File(path);
            d.current = 0;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        side_cmd(),
        open_cmd(),
        compare_head(),
        mark_cmd(),
        compare_marked(),
        compare_selected(),
        swap(),
        next(),
        prev(),
        mode(),
        eol(),
        reload(),
        rev(),
    ]
}
