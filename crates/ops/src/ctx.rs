//! Execution context shared by every pipeline: cancellation, progress, conflict
//! decisions and the backend resolver. `ops` stays synchronous and runtime-free — the
//! engine's job worker supplies the callbacks (see design D1/D3).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use fm_vfs::{FmError, Vfs, VfsUri};

use crate::{ConflictPolicy, TransferProgress};

/// Maps a location to the backend that serves it. Closures work too.
pub trait Backends {
    fn vfs(&self, uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError>;
}

impl<F> Backends for F
where
    F: Fn(&VfsUri) -> Result<Arc<dyn Vfs>, FmError>,
{
    fn vfs(&self, uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        self(uri)
    }
}

/// One backend serving every uri (tests, single-backend sessions).
pub struct SingleBackend(pub Arc<dyn Vfs>);

impl Backends for SingleBackend {
    fn vfs(&self, _uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        Ok(self.0.clone())
    }
}

/// Snapshot of a running transfer, published after every buffer and every item.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    pub bytes: TransferProgress,
    pub items: TransferProgress,
    /// Display name of the item being processed.
    pub current: String,
}

/// One side of a conflict, shown in the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSide {
    pub uri: VfsUri,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// A destination that already exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub incoming: ConflictSide,
    pub existing: ConflictSide,
}

/// The user's answer to a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Overwrite,
    KeepBoth,
    Skip,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub choice: Choice,
    /// Reuse this choice for every remaining conflict of the job.
    pub apply_to_all: bool,
}

/// What a job did. Item errors never abort the job; only cancel does.
#[derive(Debug, Default)]
pub struct OpReport {
    /// Items written (files and directories created or merged).
    pub done: u64,
    pub skipped: u64,
    pub failed: Vec<(VfsUri, FmError)>,
    pub cancelled: bool,
    /// Top-level destinations that now exist because of this job (undo input).
    pub created: Vec<VfsUri>,
    /// Top-level `(from, to)` pairs completed by a move (undo input).
    pub moved: Vec<(VfsUri, VfsUri)>,
    /// Originals now in the platform trash (undo input).
    pub trashed: Vec<VfsUri>,
}

impl OpReport {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty() && !self.cancelled
    }
}

/// Cooperative cancel flag shared with the engine.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Callbacks and state for one job.
pub struct OpCtx<'a> {
    pub cancel: CancelFlag,
    pub policy: ConflictPolicy,
    pub on_progress: &'a mut dyn FnMut(&Progress),
    pub on_conflict: &'a mut dyn FnMut(&Conflict) -> Decision,
}

impl<'a> OpCtx<'a> {
    pub fn new(
        policy: ConflictPolicy,
        on_progress: &'a mut dyn FnMut(&Progress),
        on_conflict: &'a mut dyn FnMut(&Conflict) -> Decision,
    ) -> Self {
        Self {
            cancel: CancelFlag::new(),
            policy,
            on_progress,
            on_conflict,
        }
    }

    pub fn with_cancel(mut self, cancel: CancelFlag) -> Self {
        self.cancel = cancel;
        self
    }
}
