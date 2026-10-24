//! Thumbnail service for the preview pane's grid (capability `preview-pane`): a small worker
//! pool of its own (so decoding never blocks the file-job lanes), a byte-capped in-memory LRU
//! and the owner-only PNG disk cache from `fm-preview`.
//!
//! The UI asks for the window of entries it shows ([`ThumbService::want`]); a newer request
//! for the same pane supersedes older ones, so scrolling fast skips what scrolled past.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};
use fm_layout::PaneId;
use fm_preview::thumbs::{DiskCache, Thumb, ThumbKind, classify, make};
use fm_vfs::{Entry, Vfs, VfsUri};

/// Decoded thumbnails kept in memory (least recently drawn go first).
pub const MEMORY_BYTES: usize = 192 << 20;
/// The disk cache is trimmed to this on first use.
pub const DISK_BYTES: u64 = 256 << 20;
const MAX_WORKERS: usize = 4;

type Key = (VfsUri, Option<SystemTime>, Option<u64>, u32);

fn key_of(e: &Entry, edge: u32) -> Key {
    (e.uri.clone(), e.modified, e.size, edge)
}

/// What the grid knows about one entry's thumbnail.
#[derive(Clone, Debug)]
pub enum ThumbState {
    /// Not a thumbnail candidate, or not requested yet.
    None,
    Pending,
    Ready(Arc<Thumb>),
    /// Decoding failed (corrupt file, no ffmpeg, remote file): show the type tile.
    Failed,
}

#[derive(Debug)]
enum Slot {
    Pending(u64),
    Ready(Arc<Thumb>),
    Failed,
}

#[derive(Debug)]
struct Item {
    slot: Slot,
    used: u64,
}

#[derive(Debug, Default)]
struct Store {
    map: HashMap<Key, Item>,
    bytes: usize,
    tick: u64,
}

impl Store {
    fn evict(&mut self, cap: usize) {
        while self.bytes > cap {
            let oldest = self
                .map
                .iter()
                .filter(|(_, i)| matches!(i.slot, Slot::Ready(_)))
                .min_by_key(|(_, i)| i.used)
                .map(|(k, _)| k.clone());
            let Some(k) = oldest else { break };
            if let Some(Item {
                slot: Slot::Ready(t),
                ..
            }) = self.map.remove(&k)
            {
                self.bytes = self.bytes.saturating_sub(t.bytes());
            }
        }
    }
}

struct Job {
    key: Key,
    entry: Entry,
    pane: PaneId,
    epoch: u64,
    ticket: u64,
    vfs: Arc<dyn Vfs>,
}

struct Inner {
    store: Mutex<Store>,
    epochs: Mutex<HashMap<PaneId, u64>>,
    cache: RwLock<Option<DiskCache>>,
    done: AtomicU64,
    seen: AtomicU64,
    pending: AtomicUsize,
    pruned: AtomicBool,
    memory_cap: AtomicUsize,
}

impl Inner {
    fn epoch(&self, pane: PaneId) -> u64 {
        self.epochs
            .lock()
            .map(|e| e.get(&pane).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn finish(&self, job: &Job, thumb: Option<Thumb>) {
        let mut st = self.lock();
        let live = matches!(
            st.map.get(&job.key),
            Some(Item { slot: Slot::Pending(t), .. }) if *t == job.ticket
        );
        if live {
            st.tick += 1;
            let used = st.tick;
            let slot = match thumb {
                Some(t) => {
                    st.bytes += t.bytes();
                    Slot::Ready(Arc::new(t))
                }
                None => Slot::Failed,
            };
            st.map.insert(job.key.clone(), Item { slot, used });
            let cap = self.memory_cap.load(Ordering::Relaxed);
            st.evict(cap);
            self.done.fetch_add(1, Ordering::Release);
        }
        drop(st);
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }

    fn run(&self, job: &Job) {
        let live = {
            let st = self.lock();
            matches!(
                st.map.get(&job.key),
                Some(Item { slot: Slot::Pending(t), .. }) if *t == job.ticket
            )
        };
        if !live {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        if self.epoch(job.pane) != job.epoch {
            // Scrolled away before we got to it; a later request re-queues it if needed.
            let mut st = self.lock();
            st.map.remove(&job.key);
            drop(st);
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        let cache = self.cache.read().ok().and_then(|c| c.clone());
        let disk_key = cache
            .as_ref()
            .map(|_| DiskCache::key(&job.entry, job.key.3));
        let cached = cache
            .as_ref()
            .zip(disk_key.as_deref())
            .and_then(|(c, k)| c.get(k));
        let thumb = cached.or_else(|| {
            let made = catch_unwind(AssertUnwindSafe(|| make(&*job.vfs, &job.entry, job.key.3)))
                .ok()
                .flatten();
            if let (Some(t), Some(c), Some(k)) = (&made, &cache, &disk_key) {
                c.put(k, t);
            }
            made
        });
        self.finish(job, thumb);
    }
}

fn worker(inner: &Inner, rx: &Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        if let Some(c) = inner.cache.read().ok().and_then(|c| c.clone())
            && !inner.pruned.swap(true, Ordering::AcqRel)
        {
            c.prune(DISK_BYTES);
        }
        inner.run(&job);
    }
}

pub struct ThumbService {
    inner: Arc<Inner>,
    tx: Mutex<Option<Sender<Job>>>,
    next: AtomicU64,
}

impl std::fmt::Debug for ThumbService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThumbService")
            .field("pending", &self.inner.pending.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for ThumbService {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                store: Mutex::new(Store::default()),
                epochs: Mutex::new(HashMap::new()),
                cache: RwLock::new(None),
                done: AtomicU64::new(0),
                seen: AtomicU64::new(0),
                pending: AtomicUsize::new(0),
                pruned: AtomicBool::new(false),
                memory_cap: AtomicUsize::new(MEMORY_BYTES),
            }),
            tx: Mutex::new(None),
            next: AtomicU64::new(1),
        }
    }
}

