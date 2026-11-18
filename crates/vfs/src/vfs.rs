//! The [`Vfs`] trait — the only door to a filesystem.
//!
//! Every method is **blocking** by contract (the `fm-engine` job lanes own the
//! threads; the UI thread never calls these directly). Implementations must be
//! `Send + Sync` so a `Vfs` can be shared across lanes.

use std::io::{Read, Write};

use crate::attrs::{AttrCaps, AttrReport, AttrSet, Attrs};
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

    /// True only for the real local-disk backend, whose `file://` paths may also be read
    /// with `std::fs` directly (fast walks). Test and remote backends keep the default.
    fn is_native_local(&self) -> bool {
        false
    }

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

    /// Every attribute the backend can read for one item (a final symlink is not followed).
    /// Backends without attribute support keep this default.
    fn stat_full(&self, uri: &VfsUri) -> Result<Attrs, FmError> {
        let _ = uri;
        Err(FmError::Unsupported(
            "this location does not expose full attributes",
        ))
    }

    /// Apply the attribute groups in `which` that `attrs` carries, best effort: one failing
    /// group never stops the others (see [`AttrReport`]). A final symlink is not followed.
    fn set_attrs(
        &self,
        uri: &VfsUri,
        attrs: &Attrs,
        which: AttrSet,
    ) -> Result<AttrReport, FmError> {
        let _ = (uri, attrs, which);
        Err(FmError::Unsupported("this location cannot set attributes"))
    }

    /// What this backend can read and write at `uri`; nothing by default.
    fn attr_caps(&self, uri: &VfsUri) -> AttrCaps {
        let _ = uri;
        AttrCaps::NONE
    }

    /// Are both paths on the same volume? Drives move/copy decisions.
    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool;

    /// Disk usage of a volume, if the backend can tell.
    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uri::{Scheme, VfsUri};

    struct Bare;

    impl Vfs for Bare {
        fn scheme(&self) -> Scheme {
            Scheme::File
        }
        fn read_dir(&self, _: &VfsUri) -> Result<Vec<Entry>, FmError> {
            Ok(vec![])
        }
        fn stat(&self, u: &VfsUri) -> Result<Entry, FmError> {
            Err(FmError::NotFound(u.to_string()))
        }
        fn open_read(&self, u: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
            Err(FmError::NotFound(u.to_string()))
        }
        fn open_write(&self, u: &VfsUri, _: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
            Err(FmError::NotFound(u.to_string()))
        }
        fn create_dir(&self, _: &VfsUri) -> Result<(), FmError> {
            Ok(())
        }
        fn remove(&self, _: &VfsUri) -> Result<(), FmError> {
            Ok(())
        }
        fn rename(&self, _: &VfsUri, _: &VfsUri) -> Result<(), FmError> {
            Ok(())
        }
        fn same_device(&self, _: &VfsUri, _: &VfsUri) -> bool {
            true
        }
        fn space(&self, _: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
            Ok(None)
        }
    }

    #[test]
    fn a_backend_without_attribute_support_reports_unsupported() {
        let uri = VfsUri::root_file();
        assert!(matches!(Bare.stat_full(&uri), Err(FmError::Unsupported(_))));
        assert!(matches!(
            Bare.set_attrs(&uri, &Attrs::default(), AttrSet::ALL),
            Err(FmError::Unsupported(_))
        ));
        assert_eq!(Bare.attr_caps(&uri), AttrCaps::NONE);
    }
}
