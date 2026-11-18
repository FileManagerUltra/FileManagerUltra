//! Running a reviewed plan. Content is copied through `fm-ops` (temp file + atomic commit),
//! attributes follow, deletes go deepest-first through the chosen policy and never use a
//! recursive remove. An item that fails is reported and skipped; only cancel stops the run.
//! For two-way sync the snapshot is rebuilt from what was verified, never from intentions.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fm_ops::{Backends, CancelFlag, Choice, ConflictPolicy, Decision, OpCtx, Progress};
use fm_vfs::{AttrFailKind, AttrReport, AttrSet, EntryKind, FmError, Vfs, VfsUri};

use crate::plan::{Action, Item, Plan, Status, uri_for};
use crate::profile::{DeletePolicy, OnAttrFailure, Retention, Side, SyncMode, parse_endpoint};
use crate::scan::{Node, NodeKind};
use crate::snapshot::{Fingerprint, Pair, Snapshot, SnapshotStore};

/// Entries visited when pruning or sizing the version archive.
const PRUNE_BUDGET: usize = 2_000_000;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncProgress {
    pub items_done: usize,
    pub items_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Clone, Debug, Default)]
pub struct RunReport {
    pub copied: usize,
    pub deleted: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
    /// Content is in place but an attribute could not be applied.
    pub attr_warnings: Vec<(String, String)>,
    pub warnings: Vec<String>,
    pub cancelled: bool,
    pub bytes: u64,
    pub snapshot_saved: bool,
}

pub struct RunOptions<'a> {
    pub confirmed: bool,
    pub now: SystemTime,
    pub cancel: CancelFlag,
    pub store: Option<&'a SnapshotStore>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Done,
    Failed,
}

struct Route {
    left_root: VfsUri,
    left: Arc<dyn Vfs>,
    right: Arc<dyn Vfs>,
}

impl Backends for Route {
    fn vfs(&self, uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        Ok(if self.left_root.is_same_or_descendant_of(uri) {
            self.left.clone()
        } else {
            self.right.clone()
        })
    }
}

