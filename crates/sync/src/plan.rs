//! The comparison result: one [`Item`] per path with a status and a proposed action the
//! user can review and override before anything runs. Building a plan never writes.

use std::collections::BTreeSet;
use std::time::Duration;

use fm_ops::CancelFlag;
use fm_vfs::{FmError, Vfs, VfsUri};

use crate::compare::{files_equal, hash_file};
use crate::profile::{DeletePolicy, Profile, Side, SyncMode};
use crate::scan::{Node, NodeKind, Scan};
use crate::snapshot::{Fingerprint, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Equal,
    /// Exists only on this side.
    OnlyOn(Side),
    /// This side differs and is the one to propagate (or is newer).
    Changed(Side),
    /// This side removed it since the last sync (two-way).
    DeletedOn(Side),
    /// Both sides changed, or the two are different kinds.
    Conflict,
    /// A side could not be read here; nothing is known, nothing is done.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Skip,
    CopyToLeft,
    CopyToRight,
    DeleteLeft,
    DeleteRight,
}

impl Action {
    pub fn is_copy(self) -> bool {
        matches!(self, Action::CopyToLeft | Action::CopyToRight)
    }

    pub fn is_delete(self) -> bool {
        matches!(self, Action::DeleteLeft | Action::DeleteRight)
    }

    pub fn copy_to(side: Side) -> Action {
        match side {
            Side::Left => Action::CopyToLeft,
            Side::Right => Action::CopyToRight,
        }
    }

