//! Platform trash (design D6). Local files only; a failure is an error, never a silent
//! downgrade to permanent deletion.

use std::path::{Path, PathBuf};

use fm_vfs::{FmError, Scheme, VfsUri};

/// Whether `restore_local` can work on this platform (the `trash` crate's
/// `os_limited` API does not exist on macOS, so undo-delete is unavailable there).
pub const RESTORE_SUPPORTED: bool = cfg!(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
));

fn local_path(uri: &VfsUri) -> Result<PathBuf, FmError> {
    if uri.scheme() != Scheme::File || !uri.authority().is_local() {
        return Err(FmError::Unsupported(
            "the trash is only available for local files",
        ));
    }
    uri.local_path().ok_or(FmError::Unsupported(
        "This PC lists the drives; it cannot be trashed",
    ))
}

fn backend(err: trash::Error) -> FmError {
    FmError::backend("trash", err.to_string())
}

/// Move one local item to the platform trash. Does not follow a symlink.
pub fn trash_local(uri: &VfsUri) -> Result<(), FmError> {
    let path = local_path(uri)?;
    if std::fs::symlink_metadata(&path).is_err() {
        return Err(FmError::NotFound(uri.to_string()));
    }
    trash::delete(&path).map_err(backend)
}

/// Put a previously trashed local item back where it was.
#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub fn restore_local(original: &VfsUri) -> Result<(), FmError> {
    let path = local_path(original)?;
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(FmError::AlreadyExists(original.to_string()));
    }
    let items = trash::os_limited::list().map_err(backend)?;
    let newest = items
        .into_iter()
        .filter(|item| item.original_path() == path)
        .max_by_key(|item| item.time_deleted)
        .ok_or_else(|| FmError::NotFound(format!("{original} is no longer in the trash")))?;
    trash::os_limited::restore_all([newest]).map_err(backend)
}

#[cfg(not(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
pub fn restore_local(_original: &VfsUri) -> Result<(), FmError> {
    Err(FmError::Unsupported(
        "restoring from the trash is not available on this platform",
    ))
}

/// Most entries a trash listing returns; the rest are reported as truncated.
pub const MAX_TRASH_ENTRIES: usize = 20_000;

/// One item in the platform trash, as shown by the trash manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// Platform identifier; opaque, only meaningful to the functions below.
    pub id: String,
    pub name: String,
    /// Folder the item was deleted from.
    pub original_parent: String,
    /// Seconds since the Unix epoch.
    pub deleted: i64,
    /// A folder whose contents can be browsed with [`list_trash_dir`].
    pub is_dir: bool,
}

/// One item inside a trashed folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashChild {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Most children listed for one trashed folder.
pub const MAX_TRASH_CHILDREN: usize = 5_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashChildren {
    pub entries: Vec<TrashChild>,
    pub truncated: bool,
}

/// The trash listing, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrashListing {
    pub entries: Vec<TrashEntry>,
    /// More than [`MAX_TRASH_ENTRIES`] items exist.
    pub truncated: bool,
}

#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
mod manage {
    use super::*;

    fn entry(item: &trash::TrashItem) -> TrashEntry {
        TrashEntry {
            id: item.id.to_string_lossy().into_owned(),
            name: item.name.to_string_lossy().into_owned(),
            original_parent: item.original_parent.to_string_lossy().into_owned(),
            deleted: item.time_deleted,
            is_dir: files_dir(&item.id.to_string_lossy(), &[]).is_ok(),
        }
    }

    /// Where a trashed item's data lives: `<trash>/info/<n>.trashinfo` -> `<trash>/files/<n>`,
    /// then down `path`. Refuses `..`, separators and symlinked folders on the way.
    fn item_path(id: &str, path: &[String]) -> Result<std::path::PathBuf, FmError> {
        let bad = || FmError::InvalidParams("not a trash entry".to_string());
        let info = Path::new(id);
        if info.extension().and_then(|e| e.to_str()) != Some("trashinfo") {
            return Err(bad());
        }
        let stem = info.file_stem().ok_or_else(bad)?;
        let info_dir = info.parent().ok_or_else(bad)?;
        if info_dir.file_name().and_then(|n| n.to_str()) != Some("info") || stem == ".." {
            return Err(bad());
        }
        let mut dir = info_dir.parent().ok_or_else(bad)?.join("files").join(stem);
        for part in path {
            if part.is_empty() || part == "." || part == ".." || part.contains(['/', '\0']) {
                return Err(bad());
            }
            dir.push(part);
        }
        Ok(dir)
    }

