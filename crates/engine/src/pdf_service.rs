//! PDF render service (change `add-pdf-preview`, design D3/D10): two workers of its own
//! (so a large PDF never queues behind file copies), a byte-capped RGBA cache, per-page
//! text extraction and signature verification.
//!
//! hayro cannot be cancelled mid-page, so a job that runs past its deadline is abandoned:
//! its result is discarded, its worker replaced, and the page shows an error tile. After
//! [`MAX_ABANDONED`] abandoned jobs a document stops rendering ("too complex").

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fm_layout::PaneId;
use fm_pdf::render::{ColorMode, Pixels, Tile};
use fm_pdf::verify::SignatureReport;
use fm_pdf::{Document, render, text, verify};
use fm_preview::pdf_prev::TextCache;

/// Render workers.
pub const WORKERS: usize = 2;
/// Rendered pages and tiles kept in memory.
pub const CACHE_BYTES: usize = 192 << 20;
/// A page render (or text extraction) running longer is abandoned.
pub const DEADLINE: Duration = Duration::from_secs(10);
/// Signature verification budget per document.
pub const VERIFY_BUDGET: Duration = Duration::from_secs(5);
/// Abandoned jobs after which a document stops rendering.
pub const MAX_ABANDONED: u32 = 2;
/// Edge of a tile (pages above the pixel cap render as tiles of the visible area).
pub const TILE: u32 = 1024;

/// One rendered bitmap: a whole page or a tile, at a scale step of 0.25.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderKey {
    pub doc: u64,
    pub page: usize,
    /// Scale in quarters (4 = 1.0 pixel per point).
    pub step: u32,
    pub colors: ColorMode,
    pub tile: Option<Tile>,
}

impl RenderKey {
    pub fn scale(&self) -> f32 {
        self.step as f32 / 4.0
    }

    /// The step at or above `scale` (sharp after downscaling), within 0.25–64.
    pub fn step_for(scale: f32) -> u32 {
        if !scale.is_finite() {
            return 4;
        }
        (scale * 4.0).ceil().clamp(1.0, 256.0) as u32
    }
}

/// What the view may draw for a key.
#[derive(Clone, Debug)]
pub enum RenderState {
    Pending,
    Ready(Arc<Pixels>),
    Failed(String),
}

/// Signature verification of one document.
#[derive(Clone, Debug)]
pub enum VerifyState {
    Running,
    Done(Arc<Vec<SignatureReport>>),
    TimedOut,
}

enum Work {
    Render {
        key: RenderKey,
        doc: Arc<Document>,
    },
    Text {
        doc_id: u64,
        page: usize,
        doc: Arc<Document>,
        cache: Arc<TextCache>,
    },
    Verify {
        doc_id: u64,
        doc: Arc<Document>,
    },
    /// Anything else that must stay off the UI thread and the job lanes (form rebuilds).
    Task(Box<dyn FnOnce() + Send>),
}

impl Work {
    fn deadline(&self) -> Option<Duration> {
        match self {
            Self::Render { .. } | Self::Text { .. } => Some(DEADLINE),
            Self::Verify { .. } => Some(VERIFY_BUDGET),
            Self::Task(_) => None,
        }
    }

    fn doc_id(&self) -> Option<u64> {
        match self {
            Self::Render { key, .. } => Some(key.doc),
            Self::Text { doc_id, .. } | Self::Verify { doc_id, .. } => Some(*doc_id),
            Self::Task(_) => None,
        }
    }
}

/// What a running job is, for the supervisor.
#[derive(Clone, Copy, Debug)]
enum Tag {
    Render(RenderKey),
    Text(u64, usize),
    Verify(u64),
    Task,
}

struct Running {
    since: Instant,
    tag: Tag,
    deadline: Option<Duration>,
    doc: Option<u64>,
}

struct WorkerSlot {
    id: u64,
    running: Option<Running>,
    abandoned: Arc<AtomicBool>,
}

