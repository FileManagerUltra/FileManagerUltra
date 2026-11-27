//! Replacing the contents of an existing file the user edited (the text editor's save):
//! write a temporary sibling that only the owner can read, give it the original's
//! permissions and rename it over the target. Backends without rename write in place.
//! A failed save removes the temporary file and leaves the original as it was.

use std::io::Write;

use fm_vfs::{Entry, FmError, Vfs, VfsUri, WriteOpts};

use crate::transfer::{commit, temp_name};

const CHUNK: usize = 1024 * 1024;
/// Permission bits of the temporary file while it holds the new contents.
const OWNER_ONLY: u32 = 0o600;

/// Write `bytes` as the new contents of `dest`. `mode` is the permission set the file
/// had when it was loaded, applied to the new file where the backend supports it.
/// `cancelled` is polled between chunks. Returns the file's entry after the save, so
/// the caller can record its new size and modification time.
pub fn replace_contents(
    vfs: &dyn Vfs,
    dest: &VfsUri,
    bytes: &[u8],
    mode: Option<u32>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Entry, FmError> {
    let parent = dest
        .parent()
        .ok_or_else(|| FmError::NotADirectory(dest.to_string()))?;
    let exists = match vfs.stat(dest) {
        Ok(_) => true,
        Err(FmError::NotFound(_)) => false,
        Err(e) => return Err(e),
    };
    let tmp = parent.join(&temp_name(".fmsave"))?;
    let staged = write_all(vfs, &tmp, bytes, cancelled, true).and_then(|()| {
        if let Some(mode) = mode {
            // Best effort: some backends (SMB, Windows) have no Unix permissions.
            let _ = vfs.set_permissions(&tmp, mode & 0o7777);
        }
        commit(vfs, &tmp, dest, &parent, exists)
    });
    match staged {
        Ok(()) => {}
        Err(FmError::Unsupported(_)) => {
            // No rename on this backend: write the target in place.
            let _ = vfs.remove(&tmp);
            write_all(vfs, dest, bytes, cancelled, false)?;
        }
        Err(e) => {
            let _ = vfs.remove(&tmp);
            return Err(e);
        }
    }
    vfs.stat(dest)
}

fn write_all(
    vfs: &dyn Vfs,
    to: &VfsUri,
    bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
    private: bool,
) -> Result<(), FmError> {
    let mut w = vfs.open_write(to, WriteOpts::default())?;
    if private {
        let _ = vfs.set_permissions(to, OWNER_ONLY);
    }
    for chunk in bytes.chunks(CHUNK) {
        if cancelled() {
            return Err(FmError::Cancelled);
        }
        w.write_all(chunk)?;
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use fm_testkit::{FaultFs, MemFs};

    fn uri(p: &str) -> VfsUri {
        VfsUri::parse(p).expect("uri")
    }

    #[test]
    fn replaces_contents_and_leaves_no_temp_file() {
        let mem = MemFs::new();
        mem.put_file(&uri("/d/a.txt"), b"old").expect("put");
        let entry =
            replace_contents(&mem, &uri("/d/a.txt"), b"new text", None, &|| false).expect("save");
        assert_eq!(mem.get_file(&uri("/d/a.txt")).expect("get"), b"new text");
        assert_eq!(entry.size, Some(8));
        let names: Vec<String> = mem
            .read_dir(&uri("/d"))
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["a.txt".to_string()]);
    }

    #[test]
    fn failed_write_keeps_the_original_and_removes_the_temp() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner()
            .put_file(&uri("/d/a.txt"), b"original")
            .expect("put");
        fs.faults().fail_write_after = Some(2);
        let err = replace_contents(&fs, &uri("/d/a.txt"), b"new contents", None, &|| false);
        assert!(err.is_err());
        assert_eq!(
            fs.inner().get_file(&uri("/d/a.txt")).expect("get"),
            b"original"
        );
        assert_eq!(fs.inner().read_dir(&uri("/d")).expect("list").len(), 1);
    }

    #[test]
    fn without_rename_it_writes_in_place() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner()
            .put_file(&uri("/d/a.txt"), b"original")
            .expect("put");
        fs.faults().rename_unsupported = true;
        replace_contents(&fs, &uri("/d/a.txt"), b"in place", None, &|| false).expect("save");
        assert_eq!(
            fs.inner().get_file(&uri("/d/a.txt")).expect("get"),
            b"in place"
        );
        assert_eq!(fs.inner().read_dir(&uri("/d")).expect("list").len(), 1);
    }

    #[test]
    fn cancel_stops_before_anything_is_replaced() {
        let mem = MemFs::new();
        mem.put_file(&uri("/d/a.txt"), b"original").expect("put");
        let err = replace_contents(&mem, &uri("/d/a.txt"), b"new", None, &|| true);
        assert!(matches!(err, Err(FmError::Cancelled)));
        assert_eq!(mem.get_file(&uri("/d/a.txt")).expect("get"), b"original");
        assert_eq!(mem.read_dir(&uri("/d")).expect("list").len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn local_save_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("script.sh");
        std::fs::write(&path, b"echo hi").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).expect("chmod");
        let vfs = fm_fs_local::LocalFs::new();
        let dest = uri(&path.to_string_lossy());
        replace_contents(&vfs, &dest, b"echo bye", Some(0o750), &|| false).expect("save");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o7777;
        assert_eq!(mode, 0o750);
        assert_eq!(std::fs::read(&path).expect("read"), b"echo bye");
        assert_eq!(std::fs::read_dir(tmp.path()).expect("list").count(), 1);
    }
}
