//! Copy and move pipelines (design D2–D5): execute a [`Plan`] item by item, stream
//! through a bounded buffer into a temp name, commit by rename, never leave partial
//! files, never delete a move source before the copy is verified.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use fm_vfs::{Entry, EntryKind, FmError, Scheme, Vfs, VfsUri, WriteOpts};

use crate::ConflictPolicy;
use crate::ctx::{Backends, Choice, Conflict, ConflictSide, OpCtx, OpReport, Progress};
use crate::names::keep_both_name;
use crate::plan::{Plan, PlanItem, PlanKind, Top};
use crate::symlink::retarget;

const BUF_BYTES: usize = 1024 * 1024;
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Copy every top-level source in `plan`.
pub fn run_copy(backends: &dyn Backends, plan: &Plan, ctx: &mut OpCtx<'_>) -> OpReport {
    Runner::new(backends, plan, ctx).run(false)
}

/// Move every top-level source in `plan` (rename when possible, else copy → verify → delete).
pub fn run_move(backends: &dyn Backends, plan: &Plan, ctx: &mut OpCtx<'_>) -> OpReport {
    Runner::new(backends, plan, ctx).run(true)
}

enum Outcome {
    /// Written; `created` is the destination when this job brought it into existence.
    Done {
        created: Option<VfsUri>,
    },
    Skipped,
    Failed(FmError),
    Cancelled,
}

struct Runner<'a, 'b> {
    backends: &'a dyn Backends,
    plan: &'a Plan,
    ctx: &'a mut OpCtx<'b>,
    report: OpReport,
    progress: Progress,
    sticky: Option<Choice>,
    direct_write: bool,
    /// `(source root, destination root)` of the top-level item being transferred.
    roots: Option<(String, String)>,
}

impl<'a, 'b> Runner<'a, 'b> {
    fn new(backends: &'a dyn Backends, plan: &'a Plan, ctx: &'a mut OpCtx<'b>) -> Self {
        let mut progress = Progress::default();
        progress.items.total = plan.total_items();
        progress.bytes.total = plan.total_bytes;
        Self {
            backends,
            plan,
            ctx,
            report: OpReport::default(),
            progress,
            sticky: None,
            direct_write: false,
            roots: None,
        }
    }

    fn emit(&mut self) {
        (self.ctx.on_progress)(&self.progress);
    }

    fn run(mut self, moving: bool) -> OpReport {
        self.emit();
        for top in &self.plan.tops {
            if self.ctx.cancel.is_cancelled() {
                self.report.cancelled = true;
                break;
            }
            if moving && self.try_rename(top) {
                continue;
            }
            self.transfer_top(top, moving);
            if self.report.cancelled {
                break;
            }
        }
        self.report
    }

    /// Same-authority, same-device, free destination → one `rename`, no bytes copied.
    fn try_rename(&mut self, top: &Top) -> bool {
        let (Ok(src_vfs), Ok(dst_vfs)) = (self.backends.vfs(&top.src), self.backends.vfs(&top.dst))
        else {
            return false;
        };
        let same_place = top.src.scheme() == top.dst.scheme()
            && top.src.authority() == top.dst.authority()
            && src_vfs.same_device(&top.src, &top.dst);
        if !same_place || !matches!(dst_vfs.stat(&top.dst), Err(FmError::NotFound(_))) {
            return false;
        }
        if src_vfs.rename(&top.src, &top.dst).is_err() {
            return false;
        }
        let count = (top.end - top.start) as u64;
        let bytes: u64 = self
            .plan
            .items
            .get(top.start..top.end)
            .map_or(0, |s| s.iter().map(|i| i.size).sum());
        self.progress.items.done += count;
        self.progress.bytes.done += bytes;
        self.progress.current = top.dst.file_name().unwrap_or_default().to_string();
        self.report.done += count;
        self.report.moved.push((top.src.clone(), top.dst.clone()));
        self.emit();
        true
    }

