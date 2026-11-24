//! Private, owner-only temporary folders (remote RAR copies, staged archive changes);
//! each is removed when dropped.

use std::path::{Path, PathBuf};

use fm_vfs::FmError;

pub struct SpoolDir(PathBuf);

impl SpoolDir {
    /// A new `0700` folder in the system temp dir named `<prefix>-<pid>-<n>`.
    pub fn create(prefix: &str) -> Result<Self, FmError> {
        use std::sync::atomic::{AtomicU32, Ordering};
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
        Ok(Self(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SpoolDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