    fn files_dir(id: &str, path: &[String]) -> Result<std::path::PathBuf, FmError> {
        let dir = item_path(id, path)?;
        let meta = std::fs::symlink_metadata(&dir)?;
        if !meta.is_dir() {
            return Err(FmError::NotADirectory(dir.to_string_lossy().into_owned()));
        }
        Ok(dir)
    }

    /// Move one item from inside a trashed folder back to where it was, recreating missing
    /// parent folders. Never overwrites.
    pub fn restore_trash_child(
        id: &str,
        path: &[String],
        original_parent: &str,
        top_name: &str,
    ) -> Result<(), FmError> {
        if path.is_empty() || top_name.is_empty() || top_name.contains(['/', '\0']) {
            return Err(FmError::InvalidParams("not a trashed child".to_string()));
        }
        let source = item_path(id, path)?;
        std::fs::symlink_metadata(&source)?;
        let mut dest = Path::new(original_parent).join(top_name);
        dest.extend(path);
        if std::fs::symlink_metadata(&dest).is_ok() {
            return Err(FmError::AlreadyExists(dest.to_string_lossy().into_owned()));
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&source, &dest)?;
        Ok(())
    }

    /// The contents of a trashed folder (or a sub-folder of it), folders first.
    pub fn list_trash_dir(id: &str, path: &[String]) -> Result<TrashChildren, FmError> {
        let dir = files_dir(id, path)?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for item in std::fs::read_dir(&dir)? {
            if entries.len() >= MAX_TRASH_CHILDREN {
                truncated = true;
                break;
            }
            let Ok(item) = item else { continue };
            let Ok(meta) = item.metadata() else { continue };
            entries.push(TrashChild {
                name: item.file_name().to_string_lossy().into_owned(),
                is_dir: meta.is_dir(),
                size: if meta.is_dir() { 0 } else { meta.len() },
            });
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        Ok(TrashChildren { entries, truncated })
    }

    pub fn list_trash() -> Result<TrashListing, FmError> {
        let mut items = trash::os_limited::list().map_err(backend)?;
        items.sort_by_key(|i| std::cmp::Reverse(i.time_deleted));
        let truncated = items.len() > MAX_TRASH_ENTRIES;
        items.truncate(MAX_TRASH_ENTRIES);
        Ok(TrashListing {
            entries: items.iter().map(entry).collect(),
            truncated,
        })
    }

    fn pick(ids: &[String]) -> Result<Vec<trash::TrashItem>, FmError> {
        let items = trash::os_limited::list().map_err(backend)?;
        Ok(items
            .into_iter()
            .filter(|i| ids.iter().any(|id| *id == i.id.to_string_lossy()))
            .collect())
    }

    /// Put the given trashed items back where they were.
    pub fn restore_trash(ids: &[String]) -> Result<usize, FmError> {
        let items = pick(ids)?;
        let n = items.len();
        trash::os_limited::restore_all(items).map_err(backend)?;
        Ok(n)
    }

    /// Permanently remove the given trashed items.
    pub fn purge_trash(ids: &[String]) -> Result<usize, FmError> {
        let items = pick(ids)?;
        let n = items.len();
        trash::os_limited::purge_all(items).map_err(backend)?;
        Ok(n)
    }

    /// Permanently remove everything in the trash.
    pub fn empty_trash() -> Result<usize, FmError> {
        let items = trash::os_limited::list().map_err(backend)?;
        let n = items.len();
        trash::os_limited::purge_all(items).map_err(backend)?;
        Ok(n)
    }
}

#[cfg(not(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
)))]
mod manage {
    use super::*;

    const NO: &str = "managing the trash is not available on this platform";

    pub fn list_trash() -> Result<TrashListing, FmError> {
        Err(FmError::Unsupported(NO))
    }
    pub fn list_trash_dir(_id: &str, _path: &[String]) -> Result<TrashChildren, FmError> {
        Err(FmError::Unsupported(NO))
    }
    pub fn restore_trash_child(
        _id: &str,
        _path: &[String],
        _original_parent: &str,
        _top_name: &str,
    ) -> Result<(), FmError> {
        Err(FmError::Unsupported(NO))
    }
    pub fn restore_trash(_ids: &[String]) -> Result<usize, FmError> {
        Err(FmError::Unsupported(NO))
    }
    pub fn purge_trash(_ids: &[String]) -> Result<usize, FmError> {
        Err(FmError::Unsupported(NO))
    }
    pub fn empty_trash() -> Result<usize, FmError> {
        Err(FmError::Unsupported(NO))
    }
}