    pub fn delete_on(side: Side) -> Action {
        match side {
            Side::Left => Action::DeleteLeft,
            Side::Right => Action::DeleteRight,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub rel: String,
    pub left: Option<Node>,
    pub right: Option<Node>,
    pub status: Status,
    pub action: Action,
    pub note: Option<String>,
}

impl Item {
    pub fn node(&self, side: Side) -> Option<&Node> {
        match side {
            Side::Left => self.left.as_ref(),
            Side::Right => self.right.as_ref(),
        }
    }

    /// Kind of whichever side exists (the left one when both do).
    pub fn kind(&self) -> NodeKind {
        self.left
            .as_ref()
            .or(self.right.as_ref())
            .map_or(NodeKind::File, |n| n.kind)
    }

    pub fn depth(&self) -> usize {
        self.rel.matches('/').count()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub equal: usize,
    pub copy_left: usize,
    pub copy_right: usize,
    pub delete_left: usize,
    pub delete_right: usize,
    pub conflicts: usize,
    pub skipped: usize,
    pub bytes: u64,
}

/// Safeguards. A run with any of these tripped needs explicit confirmation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Guards {
    pub deletes: usize,
    /// Highest share (percent) of one side that would be deleted.
    pub delete_percent: u8,
    pub over_limit: bool,
    /// One side has no entries while deletes are planned on the other.
    pub empty_side: bool,
    /// At least one delete cannot be undone (no trash, no versioning).
    pub permanent: bool,
}

impl Guards {
    pub fn confirmation_required(&self) -> bool {
        self.deletes > 0 && (self.over_limit || self.empty_side || self.permanent)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotState {
    /// Not two-way, so no history is used.
    NotUsed,
    Missing,
    Used,
    Invalid(String),
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub profile: Profile,
    pub left_root: VfsUri,
    pub right_root: VfsUri,
    pub items: Vec<Item>,
    pub summary: Summary,
    pub guards: Guards,
    pub warnings: Vec<String>,
    pub snapshot_state: SnapshotState,
    /// The snapshot the plan was made against (two-way only).
    pub base: Option<Snapshot>,
    /// Roots that do not exist yet; a run creates one when something is copied into it.
    pub missing_roots: Vec<Side>,
    left_total: usize,
    right_total: usize,
}

pub struct PlanInput<'a> {
    pub profile: &'a Profile,
    pub left: &'a Scan,
    pub right: &'a Scan,
    pub left_vfs: &'a dyn Vfs,
    pub right_vfs: &'a dyn Vfs,
    pub snapshot: Option<&'a Snapshot>,
    pub snapshot_state: SnapshotState,
    pub cancel: &'a CancelFlag,
}

pub fn uri_for(root: &VfsUri, rel: &str) -> Result<VfsUri, FmError> {
    let mut u = root.clone();
    for seg in rel.split('/') {
        u = u.join(seg)?;
    }
    Ok(u)
}

fn under(rel: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| {
        rel.strip_prefix(d.as_str())
            .is_some_and(|rest| d.is_empty() || rest.starts_with('/'))
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Unchanged,
    Changed,
    Deleted,
}

fn classify(now: Option<&Node>, then: &Fingerprint, tol: Duration) -> Change {
    match now {
        None => Change::Deleted,
        Some(n) if then.matches(n, tol) => Change::Unchanged,
        Some(_) => Change::Changed,
    }
}

impl Plan {
    pub fn build(input: PlanInput<'_>) -> Result<Plan, FmError> {
        let p = input.profile;
        let (left_root, right_root) = (p.left_uri()?, p.right_uri()?);
        let tol = Duration::from_secs(u64::from(p.tolerance_secs));
        let mut warnings: Vec<String> = Vec::new();
        warnings.extend(input.left.warnings.iter().map(|w| format!("left: {w}")));
        warnings.extend(input.right.warnings.iter().map(|w| format!("right: {w}")));

        let rels: BTreeSet<&String> = input
            .left
            .tree
            .keys()
            .chain(input.right.tree.keys())
            .collect();
        let mut items = Vec::with_capacity(rels.len());
        for rel in rels {
            if input.cancel.is_cancelled() {
                return Err(FmError::Cancelled);
            }
            let l = input.left.tree.get(rel);
            let r = input.right.tree.get(rel);
            let mut equal = |l: &Node, r: &Node| -> Result<bool, FmError> {
                match l.kind {
                    NodeKind::Dir => Ok(true),
                    NodeKind::Symlink => Ok(l.link == r.link),
                    NodeKind::File => {
                        let mut hash = |left: bool| {
                            let (vfs, root) = if left {
                                (input.left_vfs, &left_root)
                            } else {
                                (input.right_vfs, &right_root)
                            };
                            hash_file(vfs, &uri_for(root, rel)?, input.cancel)
                        };
                        files_equal(p.compare, tol, l, r, &mut hash)
                    }
                }
            };
            let (status, action, note) = decide(
                p,
                rel,
                l,
                r,
                input.snapshot,
                tol,
                &mut equal,
                input.left,
                input.right,
            )?;
            items.push(Item {
                rel: rel.clone(),
                left: l.cloned(),
                right: r.cloned(),
                status,
                action,
                note,
            });
        }
        let mut plan = Plan {
            profile: p.clone(),
            left_root,
            right_root,
            items,
            summary: Summary::default(),
            guards: Guards::default(),
            warnings,
            snapshot_state: input.snapshot_state,
            base: input.snapshot.cloned(),
            missing_roots: [
                (Side::Left, input.left.missing),
                (Side::Right, input.right.missing),
            ]
            .into_iter()
            .filter_map(|(side, missing)| missing.then_some(side))
            .collect(),
            left_total: input.left.tree.len(),
            right_total: input.right.tree.len(),
        };
        plan.refresh();
        Ok(plan)
    }

    /// Recompute summary and guards after any action changed.
    pub fn refresh(&mut self) {
        let mut s = Summary::default();
        let (mut dl, mut dr) = (0usize, 0usize);
        for it in &self.items {
            match it.action {
                Action::CopyToLeft => {
                    s.copy_left += 1;
                    s.bytes += it.right.as_ref().map_or(0, |n| n.size);
                }
                Action::CopyToRight => {
                    s.copy_right += 1;
                    s.bytes += it.left.as_ref().map_or(0, |n| n.size);
                }
                Action::DeleteLeft => dl += 1,
                Action::DeleteRight => dr += 1,
                Action::Skip => s.skipped += 1,
                Action::None => {}
            }
            if it.status == Status::Equal {
                s.equal += 1;
            }
            if it.status == Status::Conflict {
                s.conflicts += 1;
            }
        }
        s.delete_left = dl;
        s.delete_right = dr;
        let pct = |n: usize, total: usize| -> u8 {
            if n == 0 {
                0
            } else {
                u8::try_from((n * 100) / total.max(1))
                    .unwrap_or(100)
                    .min(100)
            }
        };
        let (pl, pr) = (pct(dl, self.left_total), pct(dr, self.right_total));
        let limit = self.profile.max_delete_percent;
        let permanent = [(Side::Left, dl), (Side::Right, dr)]
            .into_iter()
            .any(|(side, n)| n > 0 && self.profile.delete_policy(side) == DeletePolicy::Permanent);
        self.guards = Guards {
            deletes: dl + dr,
            delete_percent: pl.max(pr),
            over_limit: pl > limit || pr > limit,
            empty_side: (dr > 0 && self.left_total == 0) || (dl > 0 && self.right_total == 0),
            permanent,
        };
        self.summary = s;
    }

    pub fn find(&self, rel: &str) -> Option<&Item> {
        self.items
            .binary_search_by(|i| i.rel.as_str().cmp(rel))
            .ok()
            .and_then(|i| self.items.get(i))
    }

    /// Per-row override. Only actions that make sense for the item and the mode are accepted.
    pub fn set_action(&mut self, rel: &str, action: Action) -> Result<(), FmError> {
        let idx = self
            .items
            .binary_search_by(|i| i.rel.as_str().cmp(rel))
            .map_err(|_| FmError::NotFound(rel.to_string()))?;
        self.apply_action(idx, action)?;
        self.refresh();
        Ok(())
    }

    /// Set `action` on a folder and everything below it. Items the action does not make sense
    /// for are left as they were; returns how many changed, and errors if none could.
    pub fn set_action_tree(&mut self, rel: &str, action: Action) -> Result<usize, FmError> {
        let prefix = format!("{rel}/");
        let start = self
            .items
            .partition_point(|i| i.rel.as_str() < prefix.as_str());
        let mut indices: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .skip(start)
            .take_while(|(_, i)| i.rel.starts_with(&prefix))
            .map(|(n, _)| n)
            .collect();
        if let Ok(own) = self.items.binary_search_by(|i| i.rel.as_str().cmp(rel)) {
            indices.push(own);
        }
        let mut changed = 0;
        let mut first_error = None;
        for idx in indices {
            match self.apply_action(idx, action) {
                Ok(()) => changed += 1,
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        if changed == 0 {
            return Err(first_error.unwrap_or_else(|| FmError::NotFound(rel.to_string())));
        }
        self.refresh();
        Ok(changed)
    }

    fn apply_action(&mut self, idx: usize, action: Action) -> Result<(), FmError> {
        let mode = self.profile.mode;
        let source = self.profile.source;
        let bad = |m: &str| Err(FmError::InvalidParams(m.to_string()));
        let Some(item) = self.items.get_mut(idx) else {
            return Err(FmError::NotFound(idx.to_string()));
        };
        match action {
            Action::None | Action::Skip => {}
            _ if item.status == Status::Unknown => {
                return bad("this path could not be read; nothing can be done with it");
            }
            _ if matches!((&item.left, &item.right), (Some(l), Some(r)) if l.kind != r.kind) => {
                return bad("a file and a folder share this name; resolve it by hand");
            }
            Action::CopyToLeft | Action::CopyToRight | Action::DeleteLeft | Action::DeleteRight => {
                let (touched, from) = match action {
                    Action::CopyToLeft => (Side::Left, Some(Side::Right)),
                    Action::CopyToRight => (Side::Right, Some(Side::Left)),
                    Action::DeleteLeft => (Side::Left, None),
                    _ => (Side::Right, None),
                };
                if let Some(from) = from
                    && item.node(from).is_none()
                {
                    return bad("there is nothing on the source side to copy");
                }
                if action.is_delete() && item.node(touched).is_none() {
                    return bad("there is nothing on that side to delete");
                }
                if mode != SyncMode::TwoWay && touched == source {
                    return bad("a one-way sync never changes its source folder");
                }
                if mode == SyncMode::Update && action.is_delete() {
                    return bad("an update sync never deletes");
                }
            }
        }
        item.action = action;
        Ok(())
    }

    pub fn is_empty_work(&self) -> bool {
        !self
            .items
            .iter()
            .any(|i| i.action.is_copy() || i.action.is_delete())
    }
}

type Decision = (Status, Action, Option<String>);

#[allow(clippy::too_many_arguments)]
fn decide(
    p: &Profile,
    rel: &str,
    l: Option<&Node>,
    r: Option<&Node>,
    snap: Option<&Snapshot>,
    tol: Duration,
    equal: &mut dyn FnMut(&Node, &Node) -> Result<bool, FmError>,
    lscan: &Scan,
    rscan: &Scan,
) -> Result<Decision, FmError> {
    let note = |s: &str| Some(s.to_string());
    // A side we could not list says nothing about what is (not) in it.
    match (l, r) {
        (Some(_), None) if under(rel, &rscan.unreadable) => {
            return Ok((
                Status::Unknown,
                Action::Skip,
                note("the other side could not be read"),
            ));
        }
        (None, Some(_)) if under(rel, &lscan.unreadable) => {
            return Ok((
                Status::Unknown,
                Action::Skip,
                note("the other side could not be read"),
            ));
        }
        _ => {}
    }
    let (l, r) = match (l, r) {
        (None, None) => return Ok((Status::Equal, Action::None, None)),
        (Some(l), Some(r)) => (l, r),
        (Some(_), None) => return Ok(one_sided(p, Side::Left, snap, rel, l, r, tol)),
        (None, Some(_)) => return Ok(one_sided(p, Side::Right, snap, rel, l, r, tol)),
    };
    if l.kind != r.kind {
        return Ok((
            Status::Conflict,
            Action::Skip,
            note("a file and a folder share this name"),
        ));
    }
    let same = equal(l, r)?;
    if same {
        return Ok((Status::Equal, Action::None, None));
    }
    match p.mode {
        SyncMode::Mirror | SyncMode::Update => {
            let src = p.source;
            let (sn, tn) = if src == Side::Left { (l, r) } else { (r, l) };
            let target_newer = matches!((tn.mtime, sn.mtime), (Some(t), Some(s)) if t > s)
                && !crate::compare::times_equal(tn.mtime, sn.mtime, tol);
            if p.mode == SyncMode::Update && target_newer {
                return Ok((
                    Status::Changed(src.other()),
                    Action::Skip,
                    note("the target is newer and is left alone"),
                ));
            }
            Ok((Status::Changed(src), Action::copy_to(src.other()), None))
        }
        SyncMode::TwoWay => {
            let entry = snap.and_then(|s| s.entries.get(rel));
            let Some(e) = entry else {
                return Ok((
                    Status::Conflict,
                    Action::Skip,
                    note("differs and has no sync history"),
                ));
            };
            let cl = classify(Some(l), &e.left, tol);
            let cr = classify(Some(r), &e.right, tol);
            Ok(match (cl, cr) {
                (Change::Changed, Change::Unchanged) => {
                    (Status::Changed(Side::Left), Action::CopyToRight, None)
                }
                (Change::Unchanged, Change::Changed) => {
                    (Status::Changed(Side::Right), Action::CopyToLeft, None)
                }
                _ => (
                    Status::Conflict,
                    Action::Skip,
                    note("changed on both sides"),
                ),
            })
        }
    }
}

/// `side` holds the path, the other side does not.
fn one_sided(
    p: &Profile,
    side: Side,
    snap: Option<&Snapshot>,
    rel: &str,
    l: Option<&Node>,
    r: Option<&Node>,
    tol: Duration,
) -> Decision {
    let present = if side == Side::Left { l } else { r };
    let other = side.other();
    match p.mode {
        SyncMode::Mirror | SyncMode::Update => {
            if side == p.source {
                (Status::OnlyOn(side), Action::copy_to(other), None)
            } else if p.mode == SyncMode::Mirror {
                (Status::OnlyOn(side), Action::delete_on(side), None)
            } else {
                (Status::OnlyOn(side), Action::None, None)
            }
        }
        SyncMode::TwoWay => {
            let entry = snap.and_then(|s| s.entries.get(rel));
            let Some(e) = entry else {
                // New since the last sync (or no history): safe to copy, never to delete.
                return (Status::OnlyOn(side), Action::copy_to(other), None);
            };
            let then = if side == Side::Left {
                &e.left
            } else {
                &e.right
            };
            if classify(present, then, tol) == Change::Unchanged {
                // Gone on the other side since the last sync: propagate the delete.
                (Status::DeletedOn(other), Action::delete_on(side), None)
            } else {
                (
                    Status::Conflict,
                    Action::Skip,
                    Some("changed on one side and deleted on the other".to_string()),
                )
            }
        }
    }
}
