//! [`Entry`] — one row in a directory listing.

use std::time::SystemTime;

use crate::uri::VfsUri;

/// What a directory entry *is*.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntryKind {
    File,
    Dir,
    /// Symlink; `target` is the raw target as read from the backend (may be
    /// relative, may dangle — resolution is the caller's problem).
    Symlink {
        target: String,
    },
    /// Anything else (socket, fifo, device…): shown, never opened blindly.
    Other,
}

/// One listing row. Backends fill in what they know; `None` means "unknown", never 0.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub uri: VfsUri,
    /// Display name (last path segment as the backend reports it).
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Unix permission bits (mode & 0o7777). `None` on Windows / unknown.
    pub permissions: Option<u32>,
}

impl Entry {
    pub fn new(uri: VfsUri, kind: EntryKind) -> Self {
        let name = uri.file_name().unwrap_or("/").to_string();
        Self {
            uri,
            name,
            kind,
            size: None,
            modified: None,
            permissions: None,
        }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.kind, EntryKind::Dir)
    }

    pub fn is_file(&self) -> bool {
        matches!(self.kind, EntryKind::File)
    }

    pub fn is_symlink(&self) -> bool {
        matches!(self.kind, EntryKind::Symlink { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_derives_name_from_uri() {
        let uri = VfsUri::parse("/home/alice/Docs").unwrap();
        let entry = Entry::new(uri, EntryKind::Dir);
        assert_eq!(entry.name, "Docs");
        assert!(entry.is_dir());
        assert!(!entry.is_file());
        assert!(entry.size.is_none());
    }

    #[test]
    fn root_entry_name_is_slash() {
        let entry = Entry::new(VfsUri::parse("/").unwrap(), EntryKind::Dir);
        assert_eq!(entry.name, "/");
    }
}
