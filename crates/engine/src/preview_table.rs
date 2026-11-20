//! A table preview while it is shown (change `add-table-preview`, design D3–D6): the
//! whole file opened in the background by threads the view owns, a cache of parsed rows,
//! and the filter/sort view job. Dropping the view (another file, pane closed) stops
//! every thread at its next check.
//!
//! The UI asks for the visible rows every frame ([`TableView::rows`]); present rows come
//! from the cache, missing ones become one "wanted" request that replaces the previous
//! one, served by the reader thread, which wakes the UI when they arrive.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use fm_preview::table::index::{self, Index};
use fm_preview::table::query::{Filter, SortKey, ViewError, ViewRow, ViewSpec, run_view};
use fm_preview::table::scan::MAX_FIELDS;
use fm_preview::table::source::{ByteSource, MemSource, VfsSource};
use fm_preview::table::{Encoding, MEMORY_BYTES, Row, TableDoc, transcode_utf16};
use fm_vfs::{Entry, Vfs};

/// Parsed rows kept per table.
pub const CACHE_ROWS: usize = 8192;
/// How often background threads re-check their cancel flag while idle.
const IDLE: Duration = Duration::from_millis(200);
/// Least time between two progress wake-ups of the UI.
const WAKE_EVERY: Duration = Duration::from_millis(100);

/// Wakes the UI (set by the app; a no-op in tests).
pub type Waker = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

/// Session-wide table state: the UI waker shared by every open table.
#[derive(Default)]
pub struct TableHub {
    waker: Waker,
}

impl std::fmt::Debug for TableHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TableHub")
    }
}

