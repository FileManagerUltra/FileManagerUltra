//! Platform trash (design D6). Local files only; a failure is an error, never a silent
//! downgrade to permanent deletion.

use std::path::Path;

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

fn local_path(uri: &VfsUri) -> Result<&Path, FmError> {
    if uri.scheme() != Scheme::File || !uri.authority().is_local() {
        return Err(FmError::Unsupported(
            "the trash is only available for local files",
        ));
    }
    Ok(Path::new(uri.path_str()))
}

fn backend(err: trash::Error) -> FmError {
    FmError::backend("trash", err.to_string())
}

/// Move one local item to the platform trash. Does not follow a symlink.
pub fn trash_local(uri: &VfsUri) -> Result<(), FmError> {
    let path = local_path(uri)?;
    if std::fs::symlink_metadata(path).is_err() {
        return Err(FmError::NotFound(uri.to_string()));
    }
    trash::delete(path).map_err(backend)
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
    if std::fs::symlink_metadata(path).is_ok() {
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
}