#[derive(Default)]
struct Queue {
    /// Visible pages first, by pane; replaced on every `want`.
    renders: VecDeque<(PaneId, Work)>,
    others: VecDeque<Work>,
    workers: Vec<WorkerSlot>,
    started: bool,
}

struct Item {
    state: RenderState,
    used: u64,
    bytes: usize,
}

#[derive(Default)]
struct Store {
    renders: HashMap<RenderKey, Item>,
    bytes: usize,
    tick: u64,
    abandoned: HashMap<u64, u32>,
    text_pending: HashSet<(u64, usize)>,
    /// Pages whose text extraction ran out of time (treated as having no text).
    text_failed: HashSet<(u64, usize)>,
    verify: HashMap<u64, VerifyState>,
}

impl Store {
    fn evict(&mut self) {
        while self.bytes > CACHE_BYTES {
            let oldest = self
                .renders
                .iter()
                .filter(|(_, i)| matches!(i.state, RenderState::Ready(_)))
                .min_by_key(|(_, i)| i.used)
                .map(|(k, _)| *k);
            let Some(k) = oldest else { break };
            if let Some(i) = self.renders.remove(&k) {
                self.bytes = self.bytes.saturating_sub(i.bytes);
            }
        }
    }
}

struct Inner {
    queue: Mutex<Queue>,
    work: Condvar,
    store: Mutex<Store>,
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    changed: AtomicBool,
    next_worker: AtomicU64,
}

impl Inner {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wake(&self) {
        self.changed.store(true, Ordering::Release);
        let waker = self.waker.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(w) = waker {
            w();
        }
    }
}

/// The service; cheap to share.
#[derive(Clone)]
pub struct PdfService {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PdfService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfService").finish_non_exhaustive()
    }
}

impl Default for PdfService {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                queue: Mutex::new(Queue::default()),
                work: Condvar::new(),
                store: Mutex::new(Store::default()),
                waker: Mutex::new(None),
                changed: AtomicBool::new(false),
                next_worker: AtomicU64::new(1),
            }),
        }
    }
}

fn spawn_worker(inner: &Arc<Inner>, q: &mut Queue) {
    let id = inner.next_worker.fetch_add(1, Ordering::Relaxed);
    let abandoned = Arc::new(AtomicBool::new(false));
    q.workers.push(WorkerSlot {
        id,
        running: None,
        abandoned: abandoned.clone(),
    });
    let inner = inner.clone();
    let spawned = std::thread::Builder::new()
        .name("fm-pdf".into())
        .spawn(move || worker(&inner, id, &abandoned));
    if spawned.is_err() {
        q.workers.retain(|w| w.id != id);
    }
}

fn worker(inner: &Arc<Inner>, id: u64, abandoned: &AtomicBool) {
    loop {
        let job = {
            let mut q = inner.queue();
            loop {
                if abandoned.load(Ordering::Acquire) {
                    return;
                }
                let next = q
                    .renders
                    .pop_front()
                    .map(|(_, w)| w)
                    .or_else(|| q.others.pop_front());
                if let Some(job) = next {
                    let tag = match &job {
                        Work::Render { key, .. } => Tag::Render(*key),
                        Work::Text { doc_id, page, .. } => Tag::Text(*doc_id, *page),
                        Work::Verify { doc_id, .. } => Tag::Verify(*doc_id),
                        Work::Task(_) => Tag::Task,
                    };
                    let running = Running {
                        since: Instant::now(),
                        tag,
                        deadline: job.deadline(),
                        doc: job.doc_id(),
                    };
                    if let Some(slot) = q.workers.iter_mut().find(|w| w.id == id) {
                        slot.running = Some(running);
                    }
                    break job;
                }
                q = inner.work.wait(q).unwrap_or_else(|p| p.into_inner());
            }
        };
        run_job(inner, job, abandoned);
        if abandoned.load(Ordering::Acquire) {
            // Replaced while this job ran: its result was already reported as failed.
            return;
        }
        if let Some(slot) = inner.queue().workers.iter_mut().find(|w| w.id == id) {
            slot.running = None;
        }
    }
}