pub fn run(
    plan: &Plan,
    left: Arc<dyn Vfs>,
    right: Arc<dyn Vfs>,
    opts: RunOptions<'_>,
    on_progress: &mut dyn FnMut(&SyncProgress),
) -> Result<RunReport, FmError> {
    if plan.guards.confirmation_required() && !opts.confirmed {
        return Err(FmError::InvalidParams(
            "this plan deletes files and needs confirmation first".into(),
        ));
    }
    let vfs_of = |s: Side| {
        if s == Side::Left {
            left.clone()
        } else {
            right.clone()
        }
    };
    let root_of = |s: Side| {
        if s == Side::Left {
            &plan.left_root
        } else {
            &plan.right_root
        }
    };
    let route = Route {
        left_root: plan.left_root.clone(),
        left: left.clone(),
        right: right.clone(),
    };
    let mut report = RunReport::default();
    let mut outcome: BTreeMap<&str, Outcome> = BTreeMap::new();
    let policy = plan.profile.on_attr_failure;

    // Attribute groups usable per direction: wanted, readable at the source, writable at the target.
    let mut eff = [AttrSet::NONE, AttrSet::NONE];
    for (i, to) in [Side::Left, Side::Right].into_iter().enumerate() {
        let from = to.other();
        let caps_from = vfs_of(from).attr_caps(root_of(from));
        let caps_to = vfs_of(to).attr_caps(root_of(to));
        let wanted = plan.profile.attrs;
        let usable = wanted.intersect(caps_from.read).intersect(caps_to.write);
        if let Some(slot) = eff.get_mut(i) {
            *slot = usable;
        }
        let dropped: Vec<&str> = wanted
            .iter()
            .filter(|g| !usable.contains(*g))
            .map(|g| g.label())
            .collect();
        let used = plan.items.iter().any(|it| it.action == Action::copy_to(to));
        if used && !dropped.is_empty() {
            report.warnings.push(format!(
                "not carried over to the {} folder (unsupported there): {}",
                if to == Side::Left { "left" } else { "right" },
                dropped.join(", ")
            ));
        }
    }
    let eff_for = |a: Action| {
        eff.get(usize::from(a == Action::CopyToRight))
            .copied()
            .unwrap_or(AttrSet::NONE)
    };

    let work: Vec<&Item> = plan
        .items
        .iter()
        .filter(|i| i.action.is_copy() || i.action.is_delete())
        .collect();
    let mut progress = SyncProgress {
        items_total: work.len(),
        bytes_total: plan.summary.bytes,
        ..SyncProgress::default()
    };
    on_progress(&progress);
    let mut last_emit = Instant::now();

    let fail = |report: &mut RunReport, rel: &str, e: &dyn std::fmt::Display| {
        report.failed.push((rel.to_string(), e.to_string()));
    };

    // 0. A target folder that does not exist yet (first sync).
    for side in &plan.missing_roots {
        if work.iter().any(|i| i.action == Action::copy_to(*side)) {
            ensure_dir(vfs_of(*side).as_ref(), root_of(*side))?;
        }
    }

    // 1. Folders to create, parents first.
    let mut dirs: Vec<&Item> = work
        .iter()
        .copied()
        .filter(|i| i.action.is_copy() && i.kind() == NodeKind::Dir)
        .collect();
    dirs.sort_by_key(|i| i.depth());
    for it in &dirs {
        if opts.cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }
        let to = if it.action == Action::CopyToLeft {
            Side::Left
        } else {
            Side::Right
        };
        let res = uri_for(root_of(to), &it.rel).and_then(|u| ensure_dir(vfs_of(to).as_ref(), &u));
        match res {
            Ok(()) => {
                outcome.insert(&it.rel, Outcome::Done);
                report.copied += 1;
            }
            Err(e) => {
                fail(&mut report, &it.rel, &e);
                outcome.insert(&it.rel, Outcome::Failed);
            }
        }
        progress.items_done += 1;
    }
    on_progress(&progress);

    // 2. Files and links.
    let files: Vec<&Item> = work
        .iter()
        .copied()
        .filter(|i| i.action.is_copy() && i.kind() != NodeKind::Dir)
        .collect();
    let mut bytes_base = 0u64;
    for it in files {
        if report.cancelled || opts.cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }
        let (from, to) = if it.action == Action::CopyToLeft {
            (Side::Right, Side::Left)
        } else {
            (Side::Left, Side::Right)
        };
        let size = it.node(from).map_or(0, |n| n.size);
        let res = (|| -> Result<fm_ops::OpReport, FmError> {
            let src = uri_for(root_of(from), &it.rel)?;
            let dst = uri_for(root_of(to), &it.rel)?;
            let parent = dst
                .parent()
                .ok_or_else(|| FmError::InvalidParams(it.rel.clone()))?;
            ensure_dir(vfs_of(to).as_ref(), &parent)?;
            let mut on_prog = |p: &Progress| {
                if last_emit.elapsed() >= PROGRESS_EVERY {
                    last_emit = Instant::now();
                    let mut snap = progress;
                    snap.bytes_done = bytes_base + p.bytes.done;
                    on_progress(&snap);
                }
            };
            let mut on_conflict = |_: &fm_ops::Conflict| Decision {
                choice: Choice::Overwrite,
                apply_to_all: true,
            };
            let mut ctx = OpCtx::new(ConflictPolicy::Overwrite, &mut on_prog, &mut on_conflict)
                .with_attrs(eff_for(it.action))
                .with_retarget_links(false)
                .with_cancel(opts.cancel.clone());
            fm_ops::copy(&route, &[src], &parent, &mut ctx)
        })();
        match res {
            Ok(r) if r.cancelled => {
                report.cancelled = true;
                break;
            }
            Ok(r) if !r.failed.is_empty() => {
                if let Some((_, e)) = r.failed.first() {
                    fail(&mut report, &it.rel, e);
                }
                outcome.insert(&it.rel, Outcome::Failed);
            }
            Ok(r) => {
                let mut ok = true;
                for (_, rep) in &r.attr_incomplete {
                    ok &= note_attr_failure(&mut report, policy, &it.rel, rep);
                }
                if ok {
                    outcome.insert(&it.rel, Outcome::Done);
                    report.copied += 1;
                    report.bytes += size;
                } else {
                    outcome.insert(&it.rel, Outcome::Failed);
                }
            }
            Err(e) => {
                fail(&mut report, &it.rel, &e);
                outcome.insert(&it.rel, Outcome::Failed);
            }
        }
        bytes_base += size;
        progress.items_done += 1;
        progress.bytes_done = bytes_base;
        if last_emit.elapsed() >= PROGRESS_EVERY {
            last_emit = Instant::now();
            on_progress(&progress);
        }
    }

    // 3. Folder attributes last, deepest first, so the children don't disturb the times.
    if !report.cancelled {
        for it in dirs.iter().rev() {
            if outcome.get(it.rel.as_str()) != Some(&Outcome::Done) {
                continue;
            }
            let (from, to) = if it.action == Action::CopyToLeft {
                (Side::Right, Side::Left)
            } else {
                (Side::Left, Side::Right)
            };
            let set = eff_for(it.action);
            if set.is_empty() {
                continue;
            }
            let res = (|| -> Result<AttrReport, FmError> {
                let src = uri_for(root_of(from), &it.rel)?;
                let dst = uri_for(root_of(to), &it.rel)?;
                let attrs = vfs_of(from).stat_full(&src)?;
                vfs_of(to).set_attrs(&dst, &attrs, set)
            })();
            match res {
                Ok(rep) if !rep.is_complete() => {
                    if !note_attr_failure(&mut report, policy, &it.rel, &rep) {
                        outcome.insert(&it.rel, Outcome::Failed);
                    }
                }
                Ok(_) => {}
                Err(e) => report.attr_warnings.push((it.rel.clone(), e.to_string())),
            }
        }
    }

    // 4. Deletes, deepest first. Only files, links and empty folders are ever removed.
    let mut dels: Vec<&Item> = work
        .iter()
        .copied()
        .filter(|i| i.action.is_delete())
        .collect();
    dels.sort_by(|a, b| b.depth().cmp(&a.depth()).then_with(|| b.rel.cmp(&a.rel)));
    let mut version_dirs: BTreeMap<Side, (VfsUri, Retention, VfsUri)> = BTreeMap::new();
    for it in dels {
        if report.cancelled || opts.cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }
        let side = if it.action == Action::DeleteLeft {
            Side::Left
        } else {
            Side::Right
        };
        let is_dir = it.kind() == NodeKind::Dir;
        let res = (|| -> Result<(), FmError> {
            let uri = uri_for(root_of(side), &it.rel)?;
            let vfs = vfs_of(side);
            match plan.profile.delete_policy(side) {
                _ if is_dir => remove_leaf(vfs.as_ref(), &uri),
                DeletePolicy::Permanent => remove_leaf(vfs.as_ref(), &uri),
                DeletePolicy::Trash => {
                    let mut p = |_: &Progress| {};
                    let mut c = |_: &fm_ops::Conflict| Decision {
                        choice: Choice::Skip,
                        apply_to_all: true,
                    };
                    let mut ctx = OpCtx::new(ConflictPolicy::Skip, &mut p, &mut c)
                        .with_cancel(opts.cancel.clone());
                    let r = fm_ops::trash_items(&[uri], &mut ctx);
                    match r.failed.into_iter().next() {
                        Some((_, e)) => Err(e),
                        None => Ok(()),
                    }
                }
                DeletePolicy::Versioning { folder, retention } => {
                    if let Entry::Vacant(slot) = version_dirs.entry(side) {
                        let base = parse_endpoint(&folder)?;
                        let run_dir = unique_run_dir(vfs.as_ref(), &base, opts.now)?;
                        slot.insert((base, retention, run_dir));
                    }
                    let Some((_, _, run_dir)) = version_dirs.get(&side) else {
                        return Err(FmError::InvalidParams("versioning folder".into()));
                    };
                    let dest = uri_for(run_dir, &it.rel)?;
                    let parent = dest
                        .parent()
                        .ok_or_else(|| FmError::InvalidParams(it.rel.clone()))?;
                    ensure_dir(vfs.as_ref(), &parent)?;
                    vfs.rename(&uri, &dest)
                }
            }
        })();
        match res {
            Ok(()) | Err(FmError::NotFound(_)) => {
                outcome.insert(&it.rel, Outcome::Done);
                report.deleted += 1;
            }
            Err(FmError::NotEmpty(_)) => {
                report
                    .warnings
                    .push(format!("{}: kept, the folder is not empty", it.rel));
                outcome.insert(&it.rel, Outcome::Failed);
            }
            Err(FmError::Cancelled) => {
                report.cancelled = true;
                break;
            }
            Err(e) => {
                fail(&mut report, &it.rel, &e);
                outcome.insert(&it.rel, Outcome::Failed);
            }
        }
        progress.items_done += 1;
    }
    for (side, (base, retention, keep)) in &version_dirs {
        if let Err(e) = prune_versions(vfs_of(*side).as_ref(), base, keep, retention, opts.now) {
            report
                .warnings
                .push(format!("could not prune old versions: {e}"));
        }
    }
    report.skipped = plan
        .items
        .iter()
        .filter(|i| i.action == Action::Skip)
        .count();
    on_progress(&progress);

    // 5. Two-way history, from verified state only.
    if plan.profile.mode == SyncMode::TwoWay
        && let Some(store) = opts.store
    {
        let snap = rebuild_snapshot(plan, &outcome, &left, &right);
        match store.save(&plan.profile.id, &snap) {
            Ok(()) => report.snapshot_saved = true,
            Err(e) => report
                .warnings
                .push(format!("could not save the sync history: {e}")),
        }
    }
    Ok(report)
}

