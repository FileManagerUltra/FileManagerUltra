//! SMB backend (change 4): `pavao`/libsmbclient on Linux and macOS, native UNC paths on
//! Windows. All SMB work runs on the engine's dedicated `smb` lane — `pavao` serializes
//! every call process-wide (see `docs/architecture.md` §4.4 and §7).

use fm_vfs::{Entry, FmError, Scheme, Vfs, VfsUri, WriteOpts};

/// The SMB backend. `scheme()` is the only working method until change 4.
#[derive(Debug, Default, Clone, Copy)]
pub struct SmbFs;

impl SmbFs {
    pub fn new() -> Self {
        Self
    }
}

impl Vfs for SmbFs {
    fn scheme(&self) -> Scheme {
        Scheme::Smb
    }

    fn read_dir(&self, _uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        Err(FmError::Unsupported("fs-smb::read_dir (change 4)"))
    }

    fn stat(&self, _uri: &VfsUri) -> Result<Entry, FmError> {
        Err(FmError::Unsupported("fs-smb::stat (change 4)"))
    }

    fn open_read(&self, _uri: &VfsUri) -> Result<Box<dyn std::io::Read + Send>, FmError> {
        Err(FmError::Unsupported("fs-smb::open_read (change 4)"))
    }

    fn open_write(
        &self,
        _uri: &VfsUri,
        _opts: WriteOpts,
    ) -> Result<Box<dyn std::io::Write + Send>, FmError> {
        Err(FmError::Unsupported("fs-smb::open_write (change 4)"))
    }

    fn create_dir(&self, _uri: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-smb::create_dir (change 4)"))
    }

    fn remove(&self, _uri: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-smb::remove (change 4)"))
    }

    fn rename(&self, _from: &VfsUri, _to: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-smb::rename (change 4)"))
    }

    fn same_device(&self, _a: &VfsUri, _b: &VfsUri) -> bool {
        false
    }

    fn space(&self, _uri: &VfsUri) -> Result<Option<fm_vfs::SpaceInfo>, FmError> {
        Err(FmError::Unsupported("fs-smb::space (change 4)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_is_smb_and_ops_are_unsupported() {
        let fs = SmbFs::new();
        assert_eq!(fs.scheme(), Scheme::Smb);
        let uri = VfsUri::parse("smb://nas/media").expect("static uri");
        assert!(matches!(fs.read_dir(&uri), Err(FmError::Unsupported(_))));
    }
}