fn run_job(inner: &Inner, job: Work, abandoned: &AtomicBool) {
    match job {
        Work::Render { key, doc } => {
            let result = render::render_page(&doc, key.page, key.scale(), key.colors, key.tile);
            if abandoned.load(Ordering::Acquire) {
                return;
            }
            let mut st = inner.store();
            st.tick += 1;
            let used = st.tick;
            let (state, bytes) = match result {
                Ok(px) => {
                    let bytes = px.bytes();
                    (RenderState::Ready(Arc::new(px)), bytes)
                }
                Err(e) => (RenderState::Failed(e.to_string()), 0),
            };
            if let Some(old) = st.renders.insert(key, Item { state, used, bytes }) {
                st.bytes = st.bytes.saturating_sub(old.bytes);
            }
            st.bytes += bytes;
            st.evict();
            drop(st);
            inner.wake();
        }
        Work::Text {
            doc_id,
            page,
            doc,
            cache,
        } => {
            let t = text::page_text(&doc, page).unwrap_or_default();
            let links = doc.links(page).unwrap_or_default();
            if abandoned.load(Ordering::Acquire) {
                return;
            }
            cache.put_links(page, Arc::new(links));
            cache.put(page, Arc::new(t));
            inner.store().text_pending.remove(&(doc_id, page));
            inner.wake();
        }
        Work::Verify { doc_id, doc } => {
            let reports = verify::verify_all(&doc);
            if abandoned.load(Ordering::Acquire) {
                return;
            }
            inner
                .store()
                .verify
                .insert(doc_id, VerifyState::Done(Arc::new(reports)));
            inner.wake();
        }
        Work::Task(task) => {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
            inner.wake();
        }
    }
}

