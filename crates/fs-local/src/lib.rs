//! Local filesystem backend: local drives and OS-mounted volumes (Linux, macOS, Windows).
//!
//! Behaviour lands in change 2 (multi-pane UI needs listings); the crate exists from the
//! scaffold so the layering table and dependency direction are real from day one.

use fm_vfs::{Entry, FmError, Scheme, Vfs, VfsUri};

/// The local-disk backend. `scheme()` is the only working method until change 2.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFs;

impl LocalFs {
    pub fn new() -> Self {
        Self
    }
}

impl Vfs for LocalFs {
    fn scheme(&self) -> Scheme {
        Scheme::File
    }

    fn read_dir(&self, _uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        Err(FmError::Unsupported("fs-local::read_dir (change 2)"))
    }

    fn stat(&self, _uri: &VfsUri) -> Result<Entry, FmError> {
        Err(FmError::Unsupported("fs-local::stat (change 2)"))
    }

    fn open_read(&self, _uri: &VfsUri) -> Result<Box<dyn std::io::Read + Send>, FmError> {
        Err(FmError::Unsupported("fs-local::open_read (change 2)"))
    }

    fn open_write(
        &self,
        _uri: &VfsUri,
        _opts: fm_vfs::WriteOpts,
    ) -> Result<Box<dyn std::io::Write + Send>, FmError> {
        Err(FmError::Unsupported("fs-local::open_write (change 2)"))
    }

    fn create_dir(&self, _uri: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-local::create_dir (change 2)"))
    }

    fn remove(&self, _uri: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-local::remove (change 2)"))
    }

    fn rename(&self, _from: &VfsUri, _to: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-local::rename (change 2)"))
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        // Conservative until real stat exists: never claim same-device.
        a == b
    }

    fn space(&self, _uri: &VfsUri) -> Result<Option<fm_vfs::SpaceInfo>, FmError> {
        Err(FmError::Unsupported("fs-local::space (change 2)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_is_file_and_ops_are_unsupported() {
        let fs = LocalFs::new();
        assert_eq!(fs.scheme(), Scheme::File);
        let uri = VfsUri::parse("/tmp").expect("static uri");
        let err = fs.read_dir(&uri).expect_err("stub must error");
        assert!(matches!(err, FmError::Unsupported(_)), "{err:?}");
        assert!(matches!(fs.stat(&uri), Err(FmError::Unsupported(_))));
        assert!(matches!(fs.space(&uri), Err(FmError::Unsupported(_))));
    }
}