impl ThumbService {
    /// Use (or stop using) a disk cache directory.
    pub fn set_cache_dir(&self, dir: Option<std::path::PathBuf>) {
        if let Ok(mut c) = self.inner.cache.write() {
            *c = dir.and_then(DiskCache::open);
        }
        self.inner.pruned.store(false, Ordering::Release);
    }

    pub fn set_memory_cap(&self, bytes: usize) {
        self.inner.memory_cap.store(bytes, Ordering::Relaxed);
    }

    /// Whether `entry` gets a thumbnail attempt at all.
    pub fn candidate(entry: &Entry) -> bool {
        !entry.is_dir_like() && classify(&entry.name) != ThumbKind::Other
    }

    pub fn get(&self, entry: &Entry, edge: u32) -> ThumbState {
        if !Self::candidate(entry) {
            return ThumbState::None;
        }
        let mut st = self.inner.lock();
        st.tick += 1;
        let tick = st.tick;
        match st.map.get_mut(&key_of(entry, edge)) {
            None => ThumbState::None,
            Some(item) => {
                item.used = tick;
                match &item.slot {
                    Slot::Pending(_) => ThumbState::Pending,
                    Slot::Ready(t) => ThumbState::Ready(t.clone()),
                    Slot::Failed => ThumbState::Failed,
                }
            }
        }
    }

