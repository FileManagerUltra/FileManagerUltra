//! Private, owner-only temporary folders (remote RAR copies, unpacked compressed tars,
//! staged archive changes); each is removed when dropped.
//!
//! Temporary copies of archives share one budget (Preferences › Files › Archives,
//! [`crate::limits::spool_cap`]): all open archives together, not per archive. A copy
//! reserves its size before it is written ([`SpoolDir::create_reserved`]) and gives it
//! back when its folder is removed. A copy is also refused when it would take more than
//! 80 % of the free space on the disk holding the temporary folder.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fm_vfs::FmError;

/// Bytes reserved by temporary copies of all open archives.
static RESERVED: AtomicU64 = AtomicU64::new(0);

/// Bytes the temporary copies of open archives hold now (for Preferences).
pub fn reserved_bytes() -> u64 {
    RESERVED.load(Ordering::Relaxed)
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / f64::from(1u32 << 30))
}

/// Reserve `bytes` of the budget counted in `reserved` under `cap`, given `free` bytes on
/// the temporary disk (`None`: unknown, not checked). `Err` says why the copy is not made.
fn reserve(reserved: &AtomicU64, bytes: u64, cap: u64, free: Option<u64>) -> Result<(), FmError> {
    if let Some(free) = free
        && u128::from(bytes) * 10 > u128::from(free) * 8
    {
        return Err(FmError::backend(
            "archive",
            format!(
                "a temporary copy of {} would leave too little free space ({} free)",
                gib(bytes),
                gib(free)
            ),
        ));
    }
    reserved
        .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
            used.checked_add(bytes).filter(|total| *total <= cap)
        })
        .map(|_| ())
        .map_err(|used| {
            FmError::backend(
                "archive",
                format!(
                    "temporary copies of open archives are limited to {} together ({} in \
                     use, {} more needed); close an archive or raise the limit in \
                     Preferences › Files",
                    gib(cap),
                    gib(used),
                    gib(bytes)
                ),
            )
        })
}

fn release(reserved: &AtomicU64, bytes: u64) {
    let _ = reserved.try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
        Some(used.saturating_sub(bytes))
    });
}

/// Free bytes on the disk holding the system temporary folder, when known.
fn temp_free_space() -> Option<u64> {
    use fm_vfs::Vfs as _;
    let dir = std::env::temp_dir();
    let uri = fm_vfs::VfsUri::parse(dir.to_str()?).ok()?;
    fm_fs_local::LocalFs::new()
        .space(&uri)
        .ok()
        .flatten()
        .map(|s| s.available)
}

pub struct SpoolDir {
    path: PathBuf,
    /// Bytes of the shared budget this folder holds, given back on drop.
    reserved: u64,
}

impl SpoolDir {
    /// A new `0700` folder in the system temp dir named `<prefix>-<pid>-<n>`.
    pub fn create(prefix: &str) -> Result<Self, FmError> {
        use std::sync::atomic::AtomicU32;
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(name);
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            let mut b = builder;
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            b
        };
        builder.create(&dir).map_err(FmError::Io)?;
        Ok(Self {
            path: dir,
            reserved: 0,
        })
    }

    /// [`Self::create`] for a temporary copy of `bytes`, counted against the shared
    /// budget of all open archives until the folder is dropped.
    pub fn create_reserved(prefix: &str, bytes: u64) -> Result<Self, FmError> {
        reserve(
            &RESERVED,
            bytes,
            crate::limits::spool_cap(),
            temp_free_space(),
        )?;
        match Self::create(prefix) {
            Ok(mut dir) => {
                dir.reserved = bytes;
                Ok(dir)
            }
            Err(e) => {
                release(&RESERVED, bytes);
                Err(e)
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SpoolDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        release(&RESERVED, self.reserved);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_is_shared_and_given_back() {
        const G: u64 = 1 << 30;
        let used = AtomicU64::new(0);
        reserve(&used, 3 * G, 4 * G, None).expect("first copy fits");
        let refused = reserve(&used, 2 * G, 4 * G, None).expect_err("together over the cap");
        assert!(refused.to_string().contains("together"), "{refused}");
        reserve(&used, G, 4 * G, None).expect("exactly the cap");
        release(&used, 3 * G);
        reserve(&used, 2 * G, 4 * G, None).expect("fits once the first is gone");
        assert_eq!(used.load(Ordering::Relaxed), 3 * G);
    }

    #[test]
    fn a_copy_must_leave_a_fifth_of_the_free_space() {
        const G: u64 = 1 << 30;
        let used = AtomicU64::new(0);
        reserve(&used, 8 * G, u64::MAX, Some(10 * G)).expect("exactly 80 %");
        release(&used, 8 * G);
        let refused = reserve(&used, 8 * G + 1, u64::MAX, Some(10 * G)).expect_err("over 80 %");
        assert!(refused.to_string().contains("free space"), "{refused}");
    }

    #[test]
    fn a_reserved_folder_releases_its_bytes_when_dropped() {
        let dir = SpoolDir::create_reserved("fm-test", 1).expect("tiny copy");
        let path = dir.path().to_path_buf();
        assert!(path.is_dir());
        assert!(reserved_bytes() >= 1);
        drop(dir);
        assert!(!path.exists(), "removed on drop");
    }
}
