//! SSH/SFTP backend (change 4): `russh` + `russh-sftp`, one tokio runtime owned by this
//! crate, bridged to the blocking [`Vfs`](fm_vfs::Vfs) contract with `Handle::block_on`.

use fm_vfs::{Entry, FmError, Scheme, Vfs, VfsUri, WriteOpts};

/// The SFTP backend. `scheme()` is the only working method until change 4.
#[derive(Debug, Default, Clone, Copy)]
pub struct SftpFs;

impl SftpFs {
    pub fn new() -> Self {
        Self
    }
}

impl Vfs for SftpFs {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn read_dir(&self, _uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        Err(FmError::Unsupported("fs-sftp::read_dir (change 4)"))
    }

    fn stat(&self, _uri: &VfsUri) -> Result<Entry, FmError> {
        Err(FmError::Unsupported("fs-sftp::stat (change 4)"))
    }

    fn open_read(&self, _uri: &VfsUri) -> Result<Box<dyn std::io::Read + Send>, FmError> {
        Err(FmError::Unsupported("fs-sftp::open_read (change 4)"))
    }

    fn open_write(
        &self,
        _uri: &VfsUri,
        _opts: WriteOpts,
    ) -> Result<Box<dyn std::io::Write + Send>, FmError> {
        Err(FmError::Unsupported("fs-sftp::open_write (change 4)"))
    }

    fn create_dir(&self, _uri: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-sftp::create_dir (change 4)"))
    }

    fn remove(&self, _uri: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-sftp::remove (change 4)"))
    }

    fn rename(&self, _from: &VfsUri, _to: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported("fs-sftp::rename (change 4)"))
    }

    fn same_device(&self, _a: &VfsUri, _b: &VfsUri) -> bool {
        false
    }

    fn space(&self, _uri: &VfsUri) -> Result<Option<fm_vfs::SpaceInfo>, FmError> {
        Err(FmError::Unsupported("fs-sftp::space (change 4)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_is_sftp_and_ops_are_unsupported() {
        let fs = SftpFs::new();
        assert_eq!(fs.scheme(), Scheme::Sftp);
        let uri = VfsUri::parse("sftp://alice@example.com/srv").expect("static uri");
        assert!(matches!(fs.read_dir(&uri), Err(FmError::Unsupported(_))));
    }
}