    /// Request thumbnails for `entries` in order, superseding earlier requests of `pane`.
    pub fn want(&self, pane: PaneId, vfs: &Arc<dyn Vfs>, entries: &[Entry], edge: u32) {
        let epoch = self.next.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut e) = self.inner.epochs.lock() {
            e.insert(pane, epoch);
        }
        let Some(tx) = self.sender() else { return };
        for entry in entries.iter().filter(|e| Self::candidate(e)) {
            let key = key_of(entry, edge);
            let ticket = self.next.fetch_add(1, Ordering::Relaxed);
            {
                let mut st = self.inner.lock();
                match st.map.get(&key).map(|i| &i.slot) {
                    Some(Slot::Ready(_) | Slot::Failed) => continue,
                    // Queued by an older request of this pane: take it over.
                    Some(Slot::Pending(_)) | None => {
                        st.tick += 1;
                        let used = st.tick;
                        st.map.insert(
                            key.clone(),
                            Item {
                                slot: Slot::Pending(ticket),
                                used,
                            },
                        );
                    }
                }
            }
            self.inner.pending.fetch_add(1, Ordering::AcqRel);
            let job = Job {
                key,
                entry: entry.clone(),
                pane,
                epoch,
                ticket,
                vfs: vfs.clone(),
            };
            if tx.send(job).is_err() {
                self.inner.pending.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    /// Forget a pane's queued work (it stopped showing thumbnails).
    pub fn cancel(&self, pane: PaneId) {
        let epoch = self.next.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut e) = self.inner.epochs.lock() {
            e.insert(pane, epoch);
        }
    }

    /// Drop every in-memory thumbnail.
    pub fn clear_memory(&self) {
        let mut st = self.inner.lock();
        st.map.retain(|_, i| matches!(i.slot, Slot::Pending(_)));
        st.bytes = 0;
    }

    /// Delete the disk cache files; returns how many went.
    pub fn clear_disk(&self) -> usize {
        self.inner
            .cache
            .read()
            .ok()
            .and_then(|c| c.as_ref().map(DiskCache::clear))
            .unwrap_or(0)
    }

    pub fn is_busy(&self) -> bool {
        self.inner.pending.load(Ordering::Acquire) > 0
    }

    pub fn pending(&self) -> usize {
        self.inner.pending.load(Ordering::Acquire)
    }

    /// Whether any thumbnail finished since the last call (the UI should repaint).
    pub fn take_changed(&self) -> bool {
        let done = self.inner.done.load(Ordering::Acquire);
        self.inner.seen.swap(done, Ordering::AcqRel) != done
    }

    pub fn memory_bytes(&self) -> usize {
        self.inner.lock().bytes
    }

    fn sender(&self) -> Option<Sender<Job>> {
        let mut guard = self.tx.lock().ok()?;
        if guard.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded::<Job>();
            let workers = std::thread::available_parallelism()
                .map_or(2, usize::from)
                .clamp(1, MAX_WORKERS);
            for n in 0..workers {
                let (inner, rx) = (self.inner.clone(), rx.clone());
                std::thread::Builder::new()
                    .name(format!("fm-thumb-{n}"))
                    .spawn(move || worker(&inner, &rx))
                    .ok()?;
            }
            *guard = Some(tx);
        }
        guard.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_vfs::EntryKind;
    use std::time::{Duration, Instant};

    fn png(dir: &std::path::Path, name: &str, w: u32, h: u32) -> Entry {
        let path = dir.join(name);
        image::RgbaImage::from_pixel(w, h, image::Rgba([9, 8, 7, 255]))
            .save(&path)
            .expect("png");
        let mut e = Entry::new(
            VfsUri::parse(&path.to_string_lossy()).expect("uri"),
            EntryKind::File,
        );
        e.size = std::fs::metadata(&path).ok().map(|m| m.len());
        e
    }

    fn wait_idle(svc: &ThumbService) {
        let end = Instant::now() + Duration::from_secs(20);
        while svc.is_busy() {
            assert!(Instant::now() < end, "thumbnails never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn vfs() -> Arc<dyn Vfs> {
        crate::backends::VfsRegistry::with_local()
            .require(&VfsUri::parse("/tmp").expect("uri"))
            .expect("local vfs")
    }

    #[test]
    fn makes_downscaled_thumbnails_and_reports_progress() {
        let dir = tempfile::tempdir().expect("tmp");
        let big = png(dir.path(), "big.png", 800, 400);
        let svc = ThumbService::default();
        assert!(matches!(svc.get(&big, 160), ThumbState::None));
        svc.want(1, &vfs(), std::slice::from_ref(&big), 160);
        wait_idle(&svc);
        assert!(svc.take_changed());
        assert!(!svc.take_changed());
        match svc.get(&big, 160) {
            ThumbState::Ready(t) => assert_eq!((t.width, t.height), (160, 80)),
            other => panic!("expected a thumbnail, got {other:?}"),
        }
    }

    #[test]
    fn corrupt_and_non_images_do_not_get_stuck() {
        let dir = tempfile::tempdir().expect("tmp");
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, b"definitely not a png").expect("write");
        let mut e = Entry::new(
            VfsUri::parse(&bad.to_string_lossy()).expect("uri"),
            EntryKind::File,
        );
        e.size = Some(20);
        let txt = Entry::new(VfsUri::parse("/tmp/a.txt").expect("uri"), EntryKind::File);
        let svc = ThumbService::default();
        svc.want(1, &vfs(), &[e.clone(), txt.clone()], 160);
        wait_idle(&svc);
        assert!(matches!(svc.get(&e, 160), ThumbState::Failed));
        assert!(matches!(svc.get(&txt, 160), ThumbState::None));
    }

    #[test]
    fn a_newer_request_supersedes_queued_work() {
        let dir = tempfile::tempdir().expect("tmp");
        let all: Vec<Entry> = (0..40)
            .map(|i| png(dir.path(), &format!("{i:02}.png"), 64, 64))
            .collect();
        let svc = ThumbService::default();
        svc.want(1, &vfs(), &all[..30], 160);
        svc.want(1, &vfs(), &all[30..], 160);
        wait_idle(&svc);
        for e in &all[30..] {
            assert!(matches!(svc.get(e, 160), ThumbState::Ready(_)));
        }
        // Whatever was dropped can be requested again.
        svc.want(1, &vfs(), &all[..30], 160);
        wait_idle(&svc);
        for e in &all {
            assert!(matches!(svc.get(e, 160), ThumbState::Ready(_)));
        }
    }

    #[test]
    fn memory_is_capped_and_disk_cache_serves_the_rest() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = tempfile::tempdir().expect("tmp");
        let all: Vec<Entry> = (0..6)
            .map(|i| png(dir.path(), &format!("{i}.png"), 64, 64))
            .collect();
        let svc = ThumbService::default();
        svc.set_cache_dir(Some(cache.path().join("thumbs")));
        svc.set_memory_cap(64 * 64 * 4 * 2);
        svc.want(1, &vfs(), &all, 160);
        wait_idle(&svc);
        assert!(svc.memory_bytes() <= 64 * 64 * 4 * 2);
        let on_disk = std::fs::read_dir(cache.path().join("thumbs"))
            .expect("dir")
            .count();
        assert_eq!(on_disk, 6);
        // Evicted entries come back from disk without touching the source files.
        std::fs::remove_dir_all(dir.path()).expect("rm");
        svc.clear_memory();
        svc.want(1, &vfs(), &all, 160);
        wait_idle(&svc);
        assert!(
            all.iter()
                .any(|e| matches!(svc.get(e, 160), ThumbState::Ready(_)))
        );
        assert_eq!(svc.clear_disk(), 6);
    }
}
