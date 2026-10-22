//! File operation pipelines: copy, move, delete, mkdir, rename — with conflict policy,
//! progress reporting, cancellation, trash integration and the symlink-escape rules from
//! the `security` capability. Behaviour lands in change 3; the types and the symlink
//! contract are fixed now.

pub mod symlink;

use fm_vfs::{FmError, VfsUri};

pub use symlink::SymlinkPolicy;

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

/// Copy entry point — implemented in change 3.
pub fn copy(_from: &VfsUri, _to: &VfsUri, _policy: ConflictPolicy) -> Result<(), FmError> {
    Err(FmError::Unsupported("ops::copy (change 3)"))
}

/// Move entry point — implemented in change 3.
pub fn move_(_from: &VfsUri, _to: &VfsUri, _policy: ConflictPolicy) -> Result<(), FmError> {
    Err(FmError::Unsupported("ops::move_ (change 3)"))
}

/// Delete entry point (must go through `trash` in the real pipeline) — change 3.
pub fn delete(_uri: &VfsUri) -> Result<(), FmError> {
    Err(FmError::Unsupported("ops::delete (change 3)"))
}

/// mkdir entry point — change 3.
pub fn mkdir(_uri: &VfsUri) -> Result<(), FmError> {
    Err(FmError::Unsupported("ops::mkdir (change 3)"))
}

/// In-place rename entry point — change 3.
pub fn rename(_from: &VfsUri, _to: &VfsUri) -> Result<(), FmError> {
    Err(FmError::Unsupported("ops::rename (change 3)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_are_unsupported_not_panics() {
        let a = VfsUri::parse("/a").expect("uri");
        let b = VfsUri::parse("/b").expect("uri");
        for result in [
            copy(&a, &b, ConflictPolicy::Ask),
            move_(&a, &b, ConflictPolicy::Overwrite),
            delete(&a),
            mkdir(&a),
            rename(&a, &b),
        ] {
            assert!(matches!(result, Err(FmError::Unsupported(_))));
        }
    }

    #[test]
    fn progress_fraction_is_total_safe() {
        let p = TransferProgress { done: 1, total: 0 };
        assert_eq!(p.fraction(), None);
        let p = TransferProgress { done: 1, total: 4 };
        assert_eq!(p.fraction(), Some(0.25));
    }
}