    fn transfer_top(&mut self, top: &Top, moving: bool) {
        let Some(items) = self.plan.items.get(top.start..top.end) else {
            return;
        };
        self.roots = Some((
            top.src.path_str().to_string(),
            top.dst.path_str().to_string(),
        ));
        let mut failed_dirs: Vec<&VfsUri> = Vec::new();
        let mut done_idx: Vec<usize> = Vec::new();
        let mut clean = true;
        for (i, item) in items.iter().enumerate() {
            if self.ctx.cancel.is_cancelled() {
                self.report.cancelled = true;
                return;
            }
            self.progress.current = item.dst.file_name().unwrap_or_default().to_string();
            if failed_dirs
                .iter()
                .any(|d| d.is_same_or_descendant_of(&item.dst))
            {
                self.progress.items.done += 1;
                clean = false;
                continue;
            }
            match self.transfer_item(item) {
                Outcome::Done { created } => {
                    self.report.done += 1;
                    done_idx.push(i);
                    if i == 0 {
                        self.report.created.extend(created);
                    }
                }
                Outcome::Skipped => {
                    self.report.skipped += 1;
                    clean = false;
                }
                Outcome::Failed(err) => {
                    if item.kind == PlanKind::Dir {
                        failed_dirs.push(&item.dst);
                    }
                    self.report.failed.push((item.src.clone(), err));
                    clean = false;
                }
                Outcome::Cancelled => {
                    self.report.cancelled = true;
                    return;
                }
            }
            self.progress.items.done += 1;
            self.emit();
        }
        if moving {
            let removed = self.delete_sources(items, &done_idx);
            if clean && removed {
                self.report.moved.push((top.src.clone(), top.dst.clone()));
            }
        }
    }

    fn transfer_item(&mut self, item: &PlanItem) -> Outcome {
        match item.kind {
            PlanKind::Dir => self.ensure_dir(item),
            PlanKind::File => self.copy_file(item),
            PlanKind::Symlink => self.copy_symlink(item),
        }
    }

    fn ensure_dir(&mut self, item: &PlanItem) -> Outcome {
        let vfs = match self.backends.vfs(&item.dst) {
            Ok(v) => v,
            Err(e) => return Outcome::Failed(e),
        };
        match vfs.stat(&item.dst) {
            Ok(existing) if existing.is_dir() => Outcome::Done { created: None },
            Ok(_) => Outcome::Failed(FmError::AlreadyExists(item.dst.to_string())),
            Err(FmError::NotFound(_)) => match vfs.create_dir(&item.dst) {
                Ok(()) => Outcome::Done {
                    created: Some(item.dst.clone()),
                },
                Err(e) => Outcome::Failed(e),
            },
            Err(e) => Outcome::Failed(e),
        }
    }

    /// Recreate a source symlink as a link (never followed). The target text is copied
    /// verbatim unless retargeting is on and it points inside the copied tree.
    fn copy_symlink(&mut self, item: &PlanItem) -> Outcome {
        let (src_vfs, dst_vfs) = match (self.backends.vfs(&item.src), self.backends.vfs(&item.dst))
        {
            (Ok(s), Ok(d)) => (s, d),
            (Err(e), _) | (_, Err(e)) => return Outcome::Failed(e),
        };
        let src_entry = match src_vfs.stat(&item.src) {
            Ok(e) => e,
            Err(e) => return Outcome::Failed(e),
        };
        let EntryKind::Symlink { target } = &src_entry.kind else {
            return Outcome::Failed(FmError::InvalidParams(
                "source is no longer a symbolic link".to_string(),
            ));
        };
        let mut target = target.clone();
        let both_local = item.src.scheme() == Scheme::File && item.dst.scheme() == Scheme::File;
        if self.ctx.retarget_links
            && both_local
            && let Some((from, to)) = &self.roots
            && let Some(moved) = retarget(&target, from, to)
        {
            target = moved;
        }
        let mut dst = item.dst.clone();
        let mut overwrite = false;
        match dst_vfs.stat(&dst) {
            Ok(existing) => match self.decide(item, &existing, &src_entry) {
                Choice::Skip => return Outcome::Skipped,
                Choice::Cancel => return Outcome::Cancelled,
                Choice::Overwrite if existing.is_dir() => {
                    return Outcome::Failed(FmError::AlreadyExists(dst.to_string()));
                }
                Choice::Overwrite => overwrite = true,
                Choice::KeepBoth => {
                    let Some(parent) = dst.parent() else {
                        return Outcome::Failed(FmError::NotADirectory(dst.to_string()));
                    };
                    let name = dst.file_name().unwrap_or_default().to_string();
                    dst = match keep_both_name(&*dst_vfs, &parent, &name) {
                        Ok(d) => d,
                        Err(e) => return Outcome::Failed(e),
                    };
                }
            },
            Err(FmError::NotFound(_)) => {}
            Err(e) => return Outcome::Failed(e),
        }
        if overwrite && let Err(e) = dst_vfs.remove(&dst) {
            return Outcome::Failed(e);
        }
        match dst_vfs.create_symlink(&target, &dst) {
            Ok(()) => Outcome::Done {
                created: (!overwrite).then_some(dst),
            },
            Err(e) => Outcome::Failed(e),
        }
    }