impl TableHub {
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut w) = self.waker.lock() {
            *w = Some(Arc::new(wake));
        }
    }

    pub(crate) fn waker(&self) -> Waker {
        self.waker.clone()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Rows the UI wants and does not have.
#[derive(Clone, Debug, PartialEq)]
enum Want {
    /// File order: `count` rows from `first`.
    Natural { first: u64, count: usize },
    /// A filtered or sorted view: these record offsets.
    Offsets(Vec<u64>),
}

#[derive(Default)]
struct RowCache {
    natural: HashMap<u64, Arc<Row>>,
    by_offset: HashMap<u64, Arc<Row>>,
    order: VecDeque<(Option<u64>, u64)>,
}

impl RowCache {
    fn put(&mut self, row: Option<u64>, offset: u64, value: Arc<Row>) {
        if let Some(r) = row {
            self.natural.insert(r, value.clone());
        }
        if self.by_offset.insert(offset, value).is_none() {
            self.order.push_back((row, offset));
        }
        while self.order.len() > CACHE_ROWS {
            if let Some((r, off)) = self.order.pop_front() {
                if let Some(r) = r {
                    self.natural.remove(&r);
                }
                self.by_offset.remove(&off);
            }
        }
    }
}

/// The view job's outcome so far.
#[derive(Default)]
struct JobState {
    /// The job being run: its revision and progress (0–1).
    running: Option<(u64, f32)>,
    /// The latest finished result and its revision.
    ready: Option<(u64, Arc<Vec<ViewRow>>)>,
    /// The latest failure and its revision.
    failed: Option<(u64, String)>,
}

struct Job {
    rev: u64,
    spec: ViewSpec,
}

struct Shared {
    doc: Arc<TableDoc>,
    source: Mutex<Option<Arc<dyn ByteSource>>>,
    failure: Mutex<Option<String>>,
    note: Mutex<Option<String>>,
    index: RwLock<Index>,
    cache: Mutex<RowCache>,
    want: Mutex<Option<Want>>,
    want_cv: Condvar,
    job: Mutex<Option<Job>>,
    job_cv: Condvar,
    state: Mutex<JobState>,
    cancel: AtomicBool,
    /// Jobs started; a stop check compares with the job's own revision.
    newest_job: AtomicU64,
    waker: Waker,
    last_wake: Mutex<Instant>,
    /// Files up to this size are read into memory (tests lower it).
    memory_cap: u64,
}

impl Shared {
    fn wake(&self) {
        if let Ok(mut t) = self.last_wake.lock() {
            *t = Instant::now();
        }
        let w = lock(&self.waker).clone();
        if let Some(w) = w {
            w();
        }
    }

    /// Wake at most every [`WAKE_EVERY`] (progress reports).
    fn wake_throttled(&self) {
        let due = self
            .last_wake
            .lock()
            .map(|t| t.elapsed() >= WAKE_EVERY)
            .unwrap_or(true);
        if due {
            self.wake();
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn source(&self) -> Option<Arc<dyn ByteSource>> {
        lock(&self.source).clone()
    }

    /// Wait for the source (or a failure, or cancel).
    fn wait_source(&self) -> Option<Arc<dyn ByteSource>> {
        loop {
            if self.cancelled() || lock(&self.failure).is_some() {
                return None;
            }
            if let Some(s) = self.source() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// How the view job stands, for the toolbar.
#[derive(Clone, Debug, PartialEq)]
pub enum ViewStatus {
    /// No filter or sort: file order.
    Natural,
    /// Filtering/sorting; `progress` 0–1.
    Running {
        progress: f32,
    },
    Ready,
    Failed(String),
}

/// A table preview with the user's view of it.
pub struct TableView {
    pub doc: Arc<TableDoc>,
    pub sort: Vec<SortKey>,
    pub filter: Filter,
    /// Selected cell: view row and column.
    pub selected: Option<(u64, usize)>,
    pub hidden: BTreeSet<usize>,
    /// The Record panel is shown.
    pub record: bool,
    /// A one-shot scroll request: view row and a sequence number the UI compares.
    pub scroll_to: Option<(u64, u64)>,
    scroll_seq: u64,
    rev: u64,
    sample: Arc<Vec<Arc<Row>>>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for TableView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableView")
            .field("columns", &self.doc.columns.len())
            .field("sort", &self.sort)
            .field("filter", &self.filter)
            .field("selected", &self.selected)
            .finish()
    }
}

impl Drop for TableView {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
        self.shared.want_cv.notify_all();
        self.shared.job_cv.notify_all();
    }
}

impl TableView {
    /// Open `doc` (the rendered first rows of `entry`) and start reading the whole file.
    pub(crate) fn open(vfs: Arc<dyn Vfs>, entry: Entry, doc: TableDoc, waker: Waker) -> Self {
        Self::open_capped(vfs, entry, doc, waker, MEMORY_BYTES)
    }

    pub(crate) fn open_capped(
        vfs: Arc<dyn Vfs>,
        entry: Entry,
        doc: TableDoc,
        waker: Waker,
        memory_cap: u64,
    ) -> Self {
        let doc = Arc::new(doc);
        let shared = Arc::new(Shared {
            doc: doc.clone(),
            source: Mutex::new(None),
            failure: Mutex::new(None),
            note: Mutex::new(None),
            index: RwLock::new(Index::default()),
            cache: Mutex::new(RowCache::default()),
            want: Mutex::new(None),
            want_cv: Condvar::new(),
            job: Mutex::new(None),
            job_cv: Condvar::new(),
            state: Mutex::new(JobState::default()),
            cancel: AtomicBool::new(false),
            newest_job: AtomicU64::new(0),
            waker,
            last_wake: Mutex::new(Instant::now()),
            memory_cap,
        });
        let sample = Arc::new(doc.sample.iter().cloned().map(Arc::new).collect());
        let view = Self {
            doc,
            sort: Vec::new(),
            filter: Filter::default(),
            selected: None,
            hidden: BTreeSet::new(),
            record: false,
            scroll_to: None,
            scroll_seq: 0,
            rev: 0,
            sample,
            shared: shared.clone(),
        };
        let spawn = |name: &str, f: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    // A panic in a parser must not take the app down; the view then
                    // shows an error.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
                })
                .is_ok()
        };
        let s1 = shared.clone();
        let ok1 = spawn(
            "fm-table-index",
            Box::new(move || index_thread(&s1, vfs, entry)),
        );
        let s2 = shared.clone();
        let ok2 = spawn("fm-table-rows", Box::new(move || reader_thread(&s2)));
        let s3 = shared.clone();
        let ok3 = spawn("fm-table-view", Box::new(move || job_thread(&s3)));
        if !(ok1 && ok2 && ok3) {
            *lock(&shared.failure) = Some("could not start reading the table".into());
        }
        view
    }

    /// Column count: the header's, or more when later records have more fields.
    pub fn columns(&self) -> usize {
        let max_fields = self.shared.index.read().map_or(0, |i| i.max_fields);
        self.doc.columns.len().max(max_fields).min(MAX_FIELDS)
    }

    /// Records in the file so far (all of them once indexing is done).
    pub fn total_rows(&self) -> u64 {
        let (rows, done) = self
            .shared
            .index
            .read()
            .map_or((0, false), |i| (i.rows, i.done));
        if done {
            rows
        } else {
            rows.max(self.sample.len() as u64)
        }
    }

    /// Indexing progress 0–1, or `None` once done.
    pub fn indexing(&self) -> Option<f32> {
        let idx = self.shared.index.read().ok()?;
        if idx.done {
            return None;
        }
        let len = self.source_len().unwrap_or(self.doc.size).max(1);
        Some((idx.scanned as f64 / len as f64).clamp(0.0, 1.0) as f32)
    }

    fn source_len(&self) -> Option<u64> {
        self.shared.source().map(|s| s.len())
    }

    pub fn malformed(&self) -> u64 {
        let idx = self.shared.index.read().map_or(0, |i| i.malformed);
        idx.max(self.doc.sample_malformed)
    }

    /// Why the file could not be read, or a note about how it was read.
    pub fn failure(&self) -> Option<String> {
        lock(&self.shared.failure).clone()
    }

    pub fn note(&self) -> Option<String> {
        lock(&self.shared.note).clone()
    }

    fn wants_view(&self) -> bool {
        !self.filter.is_empty() || !self.sort.is_empty()
    }

    /// The rows of the filtered/sorted view, when one applies. While a newer job runs
    /// (or after it failed) the previous result stays shown.
    pub fn view_rows(&self) -> Option<Arc<Vec<ViewRow>>> {
        if !self.wants_view() {
            return None;
        }
        lock(&self.shared.state)
            .ready
            .as_ref()
            .map(|(_, r)| r.clone())
    }

    pub fn view_status(&self) -> ViewStatus {
        if !self.wants_view() {
            return ViewStatus::Natural;
        }
        let st = lock(&self.shared.state);
        if let Some((rev, progress)) = st.running
            && rev == self.rev
        {
            return ViewStatus::Running { progress };
        }
        if let Some((rev, why)) = &st.failed
            && *rev == self.rev
        {
            return ViewStatus::Failed(why.clone());
        }
        match &st.ready {
            Some((rev, _)) if *rev == self.rev => ViewStatus::Ready,
            _ => ViewStatus::Running { progress: 0.0 },
        }
    }

    /// Rows shown: the view's when one applies, else the file's.
    pub fn row_count(&self) -> u64 {
        match self.view_rows() {
            Some(v) => v.len() as u64,
            None => self.total_rows(),
        }
    }

    /// Visible rows `first..first+count` as `(row number in the file, row)`; `None` for
    /// rows still being read (they are requested).
    pub fn rows(&self, first: u64, count: usize) -> Vec<Option<(u64, Arc<Row>)>> {
        let count = count.min(1024);
        let mut out = Vec::with_capacity(count);
        match self.view_rows() {
            Some(view) => {
                let mut missing = Vec::new();
                {
                    let cache = lock(&self.shared.cache);
                    for i in first..first.saturating_add(count as u64) {
                        let Some(&(off, row)) = view.get(i as usize) else {
                            break;
                        };
                        match cache.by_offset.get(&off) {
                            Some(r) => out.push(Some((row, r.clone()))),
                            None => {
                                missing.push(off);
                                out.push(None);
                            }
                        }
                    }
                }
                if !missing.is_empty() {
                    self.request(Want::Offsets(missing));
                }
            }
            None => {
                let total = self.total_rows();
                let mut missing: Option<(u64, u64)> = None;
                {
                    let cache = lock(&self.shared.cache);
                    for r in first..first.saturating_add(count as u64).min(total) {
                        if let Some(row) = self.sample.get(r as usize) {
                            out.push(Some((r, row.clone())));
                        } else if let Some(row) = cache.natural.get(&r) {
                            out.push(Some((r, row.clone())));
                        } else {
                            out.push(None);
                            missing = Some(missing.map_or((r, r), |(a, _)| (a, r)));
                        }
                    }
                }
                if let Some((a, b)) = missing {
                    self.request(Want::Natural {
                        first: a,
                        count: (b - a + 1) as usize,
                    });
                }
            }
        }
        out
    }

    /// The row at view position `row`, if loaded.
    pub fn row(&self, row: u64) -> Option<(u64, Arc<Row>)> {
        self.rows(row, 1).into_iter().next().flatten()
    }

    fn request(&self, want: Want) {
        let mut w = lock(&self.shared.want);
        if w.as_ref() != Some(&want) {
            *w = Some(want);
            self.shared.want_cv.notify_all();
        }
    }

    /// Start (or clear) the view job for the current filter and sort.
    pub(crate) fn refresh_view(&mut self) {
        self.rev += 1;
        self.selected = None;
        if !self.wants_view() {
            *lock(&self.shared.state) = JobState::default();
            *lock(&self.shared.job) = None;
            self.shared
                .newest_job
                .store(self.rev, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        let spec = ViewSpec {
            filter: self.filter.clone(),
            sort: self.sort.clone(),
            numeric: self.doc.numeric_columns(),
            decimal_comma: self.doc.decimal_comma,
        };
        self.shared.newest_job.store(self.rev, Ordering::Relaxed);
        lock(&self.shared.state).running = Some((self.rev, 0.0));
        *lock(&self.shared.job) = Some(Job {
            rev: self.rev,
            spec,
        });
        self.shared.job_cv.notify_all();
    }

    /// Ask the UI to scroll `row` into view.
    pub(crate) fn scroll_to(&mut self, row: u64) {
        self.scroll_seq += 1;
        self.scroll_to = Some((row, self.scroll_seq));
    }

    /// Whether the threads' shared state is still alive (tests check the threads end).
    #[cfg(test)]
    pub(crate) fn alive_probe(&self) -> impl Fn() -> bool + use<> {
        let weak = Arc::downgrade(&self.shared);
        move || weak.upgrade().is_some()
    }

    /// Whether any background work is still running (for repaint scheduling and tests).
    pub fn is_busy(&self) -> bool {
        self.indexing().is_some()
            || matches!(self.view_status(), ViewStatus::Running { .. })
            || lock(&self.shared.want).is_some()
    }
}

fn read_capped(vfs: &dyn Vfs, entry: &Entry, cap: u64) -> Result<(Vec<u8>, bool), String> {
    let mut buf = Vec::new();
    vfs.open_read(&entry.uri)
        .map_err(|e| e.to_string())?
        .take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    let more = buf.len() as u64 > cap;
    buf.truncate(cap as usize);
    Ok((buf, more))
}

fn load_source(
    shared: &Shared,
    vfs: Arc<dyn Vfs>,
    entry: &Entry,
) -> Result<Arc<dyn ByteSource>, String> {
    let doc = &shared.doc;
    let cap = shared.memory_cap;
    let truncated = |shared: &Shared| {
        *lock(&shared.note) = Some(format!(
            "showing the first {} MiB of this file",
            (cap >> 20).max(1)
        ));
    };
    match doc.encoding {
        Encoding::Utf8 => match entry.size {
            Some(size) if size > cap => Ok(Arc::new(VfsSource::new(vfs, entry.uri.clone(), size))),
            _ => {
                let (bytes, more) = read_capped(&*vfs, entry, cap)?;
                if more {
                    truncated(shared);
                }
                Ok(Arc::new(MemSource(Arc::from(bytes))))
            }
        },
        enc => {
            let (raw, more) = read_capped(&*vfs, entry, cap)?;
            if more {
                truncated(shared);
            }
            let body = raw.get(2..).unwrap_or(&[]);
            Ok(Arc::new(MemSource(Arc::from(transcode_utf16(body, enc)))))
        }
    }
}

fn index_thread(shared: &Shared, vfs: Arc<dyn Vfs>, entry: Entry) {
    let src = match load_source(shared, vfs, &entry) {
        Ok(s) => s,
        Err(e) => {
            *lock(&shared.failure) = Some(e);
            shared.wake();
            return;
        }
    };
    if shared.cancelled() {
        return;
    }
    *lock(&shared.source) = Some(src.clone());
    shared.want_cv.notify_all();
    shared.job_cv.notify_all();
    let result = index::build(
        &*src,
        shared.doc.dialect,
        shared.doc.data_start,
        &shared.cancel,
        |delta| {
            let done = delta.done;
            if let Ok(mut idx) = shared.index.write() {
                idx.apply(delta);
            }
            // Rows waiting for the index can be served now.
            shared.want_cv.notify_all();
            if done {
                shared.wake();
            } else {
                shared.wake_throttled();
            }
        },
    );
    if let Err(e) = result {
        *lock(&shared.failure) = Some(e.to_string());
        if let Ok(mut idx) = shared.index.write() {
            idx.done = true;
        }
        shared.wake();
    }
}

fn reader_thread(shared: &Shared) {
    let Some(src) = shared.wait_source() else {
        return;
    };
    loop {
        let want = {
            let mut w = lock(&shared.want);
            loop {
                if shared.cancelled() {
                    return;
                }
                if let Some(want) = w.take() {
                    break want;
                }
                w = match shared.want_cv.wait_timeout(w, IDLE) {
                    Ok((g, _)) => g,
                    Err(p) => p.into_inner().0,
                };
            }
        };
        let d = shared.doc.dialect;
        let served = match want {
            Want::Offsets(offsets) => {
                let mut any = false;
                for off in offsets.into_iter().take(1024) {
                    if shared.cancelled() || lock(&shared.want).is_some() {
                        break;
                    }
                    if lock(&shared.cache).by_offset.contains_key(&off) {
                        continue;
                    }
                    if let Ok(rows) = index::read_rows(&*src, d, off, 0, 1)
                        && let Some((at, row)) = rows.into_iter().next()
                    {
                        lock(&shared.cache).put(None, at, Arc::new(row));
                        any = true;
                    }
                }
                any
            }
            Want::Natural { first, count } => {
                let located = shared.index.read().ok().and_then(|i| i.locate(first));
                match located {
                    Some((off, skip)) => match index::read_rows(&*src, d, off, skip, count) {
                        Ok(rows) => {
                            let mut cache = lock(&shared.cache);
                            for (k, (at, row)) in rows.into_iter().enumerate() {
                                cache.put(Some(first + k as u64), at, Arc::new(row));
                            }
                            true
                        }
                        Err(e) => {
                            *lock(&shared.failure) = Some(e.to_string());
                            true
                        }
                    },
                    None => {
                        // Not indexed yet: ask again once the index grows (the indexer
                        // notifies), unless a newer request came in.
                        let done = shared.index.read().map_or(true, |i| i.done);
                        if !done {
                            let mut w = lock(&shared.want);
                            if w.is_none() {
                                *w = Some(Want::Natural { first, count });
                            }
                            drop(w);
                            std::thread::sleep(Duration::from_millis(30));
                        }
                        false
                    }
                }
            }
        };
        if served {
            shared.wake();
        }
    }
}

fn job_thread(shared: &Shared) {
    let Some(src) = shared.wait_source() else {
        return;
    };
    loop {
        let job = {
            let mut j = lock(&shared.job);
            loop {
                if shared.cancelled() {
                    return;
                }
                if let Some(job) = j.take() {
                    break job;
                }
                j = match shared.job_cv.wait_timeout(j, IDLE) {
                    Ok((g, _)) => g,
                    Err(p) => p.into_inner().0,
                };
            }
        };
        let rev = job.rev;
        let len = src.len().max(1);
        let stop = || shared.cancelled() || shared.newest_job.load(Ordering::Relaxed) != rev;
        let progress = |at: u64| {
            let mut st = lock(&shared.state);
            if let Some((r, p)) = &mut st.running
                && *r == rev
            {
                *p = (at as f64 / len as f64).clamp(0.0, 1.0) as f32;
            }
            drop(st);
            shared.wake_throttled();
        };
        let result = run_view(
            &*src,
            shared.doc.dialect,
            shared.doc.data_start,
            &job.spec,
            &stop,
            &progress,
        );
        let mut st = lock(&shared.state);
        match result {
            Err(ViewError::Cancelled) => continue,
            Ok(rows) => {
                st.ready = Some((rev, Arc::new(rows)));
                st.failed = None;
            }
            Err(e) => st.failed = Some((rev, e.to_string())),
        }
        if st.running.is_some_and(|(r, _)| r == rev) {
            st.running = None;
        }
        drop(st);
        shared.wake();
    }
}

#[cfg(test)]
pub(crate) mod tests;
