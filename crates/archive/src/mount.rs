//! [`LazyArchive`]: the backend registered for one archive handle. It opens the archive on
//! first use, which happens on the archive job lane (a directory read), never in a frame.
//! Errors are not cached, so a read that needed a password succeeds once one is added.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fm_vfs::{Attrs, Entry, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

use crate::format::{self, FormatId, Passwords, Progress};
use crate::fs::ArchiveFs;
use crate::source::{SliceSource, Source, VfsSource};

/// Bytes read to recognise a format (enough for the ISO signature at 32 769).
const HEAD: usize = 32_774;

pub struct LazyArchive {
    root: VfsUri,
    container: VfsUri,
    container_vfs: Arc<dyn Vfs>,
    forced: Option<FormatId>,
    offset: u64,
    passwords: Arc<Passwords>,
    state: Mutex<Option<Arc<ArchiveFs>>>,
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
}

impl LazyArchive {
    /// `root` is `archive://<handle>/`; `container` the archive file on `container_vfs`.
    pub fn new(
        root: VfsUri,
        container: VfsUri,
        container_vfs: Arc<dyn Vfs>,
        forced: Option<FormatId>,
        offset: u64,
    ) -> Self {
        Self {
            root: root.root(),
            container,
            container_vfs,
            forced,
            offset,
            passwords: Arc::new(Passwords::default()),
            state: Mutex::new(None),
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
        }
    }

    pub fn container(&self) -> &VfsUri {
        &self.container
    }

    pub fn root(&self) -> &VfsUri {
        &self.root
    }

    pub fn passwords(&self) -> &Arc<Passwords> {
        &self.passwords
    }

    /// The opened archive, if it is open.
    pub fn opened(&self) -> Option<Arc<ArchiveFs>> {
        self.state.lock().ok().and_then(|s| s.clone())
    }

    /// `(done, total)` bytes of the current index build, while one runs.
    pub fn progress(&self) -> Option<(u64, u64)> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0 && self.opened().is_none()).then(|| (self.done.load(Ordering::Relaxed), total))
    }

    /// Stop an index build in progress (leaving the location).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Close the archive and forget its passwords; the next use opens it again.
    pub fn unmount(&self) {
        if let Ok(mut s) = self.state.lock() {
            *s = None;
        }
        self.passwords.forget();
    }

    fn name(&self) -> String {
        self.container.file_name().unwrap_or("archive").to_string()
    }

    /// Open the archive now (blocking; call from a job lane).
    pub fn mount(&self) -> Result<Arc<ArchiveFs>, FmError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FmError::backend("archive", "archive state poisoned"))?;
        if let Some(fs) = state.as_ref() {
            return Ok(fs.clone());
        }
        self.cancel.store(false, Ordering::Relaxed);
        let name = self.name();
        let file: Arc<dyn Source> = Arc::new(VfsSource::open(
            self.container_vfs.clone(),
            self.container.clone(),
        )?);
        let src: Arc<dyn Source> = if self.offset > 0 {
            let len = file.len().saturating_sub(self.offset);
            Arc::new(SliceSource::new(file, self.offset, len))
        } else {
            file
        };
        let mut head = vec![0u8; HEAD.min(usize::try_from(src.len()).unwrap_or(HEAD))];
        let mut at = 0;
        while at < head.len() {
            let n = src.read_at(at as u64, head.get_mut(at..).unwrap_or(&mut []))?;
            if n == 0 {
                break;
            }
            at += n;
        }
        head.truncate(at);
        let format = match self.forced.or_else(|| format::by_name(&name)) {
            Some(f) if format::signature_fits(f, &head) => f,
            Some(f) => {
                return Err(FmError::backend(
                    "archive",
                    format!("{name} is not a {} archive", f.label()),
                ));
            }
            None => format::sniff(&head).ok_or_else(|| {
                FmError::backend(
                    "archive",
                    format!("{name} is not an archive this app can open"),
                )
            })?,
        };
        self.total.store(src.len(), Ordering::Relaxed);
        let report = |d: u64, _t: u64| self.done.store(d, Ordering::Relaxed);
        let cancelled = || self.cancel.load(Ordering::Relaxed);
        let progress = Progress {
            report: &report,
            cancelled: &cancelled,
        };
        let reader =
            format::guarded(|| format::open(src, format, &name, &self.passwords, &progress));
        self.total.store(0, Ordering::Relaxed);
        let fs = Arc::new(ArchiveFs::new(
            self.root.clone(),
            reader?,
            self.passwords.clone(),
        ));
        *state = Some(fs.clone());
        Ok(fs)
    }
}