    fn decide(&mut self, item: &PlanItem, existing: &Entry, src: &Entry) -> Choice {
        if let Some(choice) = self.sticky {
            return choice;
        }
        match self.ctx.policy {
            ConflictPolicy::Overwrite => Choice::Overwrite,
            ConflictPolicy::KeepBoth => Choice::KeepBoth,
            ConflictPolicy::Skip => Choice::Skip,
            ConflictPolicy::Ask => {
                let conflict = Conflict {
                    incoming: ConflictSide {
                        uri: item.src.clone(),
                        size: src.size,
                        modified: src.modified,
                    },
                    existing: ConflictSide {
                        uri: existing.uri.clone(),
                        size: existing.size,
                        modified: existing.modified,
                    },
                };
                let decision = (self.ctx.on_conflict)(&conflict);
                if decision.apply_to_all {
                    self.sticky = Some(decision.choice);
                }
                decision.choice
            }
        }
    }

    fn copy_file(&mut self, item: &PlanItem) -> Outcome {
        let (src_vfs, dst_vfs) = match (self.backends.vfs(&item.src), self.backends.vfs(&item.dst))
        {
            (Ok(s), Ok(d)) => (s, d),
            (Err(e), _) | (_, Err(e)) => return Outcome::Failed(e),
        };
        let mut dst = item.dst.clone();
        let mut overwrite = false;
        match dst_vfs.stat(&dst) {
            Ok(existing) if existing.is_file() => {
                let src_entry = match src_vfs.stat(&item.src) {
                    Ok(e) => e,
                    Err(e) => return Outcome::Failed(e),
                };
                match self.decide(item, &existing, &src_entry) {
                    Choice::Skip => {
                        self.progress.bytes.done += item.size;
                        return Outcome::Skipped;
                    }
                    Choice::Cancel => return Outcome::Cancelled,
                    Choice::Overwrite => overwrite = true,
                    Choice::KeepBoth => {
                        let Some(parent) = dst.parent() else {
                            return Outcome::Failed(FmError::NotADirectory(dst.to_string()));
                        };
                        let name = dst.file_name().unwrap_or_default().to_string();
                        dst = match keep_both_name(&*dst_vfs, &parent, &name) {
                            Ok(d) => d,
                            Err(e) => return Outcome::Failed(e),
                        };
                    }
                }
            }
            // Directories, symlinks and special files are never written through.
            Ok(_) => return Outcome::Failed(FmError::AlreadyExists(dst.to_string())),
            Err(FmError::NotFound(_)) => {}
            Err(e) => return Outcome::Failed(e),
        }
        match self.write_file(&*src_vfs, &*dst_vfs, &item.src, &dst, overwrite) {
            Ok(()) => Outcome::Done {
                created: (!overwrite).then_some(dst),
            },
            Err(FmError::Cancelled) => Outcome::Cancelled,
            Err(e) => Outcome::Failed(e),
        }
    }