/// Returns `false` when the policy turns the attribute failure into a failed item.
fn note_attr_failure(
    report: &mut RunReport,
    policy: OnAttrFailure,
    rel: &str,
    rep: &AttrReport,
) -> bool {
    if rep.is_complete() || policy == OnAttrFailure::Ignore {
        return true;
    }
    let msg = rep
        .failed
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    match policy {
        OnAttrFailure::Error => {
            report
                .failed
                .push((rel.to_string(), format!("attributes: {msg}")));
            false
        }
        _ => {
            // A group the other side can never hold is reported once per run, not per file.
            let unsupported_only = rep
                .failed
                .iter()
                .all(|f| f.kind == AttrFailKind::Unsupported);
            if !unsupported_only {
                report.attr_warnings.push((rel.to_string(), msg));
            }
            true
        }
    }
}

fn ensure_dir(vfs: &dyn Vfs, uri: &VfsUri) -> Result<(), FmError> {
    match vfs.stat(uri) {
        Ok(e) if e.is_dir() => return Ok(()),
        Ok(_) => return Err(FmError::NotADirectory(uri.to_string())),
        Err(FmError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    if let Some(p) = uri.parent() {
        ensure_dir(vfs, &p)?;
    }
    match vfs.create_dir(uri) {
        Ok(()) | Err(FmError::AlreadyExists(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

fn remove_leaf(vfs: &dyn Vfs, uri: &VfsUri) -> Result<(), FmError> {
    vfs.remove(uri)
}

// ---- versioning ----

fn stamp(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Inverse of [`stamp`] for pruning by age; `None` for names we did not create.
fn parse_stamp(name: &str) -> Option<u64> {
    let base = name.get(..15)?;
    let (date, time) = base.split_once('-')?;
    if date.len() != 8
        || time.len() != 6
        || !base
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 8 || b.is_ascii_digit())
    {
        return None;
    }
    if let Some(rest) = name.get(15..)
        && !(rest.is_empty()
            || (rest.starts_with('-') && rest.get(1..)?.bytes().all(|b| b.is_ascii_digit())))
    {
        return None;
    }
    let n = |s: &str| s.parse::<i64>().ok();
    let (y, m, d) = (n(date.get(..4)?)?, n(date.get(4..6)?)?, n(date.get(6..8)?)?);
    let (hh, mm, ss) = (n(time.get(..2)?)?, n(time.get(2..4)?)?, n(time.get(4..6)?)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

fn unique_run_dir(vfs: &dyn Vfs, base: &VfsUri, now: SystemTime) -> Result<VfsUri, FmError> {
    ensure_dir(vfs, base)?;
    let s = stamp(now);
    for n in 0..1000 {
        let name = if n == 0 {
            s.clone()
        } else {
            format!("{s}-{n}")
        };
        let dir = base.join(&name)?;
        match vfs.stat(&dir) {
            Err(FmError::NotFound(_)) => {
                vfs.create_dir(&dir)?;
                return Ok(dir);
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }
    }
    Err(FmError::InvalidParams("no free version folder name".into()))
}

fn remove_tree(vfs: &dyn Vfs, uri: &VfsUri, budget: &mut usize) -> Result<(), FmError> {
    for e in vfs.read_dir(uri)? {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| FmError::InvalidParams("version archive too large to prune".into()))?;
        if matches!(e.kind, EntryKind::Dir) {
            remove_tree(vfs, &e.uri, budget)?;
        } else {
            vfs.remove(&e.uri)?;
        }
    }
    vfs.remove(uri)
}

fn tree_size(vfs: &dyn Vfs, uri: &VfsUri, budget: &mut usize) -> Result<u64, FmError> {
    let mut total = 0u64;
    for e in vfs.read_dir(uri)? {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| FmError::InvalidParams("version archive too large to size".into()))?;
        total = total.saturating_add(if matches!(e.kind, EntryKind::Dir) {
            tree_size(vfs, &e.uri, budget)?
        } else {
            e.size.unwrap_or(0)
        });
    }
    Ok(total)
}

/// Drop whole run folders by age, count and total size. Only folders named like ours are touched,
/// and the folder of the current run is never removed.
pub(crate) fn prune_versions(
    vfs: &dyn Vfs,
    base: &VfsUri,
    keep: &VfsUri,
    r: &Retention,
    now: SystemTime,
) -> Result<(), FmError> {
    if r.max_versions.is_none() && r.max_age_days.is_none() && r.max_bytes.is_none() {
        return Ok(());
    }
    let mut runs: Vec<(u64, VfsUri)> = vfs
        .read_dir(base)?
        .into_iter()
        .filter(|e| matches!(e.kind, EntryKind::Dir))
        .filter_map(|e| parse_stamp(&e.name).map(|t| (t, e.uri)))
        .collect();
    runs.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.to_string().cmp(&a.1.to_string()))
    });
    let now_s = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut budget = PRUNE_BUDGET;
    let mut kept = 0u32;
    let mut bytes = 0u64;
    for (t, uri) in runs {
        let current = &uri == keep;
        let too_old = r
            .max_age_days
            .is_some_and(|d| now_s.saturating_sub(t) > u64::from(d) * 86_400);
        let size = if r.max_bytes.is_some() {
            tree_size(vfs, &uri, &mut budget)?
        } else {
            0
        };
        let over_count = r.max_versions.is_some_and(|m| kept >= m);
        let over_bytes = r.max_bytes.is_some_and(|m| bytes.saturating_add(size) > m);
        if !current && (too_old || over_count || over_bytes) {
            remove_tree(vfs, &uri, &mut budget)?;
        } else {
            kept += 1;
            bytes = bytes.saturating_add(size);
        }
    }
    Ok(())
}

// ---- snapshot ----

fn node_of(vfs: &dyn Vfs, uri: &VfsUri) -> Option<Node> {
    let e = vfs.stat(uri).ok()?;
    let (kind, link) = match &e.kind {
        EntryKind::File => (NodeKind::File, None),
        EntryKind::Dir => (NodeKind::Dir, None),
        EntryKind::Symlink { target } => (NodeKind::Symlink, Some(target.clone())),
        _ => return None,
    };
    Some(Node {
        kind,
        size: if kind == NodeKind::File {
            e.size.unwrap_or(0)
        } else {
            0
        },
        mtime: e.modified,
        link,
    })
}

fn rebuild_snapshot(
    plan: &Plan,
    outcome: &BTreeMap<&str, Outcome>,
    left: &Arc<dyn Vfs>,
    right: &Arc<dyn Vfs>,
) -> Snapshot {
    let mut snap = Snapshot::new(&plan.left_root.to_string(), &plan.right_root.to_string());
    for it in &plan.items {
        let pair = match (it.action, outcome.get(it.rel.as_str())) {
            (Action::None, _) if it.status == Status::Equal => match (&it.left, &it.right) {
                (Some(l), Some(r)) => Some(Pair {
                    left: Fingerprint::of(l),
                    right: Fingerprint::of(r),
                }),
                _ => None,
            },
            (a, Some(Outcome::Done)) if a.is_copy() => {
                let nodes = (|| {
                    let l = node_of(left.as_ref(), &uri_for(&plan.left_root, &it.rel).ok()?)?;
                    let r = node_of(right.as_ref(), &uri_for(&plan.right_root, &it.rel).ok()?)?;
                    Some((l, r))
                })();
                nodes.filter(|(l, r)| l.kind == r.kind).map(|(l, r)| Pair {
                    left: Fingerprint::of(&l),
                    right: Fingerprint::of(&r),
                })
            }
            (a, Some(Outcome::Done)) if a.is_delete() => None,
            _ => plan
                .base
                .as_ref()
                .and_then(|b| b.entries.get(&it.rel))
                .cloned(),
        };
        if let Some(p) = pair {
            snap.entries.insert(it.rel.clone(), p);
        }
    }
    snap
}
