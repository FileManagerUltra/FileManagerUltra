//! Virtual filesystem model — pure data, no I/O.
//!
//! Everything the product knows about paths and files lives here: [`VfsUri`] (the one
//! path type for local, SMB and SFTP locations), [`Entry`], the [`Vfs`] trait that
//! backends implement, and [`FmError`], the workspace error type (its `Display`
//! redacts credentials — see [`redact`]).

pub mod entry;
pub mod error;
pub mod redact;
pub mod uri;
pub mod vfs;

pub use entry::{Entry, EntryKind};
pub use error::FmError;
pub use uri::{Authority, Scheme, VfsUri};
pub use vfs::{SpaceInfo, Vfs, WriteOpts};

/// Hard caps for hostile input (capability `security`). A path, component or URI that
/// exceeds these is rejected up front instead of being allocated.
pub const MAX_PATH_BYTES: usize = 8192;
pub const MAX_COMPONENT_BYTES: usize = 4096;

/// Maximum entries a single `read_dir` may return (capability `security`).
/// Over the cap the read fails with an actionable error — never a silent truncate.
pub const MAX_DIR_ENTRIES: usize = 200_000;