    fn write_file(
        &mut self,
        src_vfs: &dyn Vfs,
        dst_vfs: &dyn Vfs,
        src: &VfsUri,
        dst: &VfsUri,
        overwrite: bool,
    ) -> Result<(), FmError> {
        let parent = dst
            .parent()
            .ok_or_else(|| FmError::NotADirectory(dst.to_string()))?;
        if !self.direct_write {
            let tmp = parent.join(&temp_name(".fmpart"))?;
            let before = self.progress.bytes.done;
            let streamed = self.stream(src_vfs, dst_vfs, src, &tmp);
            let committed = streamed.and_then(|()| commit(dst_vfs, &tmp, dst, &parent, overwrite));
            match committed {
                Ok(()) => return Ok(()),
                Err(FmError::Unsupported(_)) => {
                    // No rename on this backend: discard the temp and write in place.
                    let _ = dst_vfs.remove(&tmp);
                    self.progress.bytes.done = before;
                    self.direct_write = true;
                }
                Err(e) => {
                    let _ = dst_vfs.remove(&tmp);
                    return Err(e);
                }
            }
        }
        let result = self.stream(src_vfs, dst_vfs, src, dst);
        if result.is_err() {
            let _ = dst_vfs.remove(dst);
        }
        result
    }

    fn stream(
        &mut self,
        src_vfs: &dyn Vfs,
        dst_vfs: &dyn Vfs,
        src: &VfsUri,
        to: &VfsUri,
    ) -> Result<(), FmError> {
        let mut reader = src_vfs.open_read(src)?;
        let mut writer = dst_vfs.open_write(to, WriteOpts::default())?;
        let mut buf = vec![0u8; BUF_BYTES];
        loop {
            if self.ctx.cancel.is_cancelled() {
                return Err(FmError::Cancelled);
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let chunk = buf.get(..n).ok_or(FmError::Unsupported("short buffer"))?;
            writer.write_all(chunk)?;
            self.progress.bytes.done += n as u64;
            self.emit();
        }
        writer.flush()?;
        Ok(())
    }

    /// Remove verified sources: files first, then directories bottom-up.
    /// Returns true when the whole top-level source is gone.
    fn delete_sources(&mut self, items: &[PlanItem], done_idx: &[usize]) -> bool {
        let mut all_gone = done_idx.len() == items.len();
        let mut ordered: Vec<&PlanItem> = done_idx.iter().filter_map(|&i| items.get(i)).collect();
        ordered.reverse();
        let (dirs, files): (Vec<_>, Vec<_>) =
            ordered.into_iter().partition(|i| i.kind == PlanKind::Dir);
        for item in files.into_iter().chain(dirs) {
            if self.ctx.cancel.is_cancelled() {
                return false;
            }
            match self.remove_verified(item) {
                Ok(()) | Err(FmError::NotEmpty(_)) | Err(FmError::NotFound(_)) => {}
                Err(e) => {
                    all_gone = false;
                    self.report.failed.push((item.src.clone(), e));
                }
            }
        }
        all_gone
    }

    fn remove_verified(&self, item: &PlanItem) -> Result<(), FmError> {
        let src_vfs = self.backends.vfs(&item.src)?;
        if item.kind == PlanKind::File {
            let dst_vfs = self.backends.vfs(&item.dst)?;
            let copied = dst_vfs.stat(&item.dst)?;
            let original = src_vfs.stat(&item.src)?;
            if !copied.is_file() || copied.size.is_none() || copied.size != original.size {
                return Err(FmError::backend(
                    "ops",
                    format!("verification failed for {}; source kept", item.src),
                ));
            }
        }
        src_vfs.remove(&item.src)
    }
}

fn temp_name(prefix: &str) -> String {
    let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{n}", std::process::id())
}

/// Move the finished temp into place. Overwrites keep the old file as a backup until the
/// new one is in position, so a failed commit never loses the destination.
fn commit(
    vfs: &dyn Vfs,
    tmp: &VfsUri,
    dst: &VfsUri,
    parent: &VfsUri,
    overwrite: bool,
) -> Result<(), FmError> {
    if !overwrite {
        return vfs.rename(tmp, dst);
    }
    let backup = parent.join(&temp_name(".fmbak"))?;
    vfs.rename(dst, &backup)?;
    if let Err(e) = vfs.rename(tmp, dst) {
        let _ = vfs.rename(&backup, dst);
        return Err(e);
    }
    let _ = vfs.remove(&backup);
    Ok(())
}
