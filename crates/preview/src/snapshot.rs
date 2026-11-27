//! Live preview input (change `add-editor-folding-live-preview`): one file's unsaved
//! text, served as a read-only, single-file [`Vfs`] so every renderer works on it as on
//! the file itself. The file keeps its real uri, so links and images resolve against
//! its folder through the session's own backends.

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use fm_vfs::{Entry, EntryKind, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

/// The text of an edited file at one version.
pub struct SnapshotVfs {
    uri: VfsUri,
    bytes: Arc<[u8]>,
}

impl SnapshotVfs {
    pub fn new(uri: VfsUri, text: &str) -> Self {
        Self {
            uri,
            bytes: Arc::from(text.as_bytes()),
        }
    }

    /// The entry renderers are given.
    pub fn entry(&self) -> Entry {
        let mut e = Entry::new(self.uri.clone(), EntryKind::File);
        e.size = Some(self.bytes.len() as u64);
        e
    }

    fn check(&self, uri: &VfsUri) -> Result<(), FmError> {
        if *uri == self.uri {
            Ok(())
        } else {
            Err(FmError::NotFound(uri.to_string()))
        }
    }
}

const READ_ONLY: &str = "a live preview is read-only";

impl Vfs for SnapshotVfs {
    fn scheme(&self) -> Scheme {
        self.uri.scheme()
    }

    fn read_dir(&self, _: &VfsUri) -> Result<Vec<Entry>, FmError> {
        Err(FmError::Unsupported(READ_ONLY))
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        self.check(uri)?;
        Ok(self.entry())
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        self.check(uri)?;
        Ok(Box::new(Cursor::new(ArcBytes(self.bytes.clone()))))
    }

    fn open_read_at(&self, uri: &VfsUri, offset: u64) -> Result<Box<dyn Read + Send>, FmError> {
        self.check(uri)?;
        let mut c = Cursor::new(ArcBytes(self.bytes.clone()));
        c.set_position(offset.min(self.bytes.len() as u64));
        Ok(Box::new(c))
    }

    fn open_write(&self, _: &VfsUri, _: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        Err(FmError::Unsupported(READ_ONLY))
    }

    fn create_dir(&self, _: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported(READ_ONLY))
    }

    fn remove(&self, _: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported(READ_ONLY))
    }

    fn rename(&self, _: &VfsUri, _: &VfsUri) -> Result<(), FmError> {
        Err(FmError::Unsupported(READ_ONLY))
    }

    fn same_device(&self, _: &VfsUri, _: &VfsUri) -> bool {
        false
    }

    fn space(&self, _: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        Ok(None)
    }
}

struct ArcBytes(Arc<[u8]>);

impl AsRef<[u8]> for ArcBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn serves_only_its_own_text() {
        let uri = VfsUri::parse("/notes/a.md").expect("uri");
        let vfs = SnapshotVfs::new(uri.clone(), "# Title\n");
        let mut text = String::new();
        vfs.open_read(&uri)
            .expect("read")
            .read_to_string(&mut text)
            .expect("text");
        assert_eq!(text, "# Title\n");
        assert_eq!(vfs.stat(&uri).expect("stat").size, Some(8));
        let other = VfsUri::parse("/notes/b.md").expect("uri");
        assert!(matches!(vfs.stat(&other), Err(FmError::NotFound(_))));
        assert!(vfs.open_write(&uri, WriteOpts::default()).is_err());
    }

    #[test]
    fn markdown_renders_from_the_snapshot() {
        let uri = VfsUri::parse("/notes/a.md").expect("uri");
        let vfs = SnapshotVfs::new(uri, "# Live\n\nchanged text\n");
        let preview = crate::render(&vfs, &vfs.entry(), &crate::Limits::default());
        assert!(
            matches!(preview, crate::Preview::Markdown(_)),
            "{preview:?}"
        );
    }
}
