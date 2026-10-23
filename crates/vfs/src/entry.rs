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
    /// Platform hidden flag (Windows `HIDDEN` attribute, macOS `UF_HIDDEN`). Dot-names are
    /// hidden regardless; see the engine's `is_hidden`.
    pub hidden: bool,
    /// A symlink whose target resolves to a directory (always false for other kinds).
    #[serde(default)]
    pub link_to_dir: bool,
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
            hidden: false,
            link_to_dir: false,
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

    /// A directory, or a symlink that resolves to one: something the user can open.
    pub fn is_dir_like(&self) -> bool {
        self.is_dir() || (self.is_symlink() && self.link_to_dir)
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
