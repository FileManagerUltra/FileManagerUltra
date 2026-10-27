//! The [`Vfs`] trait — the only door to a filesystem.
//!
//! Every method is **blocking** by contract (the `fm-engine` job lanes own the
//! threads; the UI thread never calls these directly). Implementations must be
//! `Send + Sync` so a `Vfs` can be shared across lanes.

use std::io::{Read, Write};

use crate::entry::Entry;
use crate::error::FmError;
use crate::uri::VfsUri;

/// Options for [`Vfs::open_write`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOpts {
    /// Create the file if missing.
    pub create: bool,
    /// Truncate to zero length first.
    pub truncate: bool,
    /// Append instead of overwriting.
    pub append: bool,
}

impl Default for WriteOpts {
    fn default() -> Self {
        Self {
            create: true,
            truncate: true,
            append: false,
        }
    }
}

/// Free space report for a volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpaceInfo {
    pub total: u64,
    pub available: u64,
}

/// One backend (local disk, SMB, SFTP, memory). No I/O happens outside an impl.
pub trait Vfs: Send + Sync {
    /// Which scheme this backend serves (exactly one).
    fn scheme(&self) -> crate::uri::Scheme;

    /// Children of a directory, unsorted; backends may report them in any order.
    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError>;

    /// Metadata for one path.
    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError>;

    /// Open for reading (whole file for v1; streaming lands with copy pipelines).
    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError>;

    /// Open for reading starting at byte `offset`. The default skips by reading;
    /// backends that can seek override it so tail/range reads of huge files are cheap.
    fn open_read_at(&self, uri: &VfsUri, offset: u64) -> Result<Box<dyn Read + Send>, FmError> {
        let mut r = self.open_read(uri)?;
        std::io::copy(&mut (&mut r).take(offset), &mut std::io::sink())?;
        Ok(r)
    }

    /// Open for writing. `flush` commits; dropping without flush is best-effort.
    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError>;

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError>;

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError>;

    /// Create a symbolic link at `link` holding `target` verbatim (never resolved or
    /// followed). Backends that cannot represent links keep this default.
    fn create_symlink(&self, target: &str, link: &VfsUri) -> Result<(), FmError> {
        let _ = (target, link);
        Err(FmError::Unsupported(
            "this backend cannot create symbolic links",
        ))
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError>;

    /// Set the Unix permission bits (`mode & 0o7777`) of one item, never following a
    /// final symlink further than the platform does. Backends without Unix modes keep this
    /// default.
    fn set_permissions(&self, uri: &VfsUri, mode: u32) -> Result<(), FmError> {
        let _ = (uri, mode);
        Err(FmError::Unsupported(
            "this location does not support Unix permissions",
        ))
    }

    /// Are both paths on the same volume? Drives move/copy decisions.
    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool;

    /// Disk usage of a volume, if the backend can tell.
    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError>;
}
