//! File operation pipelines: copy, move, delete, mkdir, rename — with conflict policy,
//! progress reporting, cancellation, trash integration and the symlink-escape rules from
//! the `security` capability. Everything is synchronous and runtime-free: the engine's
//! job worker supplies cancellation, progress and conflict callbacks through [`OpCtx`].

pub mod ctx;
pub mod delete;
pub mod names;
pub mod plan;
pub mod symlink;
pub mod transfer;
pub mod trash;
pub mod undo;

use std::io::Write;

use fm_vfs::{FmError, VfsUri, WriteOpts};

pub use ctx::{
    Backends, CancelFlag, Choice, Conflict, ConflictSide, Decision, OpCtx, OpReport, Progress,
    SingleBackend,
};
pub use delete::{delete_permanent, trash_items};
pub use names::validate_name;
pub use plan::{Plan, PlanLimits, plan_transfer};
pub use symlink::SymlinkPolicy;
pub use undo::{UndoRecord, UndoStack};

/// What to do when the destination already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Ask the user (UI turns this into a dialog; the pipeline itself never blocks on UI).
    #[default]
    Ask,
    /// Overwrite the destination.
    Overwrite,
    /// Keep both: rename the incoming item (`name (2).ext`).
    KeepBoth,
    /// Skip the conflicting item and continue.
    Skip,
}

/// Which pipeline an operation belongs to (also drives the job lane choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpKind {
    Copy,
    Move,
    Delete,
    Mkdir,
    NewFile,
    Rename,
}

/// Progress of a transfer; jobs publish these through the engine's job queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferProgress {
    /// Bytes/items finished.
    pub done: u64,
    /// Total bytes/items, when known.
    pub total: u64,
}

impl TransferProgress {
    pub fn fraction(&self) -> Option<f64> {
        if self.total == 0 {
            None
        } else {
            Some(self.done as f64 / self.total as f64)
        }
    }
}

/// Plan and copy `sources` into the directory `dest_dir`.
pub fn copy(
    backends: &dyn Backends,
    sources: &[VfsUri],
    dest_dir: &VfsUri,
    ctx: &mut OpCtx<'_>,
) -> Result<OpReport, FmError> {
    let plan = plan_transfer(backends, sources, dest_dir, PlanLimits::default())?;
    Ok(transfer::run_copy(backends, &plan, ctx))
}

/// Plan and move `sources` into the directory `dest_dir`.
pub fn move_(
    backends: &dyn Backends,
    sources: &[VfsUri],
    dest_dir: &VfsUri,
    ctx: &mut OpCtx<'_>,
) -> Result<OpReport, FmError> {
    let plan = plan_transfer(backends, sources, dest_dir, PlanLimits::default())?;
    Ok(transfer::run_move(backends, &plan, ctx))
}

/// Create the directory `name` inside `parent`.
pub fn mkdir(backends: &dyn Backends, parent: &VfsUri, name: &str) -> Result<VfsUri, FmError> {
    validate_name(name)?;
    let target = parent.join(name)?;
    let vfs = backends.vfs(&target)?;
    match vfs.stat(&target) {
        Ok(_) => return Err(FmError::AlreadyExists(target.to_string())),
        Err(FmError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    vfs.create_dir(&target)?;
    Ok(target)
}

/// Create an empty file `name` inside `parent` (like `touch`, but never touches an existing entry).
pub fn create_file(
    backends: &dyn Backends,
    parent: &VfsUri,
    name: &str,
) -> Result<VfsUri, FmError> {
    validate_name(name)?;
    let target = parent.join(name)?;
    let vfs = backends.vfs(&target)?;
    match vfs.stat(&target) {
        Ok(_) => return Err(FmError::AlreadyExists(target.to_string())),
        Err(FmError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    let opts = WriteOpts {
        create: true,
        truncate: false,
        append: false,
    };
    let mut writer = vfs.open_write(&target, opts)?;
    writer.flush().map_err(FmError::from)?;
    Ok(target)
}

/// Rename `uri` to `new_name` within its directory; never overwrites.
pub fn rename(backends: &dyn Backends, uri: &VfsUri, new_name: &str) -> Result<VfsUri, FmError> {
    validate_name(new_name)?;
    let parent = uri
        .parent()
        .ok_or_else(|| FmError::InvalidParams("cannot rename a filesystem root".into()))?;
    let target = parent.join(new_name)?;
    if target == *uri {
        return Ok(target);
    }
    let vfs = backends.vfs(uri)?;
    match vfs.stat(&target) {
        Ok(_) => return Err(FmError::AlreadyExists(target.to_string())),
        Err(FmError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    vfs.rename(uri, &target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_fraction_is_total_safe() {
        let p = TransferProgress { done: 1, total: 0 };
        assert_eq!(p.fraction(), None);
        let p = TransferProgress { done: 1, total: 4 };
        assert_eq!(p.fraction(), Some(0.25));
    }
}