impl PdfService {
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.inner.waker.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(wake));
    }

    fn ensure_started(&self, q: &mut Queue) {
        if !q.started {
            q.started = true;
            for _ in 0..WORKERS {
                spawn_worker(&self.inner, q);
            }
        }
    }

    /// Whether `doc` stopped rendering after too many abandoned pages.
    pub fn too_complex(&self, doc: u64) -> bool {
        self.inner
            .store()
            .abandoned
            .get(&doc)
            .is_some_and(|n| *n >= MAX_ABANDONED)
    }

    /// The pages (and tiles) `pane` shows, most important first. Replaces what the pane
    /// asked for before; queued work for other panes is kept.
    pub fn want(&self, pane: PaneId, doc: &Arc<Document>, keys: &[RenderKey]) {
        let mut wanted: Vec<RenderKey> = Vec::new();
        {
            let mut st = self.inner.store();
            if keys.first().is_some_and(|k| {
                st.abandoned
                    .get(&k.doc)
                    .is_some_and(|n| *n >= MAX_ABANDONED)
            }) {
                return;
            }
            st.tick += 1;
            let tick = st.tick;
            for k in keys.iter().take(64) {
                match st.renders.get_mut(k) {
                    Some(item) => item.used = tick,
                    None => {
                        st.renders.insert(
                            *k,
                            Item {
                                state: RenderState::Pending,
                                used: tick,
                                bytes: 0,
                            },
                        );
                        wanted.push(*k);
                    }
                }
            }
        }
        let mut q = self.inner.queue();
        self.ensure_started(&mut q);
        // Drop this pane's queued renders that are no longer wanted.
        let mut dropped = Vec::new();
        q.renders.retain(|(p, w)| {
            if *p != pane {
                return true;
            }
            if let Work::Render { key, .. } = w
                && !keys.contains(key)
            {
                dropped.push(*key);
                return false;
            }
            true
        });
        for k in wanted {
            q.renders.push_back((
                pane,
                Work::Render {
                    key: k,
                    doc: doc.clone(),
                },
            ));
        }
        drop(q);
        if !dropped.is_empty() {
            let mut st = self.inner.store();
            for k in dropped {
                if matches!(
                    st.renders.get(&k).map(|i| &i.state),
                    Some(RenderState::Pending)
                ) {
                    st.renders.remove(&k);
                }
            }
        }
        self.inner.work.notify_all();
    }

    pub fn get(&self, key: &RenderKey) -> Option<RenderState> {
        let mut st = self.inner.store();
        st.tick += 1;
        let tick = st.tick;
        let item = st.renders.get_mut(key)?;
        item.used = tick;
        Some(item.state.clone())
    }

    /// Any whole-page rendering of `page` (another scale), to show stretched while the
    /// sharp one is pending.
    pub fn fallback(&self, doc: u64, page: usize, colors: ColorMode) -> Option<Arc<Pixels>> {
        let st = self.inner.store();
        st.renders
            .iter()
            .filter(|(k, _)| {
                k.doc == doc && k.page == page && k.colors == colors && k.tile.is_none()
            })
            .filter_map(|(k, i)| match &i.state {
                RenderState::Ready(px) => Some((k.step, px.clone())),
                _ => None,
            })
            .max_by_key(|(step, _)| *step)
            .map(|(_, px)| px)
    }

    /// Extract the text of `page` unless it is cached or on its way.
    pub fn want_text(&self, doc_id: u64, doc: &Arc<Document>, cache: &Arc<TextCache>, page: usize) {
        if cache.get(page).is_some() || page >= doc.page_count() {
            return;
        }
        {
            let mut st = self.inner.store();
            if st.text_failed.contains(&(doc_id, page)) || !st.text_pending.insert((doc_id, page)) {
                return;
            }
        }
        let mut q = self.inner.queue();
        self.ensure_started(&mut q);
        q.others.push_back(Work::Text {
            doc_id,
            page,
            doc: doc.clone(),
            cache: cache.clone(),
        });
        drop(q);
        self.inner.work.notify_all();
    }

    /// Whether the text of `page` is known: extracted, or given up on.
    pub fn text_done(&self, doc_id: u64, cache: &TextCache, page: usize) -> bool {
        cache.get(page).is_some() || self.inner.store().text_failed.contains(&(doc_id, page))
    }

    /// Whether text of `doc_id` is still being extracted.
    pub fn text_pending(&self, doc_id: u64) -> bool {
        self.inner
            .store()
            .text_pending
            .iter()
            .any(|(d, _)| *d == doc_id)
    }

    /// Verify the signatures of `doc` once.
    pub fn want_verify(&self, doc_id: u64, doc: &Arc<Document>) {
        {
            let mut st = self.inner.store();
            if st.verify.contains_key(&doc_id) {
                return;
            }
            st.verify.insert(doc_id, VerifyState::Running);
        }
        let mut q = self.inner.queue();
        self.ensure_started(&mut q);
        q.others.push_back(Work::Verify {
            doc_id,
            doc: doc.clone(),
        });
        drop(q);
        self.inner.work.notify_all();
    }

    pub fn verify_state(&self, doc_id: u64) -> Option<VerifyState> {
        self.inner.store().verify.get(&doc_id).cloned()
    }

    /// Run `task` on the pool (no deadline: the caller bounds it).
    pub fn run(&self, task: impl FnOnce() + Send + 'static) {
        let mut q = self.inner.queue();
        self.ensure_started(&mut q);
        q.others.push_back(Work::Task(Box::new(task)));
        drop(q);
        self.inner.work.notify_all();
    }

    /// Abandon jobs past their deadline and replace their workers. Called from `poll`;
    /// returns whether anything changed.
    pub fn supervise(&self) -> bool {
        let now = Instant::now();
        let mut expired: Vec<(Tag, Option<u64>)> = Vec::new();
        {
            let mut q = self.inner.queue();
            let mut replace = 0;
            for w in &mut q.workers {
                let late = w
                    .running
                    .as_ref()
                    .is_some_and(|r| r.deadline.is_some_and(|d| now.duration_since(r.since) > d));
                if late {
                    w.abandoned.store(true, Ordering::Release);
                    if let Some(r) = w.running.take() {
                        expired.push((r.tag, r.doc));
                    }
                    replace += 1;
                }
            }
            q.workers.retain(|w| !w.abandoned.load(Ordering::Acquire));
            for _ in 0..replace {
                spawn_worker(&self.inner, &mut q);
            }
            if replace > 0 {
                // Wake the abandoned workers' replacements and let idle ones re-check.
                self.inner.work.notify_all();
            }
        }
        if expired.is_empty() {
            return self.take_changed();
        }
        let mut st = self.inner.store();
        for (tag, doc) in expired {
            match tag {
                Tag::Render(key) => {
                    st.renders.insert(
                        key,
                        Item {
                            state: RenderState::Failed(
                                "This page took too long to draw and was skipped.".into(),
                            ),
                            used: 0,
                            bytes: 0,
                        },
                    );
                }
                Tag::Text(doc_id, page) => {
                    st.text_pending.remove(&(doc_id, page));
                    st.text_failed.insert((doc_id, page));
                }
                Tag::Verify(doc_id) => {
                    st.verify.insert(doc_id, VerifyState::TimedOut);
                }
                Tag::Task => {}
            }
            if let (Some(doc), Tag::Render(_) | Tag::Text(..)) = (doc, tag) {
                *st.abandoned.entry(doc).or_insert(0) += 1;
            }
        }
        drop(st);
        let _ = self.take_changed();
        true
    }

    pub fn take_changed(&self) -> bool {
        self.inner.changed.swap(false, Ordering::AcqRel)
    }

    /// Whether any job is queued or running.
    pub fn is_busy(&self) -> bool {
        let q = self.inner.queue();
        !q.renders.is_empty()
            || !q.others.is_empty()
            || q.workers.iter().any(|w| w.running.is_some())
    }

    /// Bytes of rendered pages kept (for tests and diagnostics).
    pub fn cached_bytes(&self) -> usize {
        self.inner.store().bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(pages: usize) -> Arc<Document> {
        Arc::new(Document::open(fm_pdf::fixtures::text_pages(pages).into(), None).expect("doc"))
    }

    fn wait(svc: &PdfService, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(20), "timed out");
            svc.supervise();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn renders_wanted_pages_and_falls_back() {
        let svc = PdfService::default();
        let d = doc(3);
        let key = |page, step| RenderKey {
            doc: 7,
            page,
            step,
            colors: ColorMode::Original,
            tile: None,
        };
        svc.want(1, &d, &[key(0, 2), key(1, 2)]);
        wait(&svc, || {
            matches!(svc.get(&key(1, 2)), Some(RenderState::Ready(_)))
        });
        let Some(RenderState::Ready(px)) = svc.get(&key(0, 2)) else {
            panic!("page 0");
        };
        assert_eq!(px.width, 298);
        assert!(svc.fallback(7, 0, ColorMode::Original).is_some());
        assert!(svc.fallback(7, 2, ColorMode::Original).is_none());
        assert!(svc.cached_bytes() > 0);
        // A newer request drops what is no longer visible.
        svc.want(1, &d, &[key(2, 4)]);
        wait(&svc, || {
            matches!(svc.get(&key(2, 4)), Some(RenderState::Ready(_)))
        });
    }

    #[test]
    fn text_and_verification() {
        let svc = PdfService::default();
        let d = doc(2);
        let cache = Arc::new(TextCache::default());
        svc.want_text(9, &d, &cache, 1);
        svc.want_text(9, &d, &cache, 99);
        wait(&svc, || cache.get(1).is_some());
        assert!(cache.get(1).is_some_and(|t| t.text.contains("Page 2")));
        assert!(!svc.text_pending(9));
        svc.want_verify(9, &d);
        wait(&svc, || {
            matches!(svc.verify_state(9), Some(VerifyState::Done(_)))
        });
    }

    #[test]
    fn step_rounding() {
        assert_eq!(RenderKey::step_for(1.0), 4);
        assert_eq!(RenderKey::step_for(1.01), 5);
        assert_eq!(RenderKey::step_for(0.01), 1);
        assert_eq!(RenderKey::step_for(f32::NAN), 4);
        assert_eq!(RenderKey::step_for(1000.0), 256);
    }
}
