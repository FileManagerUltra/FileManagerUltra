//! [`FaultFs`] — a fault-injecting [`Vfs`] wrapper for `ops`/`engine` tests.
//!
//! Wraps any backend and, on demand, makes reads small and observable, writes fail
//! after N bytes, files vanish, renames unsupported, or `same_device` lie. Faults can
//! be changed while a test runs (shared state), so a hook can cancel a job mid-copy.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use fm_vfs::{Entry, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

/// Callback invoked after every `read` call on a wrapped reader.
pub type ReadHook = Arc<dyn Fn() + Send + Sync>;

/// The faults currently armed. All off by default.
#[derive(Default)]
pub struct Faults {
    /// Readers return at most this many bytes per call (progress ticks).
    pub read_chunk: Option<usize>,
    /// Called after each read call (e.g. to flip a cancel flag mid-file).
    pub read_hook: Option<ReadHook>,
    /// Writers fail with an I/O error once this many bytes were accepted in one file.
    pub fail_write_after: Option<u64>,
    /// `stat`/`open_read` report these as missing.
    pub vanished: Vec<VfsUri>,
    /// `read_dir` on these returns a permission error (unreadable subfolders).
    pub unreadable_dirs: Vec<VfsUri>,
    /// `rename` returns `Unsupported`.
    pub rename_unsupported: bool,
    /// Overrides `same_device`.
    pub same_device: Option<bool>,
}

/// Fault-injecting wrapper around another [`Vfs`].
#[derive(Clone)]
pub struct FaultFs<V: Vfs> {
    inner: Arc<V>,
    faults: Arc<Mutex<Faults>>,
}

impl<V: Vfs> FaultFs<V> {
    pub fn new(inner: V) -> Self {
        Self {
            inner: Arc::new(inner),
            faults: Arc::new(Mutex::new(Faults::default())),
        }
    }

    /// The wrapped backend (for assertions and fixtures).
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Arm or disarm faults.
    pub fn faults(&self) -> MutexGuard<'_, Faults> {
        self.faults.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn is_vanished(&self, uri: &VfsUri) -> bool {
        self.faults().vanished.contains(uri)
    }
}

struct FaultReader {
    inner: Box<dyn Read + Send>,
    chunk: Option<usize>,
    hook: Option<ReadHook>,
}

impl Read for FaultReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let cap = self.chunk.map_or(buf.len(), |c| c.min(buf.len()).max(1));
        let n = match buf.get_mut(..cap) {
            Some(window) => self.inner.read(window)?,
            None => 0,
        };
        if let Some(hook) = &self.hook {
            hook();
        }
        Ok(n)
    }
}

struct FaultWriter {
    inner: Box<dyn Write + Send>,
    written: u64,
    fail_after: Option<u64>,
}

impl Write for FaultWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if let Some(limit) = self.fail_after {
            if self.written >= limit {
                return Err(std::io::Error::other("injected write failure"));
            }
            let room = usize::try_from(limit - self.written).unwrap_or(usize::MAX);
            let take = data.len().min(room);
            let n = match data.get(..take) {
                Some(part) => self.inner.write(part)?,
                None => 0,
            };
            self.written += n as u64;
            return Ok(n);
        }
        let n = self.inner.write(data)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<V: Vfs> Vfs for FaultFs<V> {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        if self.faults().unreadable_dirs.contains(uri) {
            return Err(FmError::PermissionDenied(uri.to_string()));
        }
        self.inner.read_dir(uri)
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        if self.is_vanished(uri) {
            return Err(FmError::NotFound(uri.to_string()));
        }
        self.inner.stat(uri)
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        if self.is_vanished(uri) {
            return Err(FmError::NotFound(uri.to_string()));
        }
        let inner = self.inner.open_read(uri)?;
        let faults = self.faults();
        Ok(Box::new(FaultReader {
            inner,
            chunk: faults.read_chunk,
            hook: faults.read_hook.clone(),
        }))
    }

    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        let inner = self.inner.open_write(uri, opts)?;
        let fail_after = self.faults().fail_write_after;
        Ok(Box::new(FaultWriter {
            inner,
            written: 0,
            fail_after,
        }))
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        self.inner.create_dir(uri)
    }

    fn create_symlink(&self, target: &str, link: &VfsUri) -> Result<(), FmError> {
        self.inner.create_symlink(target, link)
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        self.inner.remove(uri)
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        if self.faults().rename_unsupported {
            return Err(FmError::Unsupported("rename (injected)"));
        }
        self.inner.rename(from, to)
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        match self.faults().same_device {
            Some(forced) => forced,
            None => self.inner.same_device(a, b),
        }
    }

    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        self.inner.space(uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemFs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn uri(s: &str) -> VfsUri {
        VfsUri::parse(s).expect("uri")
    }

    #[test]
    fn passthrough_by_default() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner().put_str("/a/f.txt", b"hello").expect("put");
        let mut buf = String::new();
        fs.open_read(&uri("/a/f.txt"))
            .expect("open")
            .read_to_string(&mut buf)
            .expect("read");
        assert_eq!(buf, "hello");
    }

    #[test]
    fn small_reads_tick_the_hook() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner().put_str("/f", &[7u8; 100]).expect("put");
        let ticks = Arc::new(AtomicUsize::new(0));
        {
            let t = ticks.clone();
            let mut f = fs.faults();
            f.read_chunk = Some(10);
            f.read_hook = Some(Arc::new(move || {
                t.fetch_add(1, Ordering::SeqCst);
            }));
        }
        let mut sink = Vec::new();
        fs.open_read(&uri("/f"))
            .expect("open")
            .read_to_end(&mut sink)
            .expect("read");
        assert_eq!(sink.len(), 100);
        assert!(ticks.load(Ordering::SeqCst) >= 10);
    }

    #[test]
    fn writes_fail_after_the_limit() {
        let fs = FaultFs::new(MemFs::new());
        fs.faults().fail_write_after = Some(4);
        let mut w = fs
            .open_write(&uri("/out"), WriteOpts::default())
            .expect("open");
        assert!(w.write_all(b"12345678").is_err());
    }

    #[test]
    fn vanished_files_are_not_found() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner().put_str("/gone", b"x").expect("put");
        fs.faults().vanished.push(uri("/gone"));
        assert!(matches!(fs.stat(&uri("/gone")), Err(FmError::NotFound(_))));
        assert!(fs.open_read(&uri("/gone")).is_err());
    }

    #[test]
    fn rename_and_device_overrides() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner().put_str("/a", b"x").expect("put");
        fs.faults().rename_unsupported = true;
        fs.faults().same_device = Some(false);
        assert!(matches!(
            fs.rename(&uri("/a"), &uri("/b")),
            Err(FmError::Unsupported(_))
        ));
        assert!(!fs.same_device(&uri("/a"), &uri("/b")));
    }
}