impl Vfs for LazyArchive {
    fn scheme(&self) -> Scheme {
        Scheme::Archive
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        self.mount()?.read_dir(uri)
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        self.mount()?.stat(uri)
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        self.mount()?.open_read(uri)
    }

    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        self.mount()?.open_write(uri, opts)
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        self.mount()?.create_dir(uri)
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        self.mount()?.remove(uri)
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        self.mount()?.rename(from, to)
    }

    fn stat_full(&self, uri: &VfsUri) -> Result<Attrs, FmError> {
        self.mount()?.stat_full(uri)
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        a.archive_handle().is_some() && a.archive_handle() == b.archive_handle()
    }

    fn space(&self, _: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs_with(name: &str, bytes: &[u8]) -> (Arc<dyn Vfs>, VfsUri) {
        let mem = Arc::new(fm_testkit::MemFs::new());
        let uri = VfsUri::parse(&format!("/data/{name}")).expect("uri");
        mem.mkdirs_str("/data").expect("dir");
        mem.put_file(&uri, bytes).expect("put");
        (mem, uri)
    }

    #[test]
    fn mounts_on_first_use_and_names_a_wrong_signature() {
        let (vfs, uri) = fs_with("site.zip", &format::zip::tests::sample_zip());
        let root = VfsUri::archive_root(&crate::handle_for(&uri, 0, None)).expect("root");
        let lazy = LazyArchive::new(root.clone(), uri, vfs, None, 0);
        assert!(lazy.opened().is_none());
        assert_eq!(lazy.read_dir(&root).expect("list").len(), 3);
        assert!(lazy.opened().is_some());
        lazy.unmount();
        assert!(lazy.opened().is_none());

        let (vfs, uri) = fs_with("notes.zip", b"just some text, not a zip at all");
        let root = VfsUri::archive_root(&crate::handle_for(&uri, 0, None)).expect("root");
        let lazy = LazyArchive::new(root.clone(), uri, vfs, None, 0);
        let err = lazy.read_dir(&root).expect_err("not a zip");
        assert!(
            err.to_string().contains("notes.zip is not a zip archive"),
            "{err}"
        );
    }

    #[test]
    fn nested_archives_mount_through_each_other() {
        use std::io::Write as _;
        // A zip inside a tar.gz.
        let inner = format::zip::tests::sample_zip();
        let mut b = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_size(inner.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, "site.zip", &inner[..])
            .expect("append");
        let tar = b.into_inner().expect("tar");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).expect("gz");
        let (vfs, uri) = fs_with("backups.tar.gz", &gz.finish().expect("gz"));

        let outer_root = VfsUri::archive_root(&crate::handle_for(&uri, 0, None)).expect("root");
        let outer: Arc<dyn Vfs> = Arc::new(LazyArchive::new(outer_root.clone(), uri, vfs, None, 0));
        let inner_uri = outer_root.join("site.zip").expect("inner");
        let inner_root =
            VfsUri::archive_root(&crate::handle_for(&inner_uri, 0, None)).expect("root");
        let inner = LazyArchive::new(inner_root.clone(), inner_uri, outer, None, 0);
        let page = inner_root
            .join("www")
            .and_then(|w| w.join("index.html"))
            .expect("page");
        let mut s = String::new();
        inner
            .open_read(&page)
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        assert_eq!(s, "<h1>hi</h1>");
    }
}