pub use manage::{
    empty_trash, list_trash, list_trash_dir, purge_trash, restore_trash, restore_trash_child,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_uris_are_unsupported_never_deleted() {
        let remote = VfsUri::parse("smb://nas/share/file.txt").expect("uri");
        assert!(matches!(trash_local(&remote), Err(FmError::Unsupported(_))));
        assert!(matches!(
            restore_local(&remote),
            Err(FmError::Unsupported(_))
        ));
    }

    #[test]
    fn missing_file_is_not_found() {
        let gone = VfsUri::parse("/definitely/not/here/xyz.txt").expect("uri");
        assert!(matches!(trash_local(&gone), Err(FmError::NotFound(_))));
    }

    // Uses the real platform trash; the file lives on the home volume so it lands in the
    // home trash, and the test restores it so nothing is left behind. Skips when the
    // machine has no usable trash (CI containers).
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn trash_then_restore_round_trip() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let Ok(dir) = tempfile::tempdir_in(home) else {
            return;
        };
        let file = dir.path().join("fm-trash-probe.txt");
        std::fs::write(&file, b"probe").expect("write");
        let uri = VfsUri::parse(&file.to_string_lossy()).expect("uri");
        if trash_local(&uri).is_err() {
            return;
        }
        assert!(!file.exists());
        restore_local(&uri).expect("restore");
        assert_eq!(std::fs::read(&file).expect("read"), b"probe");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn trashed_folder_contents_can_be_listed() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let Ok(dir) = tempfile::tempdir_in(home) else {
            return;
        };
        let name = format!("fm-trash-dir-{}", std::process::id());
        let folder = dir.path().join(&name);
        std::fs::create_dir_all(folder.join("sub")).expect("mkdir");
        std::fs::write(folder.join("sub").join("deep.txt"), b"x").expect("write");
        std::fs::write(folder.join("top.txt"), b"xyz").expect("write");
        let uri = VfsUri::parse(&folder.to_string_lossy()).expect("uri");
        if trash_local(&uri).is_err() {
            return;
        }
        let Ok(listing) = list_trash() else {
            return;
        };
        let Some(entry) = listing.entries.iter().find(|e| e.name == name) else {
            return;
        };
        assert!(entry.is_dir);
        let top = list_trash_dir(&entry.id, &[]).expect("top");
        let names: Vec<_> = top
            .entries
            .iter()
            .map(|c| (c.name.as_str(), c.is_dir))
            .collect();
        assert_eq!(names, [("sub", true), ("top.txt", false)]);
        let deep = list_trash_dir(&entry.id, &["sub".to_string()]).expect("deep");
        assert_eq!(deep.entries.len(), 1);
        for bad in [
            vec!["..".to_string()],
            vec!["a/b".to_string()],
            vec![String::new()],
        ] {
            assert!(list_trash_dir(&entry.id, &bad).is_err());
        }
        assert!(list_trash_dir("/etc/passwd", &[]).is_err());
        let (parent, id) = (entry.original_parent.clone(), entry.id.clone());
        let deep_path = ["sub".to_string(), "deep.txt".to_string()];
        restore_trash_child(&id, &deep_path, &parent, &name).expect("restore child");
        assert_eq!(
            std::fs::read(folder.join("sub").join("deep.txt")).expect("read"),
            b"x"
        );
        assert!(restore_trash_child(&id, &["top.txt".to_string()], &parent, &name).is_ok());
        assert!(
            list_trash_dir(&id, &[])
                .expect("left")
                .entries
                .iter()
                .all(|c| c.name != "top.txt")
        );
        purge_trash(&[id]).expect("purge");
    }

    // Trash a uniquely named file, find it in the listing, then purge it so the user's trash
    // ends up as it started. Skips where there is no usable trash.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn list_then_purge_round_trip() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let Ok(dir) = tempfile::tempdir_in(home) else {
            return;
        };
        let name = format!("fm-trash-list-{}.txt", std::process::id());
        let file = dir.path().join(&name);
        std::fs::write(&file, b"probe").expect("write");
        let uri = VfsUri::parse(&file.to_string_lossy()).expect("uri");
        if trash_local(&uri).is_err() {
            return;
        }
        let Ok(listing) = list_trash() else {
            return;
        };
        let found: Vec<String> = listing
            .entries
            .iter()
            .filter(|e| e.name == name)
            .map(|e| e.id.clone())
            .collect();
        if listing.truncated {
            return;
        }
        assert_eq!(found.len(), 1, "the trashed file is listed once");
        let entry = listing
            .entries
            .iter()
            .find(|e| e.name == name)
            .expect("entry");
        assert_eq!(
            entry.original_parent,
            dir.path().to_string_lossy().to_string()
        );
        assert_eq!(purge_trash(&found).expect("purge"), 1);
        let after = list_trash().expect("list");
        assert!(after.entries.iter().all(|e| e.name != name));
    }
}
