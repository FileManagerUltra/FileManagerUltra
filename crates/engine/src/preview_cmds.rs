//! Preview panes (capabilities `preview-pane`, `pane-types`): which file a preview shows,
//! the off-thread rendering, log follow/filter jobs and the `preview.*` commands.
//!
//! The UI reads [`PreviewSlot`]s and dispatches commands; every read of file content runs
//! on a job lane and reports back through the hub's inbox, drained by `Session::poll`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use fm_layout::{Axis, PaneId, PaneKind, PreviewSource};
use fm_preview::log::{
    self, FollowUpdate, Level, LogDoc, LogFilter, LogLimits, apply_filter, parse_bound,
};
use fm_preview::{Limits, Preview};
use fm_vfs::{Entry, EntryKind, FmError, Vfs, VfsUri};

use crate::backends::lane_for;
use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Selection changes settle this long before a preview is rendered.
pub const DEBOUNCE: Duration = Duration::from_millis(120);
/// Follow re-checks the file this often.
/// A render that takes longer is abandoned with an error card.
pub const RENDER_BUDGET: Duration = Duration::from_secs(15);

/// Sets its flag on drop, so a replaced or removed slot stops its scan thread.
#[derive(Debug, Default)]
pub(crate) struct ScanCancel(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ScanCancel {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Zoom {
    Fit,
    Actual,
    Scale(f32),
}

/// A parsed log plus the user's view of it.
#[derive(Debug)]
pub struct LogView {
    pub doc: Arc<RwLock<LogDoc>>,
    pub filter: LogFilter,
    /// Indices passing the filter; `None` shows every record.
    pub filtered: Option<Arc<Vec<u32>>>,
    pub filter_error: Option<String>,
    pub follow: bool,
    pub columns: bool,
    pub expand_all: bool,
    pub expanded: HashSet<u64>,
    /// Bumped whenever the document changed (append, earlier chunk, reload).
    pub rev: u64,
    /// Multi-file source ranges in the concatenated document.
    pub sources: Vec<crate::preview_multi::SourceRange>,
    pub chronological: bool,
    pub window_bytes: u64,
    busy: bool,
    next_follow: Instant,
    filter_rev: u64,
}

impl LogView {
    pub fn record_source(&self, index: usize) -> Option<&crate::preview_multi::SourceRange> {
        let source = self.sources.get(
            self.sources
                .partition_point(|source| source.records.end <= index),
        )?;
        source.records.contains(&index).then_some(source)
    }

    pub fn record_key(&self, index: usize, record: &log::LogRecord) -> u64 {
        use std::hash::{Hash, Hasher};
        let Some(source) = self.record_source(index) else {
            return record.key();
        };
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        source.entry.uri.hash(&mut hash);
        record.key().hash(&mut hash);
        hash.finish()
    }

    pub(crate) fn new(doc: LogDoc) -> Self {
        Self {
            doc: Arc::new(RwLock::new(doc)),
            filter: LogFilter::default(),
            filtered: None,
            filter_error: None,
            follow: false,
            columns: true,
            expand_all: false,
            expanded: HashSet::new(),
            rev: 0,
            sources: Vec::new(),
            chronological: false,
            window_bytes: LogLimits::default().initial_tail,
            busy: false,
            next_follow: Instant::now(),
            filter_rev: 0,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn reschedule_follow(&mut self) {
        self.next_follow = Instant::now();
    }
}

#[derive(Debug)]
pub enum Body {
    Empty,
    Loading,
    Static(Arc<Preview>),
    Log(Box<LogView>),
    Grid(Vec<(Entry, Arc<Preview>)>),
}

type Key = (VfsUri, Option<SystemTime>, Option<u64>, bool, bool);

fn key_of(e: &Entry, recursive: bool, show_hidden: bool) -> Key {
    (e.uri.clone(), e.modified, e.size, recursive, show_hidden)
}

#[derive(Debug)]
pub struct PreviewSlot {
    /// What the body is about.
    pub subject: Option<Entry>,
    pub subjects: Vec<Entry>,
    pub body: Body,
    pub pretty: bool,
    /// Show a Markdown file's text instead of its rendered document.
    pub source: bool,
    pub wrap: bool,
    pub zoom: Zoom,
    /// Magnification for this pane's content, separate from image fit mode and UI zoom.
    pub scale: f32,
    pub find: crate::preview_find_cmds::Find,
    /// Directory thumbnail grid instead of the summary card.
    pub thumbs: bool,
    /// Index into [`fm_preview::thumbs::EDGES`].
    pub thumb_size: usize,
    /// Order of the thumbnail grid.
    pub thumb_sort: fm_preview::arrange::ThumbSort,
    /// Recursive grids: headings per subfolder instead of one flat list.
    pub thumb_group: bool,
    /// Entries the grid last asked for: `(first, count)`.
    pub thumb_window: Option<(usize, usize)>,
    /// Playback of a video shown by this pane.
    pub video: Option<crate::video_cmds::VideoState>,
    pub audio: Option<crate::preview_audio_cmds::AudioState>,
    /// The Files pane being previewed (for the header).
    pub target: Option<PaneId>,
    /// Something changed inside the previewed folder; re-render it in place.
    pub stale: bool,
    /// Multi-file content changed; keep the loaded view until the user reloads.
    pub updates_available: bool,
    token: u64,
    key: Option<Vec<Key>>,
    /// Stops a running recursive folder scan when the slot moves on or is dropped.
    scan: Option<ScanCancel>,
    pending: Option<(Vec<Entry>, Instant)>,
    multi_window: u64,
    saved_log: Option<(LogFilter, bool, bool, bool)>,
    source_sort: Option<fm_layout::SortSpec>,
}

impl PreviewSlot {
    pub(crate) fn playback_ready(&self) -> bool {
        self.pending.is_none()
    }

    pub(crate) fn new() -> Self {
        Self {
            subject: None,
            subjects: Vec::new(),
            body: Body::Empty,
            pretty: false,
            source: false,
            wrap: false,
            zoom: Zoom::Fit,
            scale: 1.0,
            find: Default::default(),
            thumbs: false,
            thumb_size: 0,
            thumb_sort: Default::default(),
            thumb_group: true,
            thumb_window: None,
            video: None,
            audio: None,
            target: None,
            stale: false,
            updates_available: false,
            token: 0,
            key: None,
            scan: None,
            pending: None,
            multi_window: LogLimits::default().initial_tail,
            saved_log: None,
            source_sort: None,
        }
    }

    pub fn log(&self) -> Option<&LogView> {
        match &self.body {
            Body::Log(l) => Some(l),
            _ => None,
        }
    }

    pub(crate) fn log_mut(&mut self) -> Option<&mut LogView> {
        match &mut self.body {
            Body::Log(l) => Some(l),
            _ => None,
        }
    }

    fn busy(&self) -> bool {
        self.pending.is_some()
            || matches!(self.body, Body::Loading)
            || self.log().is_some_and(|l| l.busy || l.follow)
    }
}

#[derive(Debug)]
enum Msg {
    BatchRendered {
        pane: PaneId,
        token: u64,
        body: Body,
    },
    Rendered {
        pane: PaneId,
        token: u64,
        /// Boxed: a preview is much larger than the other messages.
        preview: Box<Preview>,
    },
    LogDone {
        pane: PaneId,
        token: u64,
        changed: bool,
    },
    Filtered {
        pane: PaneId,
        token: u64,
        rev: u64,
        result: Result<Vec<u32>, String>,
    },
}

#[derive(Debug, Default)]
pub struct PreviewHub {
    pub slots: HashMap<PaneId, PreviewSlot>,
    inbox: Arc<Mutex<Vec<Msg>>>,
    next_token: u64,
}

impl PreviewHub {
    pub fn slot(&self, pane: PaneId) -> Option<&PreviewSlot> {
        self.slots.get(&pane)
    }

    pub fn is_busy(&self) -> bool {
        self.slots.values().any(PreviewSlot::busy)
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    fn push(inbox: &Arc<Mutex<Vec<Msg>>>, msg: Msg) {
        if let Ok(mut g) = inbox.lock() {
            g.push(msg);
        }
    }
}

/// Run `work` on its own thread; give up after `budget` so a hostile file cannot hold
/// the lane (the abandoned thread ends when its work does).
pub fn run_budgeted<T: Send + 'static>(
    budget: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("fm-preview".into())
        .spawn(move || {
            let _ = tx.send(work());
        })
        .ok()?;
    rx.recv_timeout(budget).ok()
}

impl Session {
    fn preview_entries(&mut self, pane: PaneId) -> Vec<Entry> {
        let Some(state) = self.layout.state(pane) else {
            return Vec::new();
        };
        if state.listing.selected_count() > 0
            && let Some((entries, ix)) = self.pane_view(pane)
            && let Some(state) = self.layout.state(pane)
        {
            return ix
                .order
                .iter()
                .filter_map(|i| entries.get(*i as usize))
                .filter(|e| state.listing.is_selected(&e.name))
                .take(crate::preview_multi::MAX_SOURCES + 1)
                .cloned()
                .collect();
        }
        self.cursor_entry(pane).into_iter().collect()
    }

    /// The entry under the cursor of Files pane `pane` (`None` on `..` or when empty).
    /// On `.` this is the pane's own folder, so a following Preview shows the folder.
    fn cursor_entry(&mut self, pane: PaneId) -> Option<Entry> {
        let (on_current, on_parent, row, location) = {
            let state = self.layout.state(pane)?;
            (
                state.listing.on_current,
                state.listing.on_parent,
                state.listing.focused_row,
                state.location.clone(),
            )
        };
        if on_parent {
            return None;
        }
        if on_current {
            let mut entry = Entry::new(location, EntryKind::Dir);
            entry.name = ".".to_string();
            return Some(entry);
        }
        let (entries, ix) = self.pane_view(pane)?;
        let i = *ix.order.get(row)?;
        entries.get(i as usize).cloned()
    }

    /// Follow selections, drop slots of vanished panes, and start due jobs. Called from
    /// `poll`; returns whether anything observable changed.
    pub(crate) fn refresh_previews(&mut self) -> bool {
        self.layout.touch_files_focus();
        let previews: Vec<PaneId> = self
            .layout
            .states
            .iter()
            .filter(|(_, s)| s.kind == PaneKind::Preview)
            .map(|(id, _)| *id)
            .collect();
        let mut changed = false;
        let before = self.previews.slots.len();
        let parked = crate::workspace::background_panes(&self.workspaces);
        self.previews
            .slots
            .retain(|id, _| previews.contains(id) || parked.contains(id));
        changed |= before != self.previews.slots.len();
        let now = Instant::now();
        for pane in previews {
            let target = self.layout.preview_target(pane);
            if target.is_some_and(|target| !self.pane_view_ready(target))
                && self
                    .previews
                    .slots
                    .get(&pane)
                    .is_some_and(|slot| slot.subjects.len() > 1 && slot.target == target)
            {
                continue;
            }
            let desired = target.map(|t| self.preview_entries(t)).unwrap_or_default();
            let slot = self
                .previews
                .slots
                .entry(pane)
                .or_insert_with(PreviewSlot::new);
            slot.target = target;
            changed |= self.advance_slot(pane, desired, now);
        }
        changed |= self.drain_previews();
        changed |= self.thumbs.take_changed();
        changed |= self.video_tick();
        changed |= self.audio_preview_tick();
        self.reconcile_audio_owner();
        changed
    }

    /// `dir` changed on disk: folder previews showing it (or a folder above it, when
    /// recursive) re-render in place without flashing a spinner.
    pub(crate) fn dir_changed(&mut self, dir: &VfsUri) {
        for slot in self.previews.slots.values_mut() {
            let Some(subject) = slot.subject.as_ref().filter(|e| e.is_dir_like()) else {
                continue;
            };
            let mut at = Some(dir.clone());
            while let Some(d) = at {
                if d == subject.uri {
                    slot.stale = true;
                    break;
                }
                at = d.parent();
            }
        }
    }

    fn advance_slot(&mut self, pane: PaneId, desired: Vec<Entry>, now: Instant) -> bool {
        let mut changed = false;
        let recursive = self.prefs.recursive_preview;
        let show_hidden = self.show_hidden;
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return false;
        };
        match desired.as_slice() {
            [] => {
                if slot.key.is_some() || slot.pending.is_some() {
                    slot.key = None;
                    slot.scan = None;
                    slot.pending = None;
                    slot.subject = None;
                    slot.subjects.clear();
                    slot.body = Body::Empty;
                    slot.pretty = false;
                    slot.source = false;
                    slot.thumb_window = None;
                    slot.token = 0;
                    slot.saved_log = None;
                    slot.updates_available = false;
                    changed = true;
                }
            }
            entries => {
                let keys: Vec<_> = entries
                    .iter()
                    .map(|e| key_of(e, recursive, show_hidden))
                    .collect();
                let same_subject = slot
                    .subjects
                    .iter()
                    .map(|e| &e.uri)
                    .eq(entries.iter().map(|e| &e.uri));
                let same_log = entries.len() == 1 && slot.log().is_some() && same_subject;
                let source_sort = slot
                    .target
                    .and_then(|target| self.layout.state(target))
                    .map(|state| state.listing.sort.clone());
                let same_selection = slot.subjects.len() == entries.len()
                    && slot
                        .subjects
                        .iter()
                        .all(|source| entries.iter().any(|entry| entry.uri == source.uri));
                let keep_multi = entries.len() > 1
                    && same_selection
                    && source_sort == slot.source_sort
                    && (matches!(slot.body, Body::Log(_) | Body::Grid(_))
                        || (matches!(slot.body, Body::Loading) && slot.token != 0));
                if slot.stale && same_subject {
                    if slot.pending.is_none() {
                        slot.pending = Some((desired, now));
                    }
                } else if keep_multi {
                    if slot.key.as_ref() != Some(&keys) && !slot.updates_available {
                        slot.updates_available = true;
                        changed = true;
                    }
                    slot.pending = None;
                } else if (slot.key.as_ref() == Some(&keys)
                    && (slot.token != 0 || !matches!(slot.body, Body::Loading)))
                    || same_log
                {
                    slot.pending = None;
                } else if slot.pending.as_ref().map(|(p, _)| {
                    p.iter()
                        .map(|e| key_of(e, recursive, show_hidden))
                        .collect::<Vec<_>>()
                }) != Some(keys)
                {
                    slot.pending = Some((desired, now + DEBOUNCE));
                    // The cursor moved on: stop the old folder scan right away.
                    slot.scan = None;
                    slot.audio = None;
                    slot.video = None;
                    if matches!(slot.body, Body::Loading) {
                        slot.token = 0;
                    }
                }
            }
        }
        let due = self
            .previews
            .slots
            .get(&pane)
            .and_then(|s| s.pending.as_ref())
            .is_some_and(|(_, at)| *at <= now);
        if due {
            changed |= self.start_render(pane);
        }
        changed |= self.tick_log(pane, now);
        changed
    }

    fn start_render(&mut self, pane: PaneId) -> bool {
        let token = self.previews.token();
        let recursive = self.prefs.recursive_preview;
        let show_hidden = self.show_hidden;
        let log_limits = self.prefs.log_limits();
        let filters = self.prefs.preview_filters.clone();
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return false;
        };
        let Some((entries, _)) = slot.pending.take() else {
            return false;
        };
        let Some(entry) = entries.first().cloned() else {
            return false;
        };
        let refresh = slot.stale
            && slot.subjects.len() == entries.len()
            && slot
                .subjects
                .iter()
                .all(|source| entries.iter().any(|entry| entry.uri == source.uri));
        if let Some(view) = slot.log() {
            slot.saved_log = Some((
                view.filter.clone(),
                view.chronological,
                view.expand_all,
                view.columns,
            ));
        }
        slot.stale = false;
        slot.updates_available = false;
        slot.source_sort = slot
            .target
            .and_then(|target| self.layout.state(target))
            .map(|state| state.listing.sort.clone());
        if slot.subject.as_ref().is_none_or(|s| s.uri != entry.uri) {
            slot.pretty = false;
            slot.source = false;
            slot.thumb_window = None;
            self.thumbs.cancel(pane);
        }
        slot.token = token;
        slot.scan = None;
        if slot
            .subjects
            .iter()
            .map(|e| &e.uri)
            .ne(entries.iter().map(|e| &e.uri))
        {
            slot.multi_window = log_limits
                .initial_tail
                .min(crate::preview_multi::MAX_WINDOW);
        }
        slot.key = Some(
            entries
                .iter()
                .map(|e| key_of(e, recursive, show_hidden))
                .collect(),
        );
        slot.subjects = entries.clone();
        slot.subject = Some(entry.clone());
        if !refresh {
            slot.body = Body::Loading;
        }
        if entries.iter().any(|source| {
            source.uri.scheme() != entry.uri.scheme()
                || source.uri.authority() != entry.uri.authority()
        }) {
            slot.body = Body::Static(Arc::new(Preview::Error(
                "Select files from a single filesystem or server for a combined preview.".into(),
            )));
            return true;
        }
        let vfs = match self.backends.require(&entry.uri) {
            Ok(v) => v,
            Err(e) => {
                slot.body = Body::Static(Arc::new(Preview::Error(e.to_string())));
                return true;
            }
        };
        let inbox = self.previews.inbox.clone();
        let budget = self.prefs.preview_budget();
        if entries.len() > 1 {
            let window = slot.multi_window;
            if entries.len() > crate::preview_multi::MAX_SOURCES {
                slot.body = Body::Static(Arc::new(Preview::Error(format!(
                    "Select at most {} files for a combined preview.",
                    crate::preview_multi::MAX_SOURCES
                ))));
                return true;
            }
            let cancel = ScanCancel::default();
            let flag = cancel.0.clone();
            slot.scan = Some(cancel);
            self.jobs.submit(lane_for(entry.uri.scheme()), move |_ctx| {
                let body = crate::preview_multi::render(
                    vfs,
                    entries,
                    window,
                    flag.clone(),
                    filters,
                    budget,
                );
                if !flag.load(std::sync::atomic::Ordering::Relaxed) {
                    PreviewHub::push(&inbox, Msg::BatchRendered { pane, token, body });
                }
                Ok(())
            });
            return true;
        }
        if recursive && entry.is_dir_like() && entry.uri.scheme() == fm_vfs::Scheme::File {
            // A full scan has no time budget, so it gets its own thread instead of holding a job lane.
            let cancel = ScanCancel::default();
            let flag = cancel.0.clone();
            slot.scan = Some(cancel);
            let spawned = std::thread::Builder::new()
                .name("fm-dirscan".into())
                .spawn(move || {
                    let (progress_inbox, progress_flag) = (inbox.clone(), flag.clone());
                    let progress = move |doc: fm_preview::DirDoc| {
                        if !progress_flag.load(std::sync::atomic::Ordering::Relaxed) {
                            PreviewHub::push(
                                &progress_inbox,
                                Msg::Rendered {
                                    pane,
                                    token,
                                    preview: Box::new(Preview::Dir(doc)),
                                },
                            );
                        }
                    };
                    let run = std::panic::AssertUnwindSafe(|| {
                        fm_preview::dir::summarize_cancellable(
                            &*vfs,
                            &entry,
                            &Limits::unbounded_scan(),
                            true,
                            show_hidden,
                            Some(&flag),
                            Some(&progress),
                        )
                    });
                    let preview = match std::panic::catch_unwind(run) {
                        Ok(Ok(doc)) => Preview::Dir(doc),
                        Ok(Err(e)) => Preview::Error(e.to_string()),
                        Err(_) => Preview::Error("the folder scan failed".into()),
                    };
                    if !flag.load(std::sync::atomic::Ordering::Relaxed) {
                        PreviewHub::push(
                            &inbox,
                            Msg::Rendered {
                                pane,
                                token,
                                preview: Box::new(preview),
                            },
                        );
                    }
                });
            if spawned.is_err() {
                slot.scan = None;
                slot.body = Body::Static(Arc::new(Preview::Error(
                    "could not start the folder scan".into(),
                )));
            }
            return true;
        }
        self.jobs.submit(lane_for(entry.uri.scheme()), move |_ctx| {
            let preview = render_budgeted(
                vfs,
                entry,
                recursive,
                show_hidden,
                log_limits,
                filters,
                budget,
            );
            PreviewHub::push(
                &inbox,
                Msg::Rendered {
                    pane,
                    token,
                    preview: Box::new(preview),
                },
            );
            Ok(())
        });
        true
    }

    /// Start a follow poll when due.
    fn tick_log(&mut self, pane: PaneId, now: Instant) -> bool {
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return false;
        };
        if slot.pending.is_some() {
            return false;
        }
        let (Some(entry), token) = (slot.subject.clone(), slot.token) else {
            return false;
        };
        let Some(view) = slot.log_mut() else {
            return false;
        };
        if !view.follow || view.busy || now < view.next_follow {
            return false;
        }
        let Ok(vfs) = self.backends.require(&entry.uri) else {
            return false;
        };
        view.busy = true;
        view.next_follow = now + self.prefs.log_poll();
        let doc = view.doc.clone();
        let inbox = self.previews.inbox.clone();
        let lim = self.prefs.log_limits();
        self.jobs.submit(lane_for(entry.uri.scheme()), move |_ctx| {
            let limits = Limits::default();
            let update = match doc.read() {
                Ok(d) => log::follow_poll(&*vfs, &entry, &d, &limits, &lim),
                Err(_) => Ok(FollowUpdate::Unchanged),
            };
            let changed = match update {
                Ok(FollowUpdate::Appended {
                    records,
                    end_offset,
                }) => doc
                    .write()
                    .map(|mut d| d.append(records, end_offset, &lim))
                    .is_ok(),
                Ok(FollowUpdate::Reloaded(fresh)) => doc.write().map(|mut d| *d = *fresh).is_ok(),
                Ok(FollowUpdate::Unchanged) | Err(_) => false,
            };
            PreviewHub::push(
                &inbox,
                Msg::LogDone {
                    pane,
                    token,
                    changed,
                },
            );
            Ok(())
        });
        false
    }

    /// Ask for the chunk before the loaded window.
    fn start_earlier(&mut self, pane: PaneId) -> Result<(), FmError> {
        let slot = self.previews.slots.get_mut(&pane).ok_or_else(no_preview)?;
        if slot.subjects.len() > 1 {
            if slot.multi_window >= crate::preview_multi::MAX_WINDOW {
                return Err(FmError::InvalidParams("The combined preview window is full; select fewer files to search a larger window per file.".into()));
            }
            slot.multi_window = (slot.multi_window + LogLimits::default().chunk)
                .min(crate::preview_multi::MAX_WINDOW);
            slot.pending = Some((slot.subjects.clone(), Instant::now()));
            slot.stale = true;
            return Ok(());
        }
        let (Some(entry), token) = (slot.subject.clone(), slot.token) else {
            return Err(no_log());
        };
        let view = slot.log_mut().ok_or_else(no_log)?;
        if view.busy {
            return Ok(());
        }
        let (format, start, held) = {
            let d = view.doc.read().map_err(|_| no_log())?;
            (d.format, d.start_offset, d.bytes)
        };
        if start == 0 {
            return Ok(());
        }
        let vfs = self.backends.require(&entry.uri)?;
        view.busy = true;
        let doc = view.doc.clone();
        let inbox = self.previews.inbox.clone();
        self.jobs.submit(lane_for(entry.uri.scheme()), move |_ctx| {
            let lim = LogLimits::default();
            let changed = match log::read_earlier(&*vfs, &entry, format, start, held, &lim) {
                Ok(chunk) => doc.write().map(|mut d| d.apply_earlier(chunk)).is_ok(),
                Err(_) => false,
            };
            PreviewHub::push(
                &inbox,
                Msg::LogDone {
                    pane,
                    token,
                    changed,
                },
            );
            Ok(())
        });
        Ok(())
    }

    /// Recompute the filtered index off-thread.
    fn start_filter(&mut self, pane: PaneId) {
        let token = self.previews.token();
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return;
        };
        let slot_token = slot.token;
        let Some(view) = slot.log_mut() else {
            return;
        };
        view.filter_rev = token;
        if view.filter.is_empty() && !view.chronological {
            view.filtered = None;
            view.filter_error = None;
            return;
        }
        let (doc, filter, rev, chronological) = (
            view.doc.clone(),
            view.filter.clone(),
            token,
            view.chronological,
        );
        let inbox = self.previews.inbox.clone();
        self.jobs.submit(crate::jobs::JobLane::Local, move |_ctx| {
            let result = match doc.read() {
                Ok(d) => apply_filter(&d, &filter).map(|mut indices| {
                    if chronological {
                        indices.sort_by_key(|i| {
                            d.records.get(*i as usize).map(|r| (r.ts.is_none(), r.ts))
                        });
                    }
                    indices
                }),
                Err(_) => Err("log is unavailable".to_string()),
            };
            PreviewHub::push(
                &inbox,
                Msg::Filtered {
                    pane,
                    token: slot_token,
                    rev,
                    result,
                },
            );
            Ok(())
        });
    }

    fn drain_previews(&mut self) -> bool {
        let msgs: Vec<Msg> = match self.previews.inbox.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => return false,
        };
        let mut changed = false;
        let mut refilter = Vec::new();
        let mut rewant = Vec::new();
        for msg in msgs {
            match msg {
                Msg::BatchRendered {
                    pane,
                    token,
                    mut body,
                } => {
                    let Some(slot) = self.previews.slots.get_mut(&pane) else {
                        continue;
                    };
                    if slot.token != token {
                        continue;
                    }
                    let settings = slot
                        .log()
                        .map(|view| {
                            (
                                view.filter.clone(),
                                view.chronological,
                                view.expand_all,
                                view.columns,
                            )
                        })
                        .or_else(|| slot.saved_log.clone());
                    if let (Some((filter, chronological, expand, columns)), Body::Log(new)) =
                        (&settings, &mut body)
                    {
                        new.filter = filter.clone();
                        new.chronological = *chronological;
                        new.expand_all = *expand;
                        new.columns = *columns;
                    }
                    slot.body = body;
                    refilter.push(pane);
                    changed = true;
                }
                Msg::Rendered {
                    pane,
                    token,
                    preview,
                } => {
                    let Some(slot) = self.previews.slots.get_mut(&pane) else {
                        continue;
                    };
                    if slot.token != token {
                        continue;
                    }
                    slot.body = match *preview {
                        Preview::Log(doc) => {
                            let mut view = LogView::new(doc);
                            if let Some((filter, _, expand, columns)) = &slot.saved_log {
                                view.filter = filter.clone();
                                view.expand_all = *expand;
                                view.columns = *columns;
                                refilter.push(pane);
                            }
                            Body::Log(Box::new(view))
                        }
                        other => Body::Static(Arc::new(other)),
                    };
                    // New items (after a paste, say) need thumbnails even if the window is unchanged.
                    if let Some(w) = slot.thumb_window {
                        rewant.push((pane, w));
                    }
                    changed = true;
                }
                Msg::LogDone {
                    pane,
                    token,
                    changed: c,
                } => {
                    let Some(slot) = self.previews.slots.get_mut(&pane) else {
                        continue;
                    };
                    if slot.token != token {
                        continue;
                    }
                    if let Some(v) = slot.log_mut() {
                        v.busy = false;
                        if c {
                            v.rev += 1;
                            changed = true;
                            if !v.filter.is_empty() {
                                refilter.push(pane);
                            }
                        }
                    }
                }
                Msg::Filtered {
                    pane,
                    token,
                    rev,
                    result,
                } => {
                    let Some(slot) = self.previews.slots.get_mut(&pane) else {
                        continue;
                    };
                    if slot.token != token {
                        continue;
                    }
                    if let Some(v) = slot.log_mut()
                        && v.filter_rev == rev
                    {
                        match result {
                            Ok(ix) => {
                                v.filtered = Some(Arc::new(ix));
                                v.filter_error = None;
                            }
                            Err(e) => v.filter_error = Some(e),
                        }
                        changed = true;
                    }
                }
            }
        }
        for pane in refilter {
            self.start_filter(pane);
        }
        for (pane, window) in rewant {
            // A vanished backend just means no thumbnails; the listing itself reports errors.
            let _ = crate::thumb_cmds::request(self, pane, window);
        }
        changed
    }

    pub fn preview_slot(&self, pane: PaneId) -> Option<&PreviewSlot> {
        self.previews.slot(pane)
    }

    pub(crate) fn reload_preview(&mut self, params: &Params) -> Result<(), FmError> {
        let pane = self.preview_pane(params)?;
        let slot = self.previews.slots.get(&pane).ok_or_else(no_preview)?;
        if slot.subjects.len() < 2 || !matches!(slot.body, Body::Log(_) | Body::Grid(_)) {
            return Err(FmError::InvalidParams(
                "A loaded multi-file preview is required.".into(),
            ));
        }
        let target = slot.target.ok_or_else(no_preview)?;
        let subjects: Vec<_> = slot.subjects.iter().map(|e| e.uri.clone()).collect();
        let entries = self.preview_entries(target);
        if subjects.len() != entries.len()
            || subjects
                .iter()
                .any(|uri| !entries.iter().any(|entry| &entry.uri == uri))
        {
            return Err(FmError::InvalidParams(
                "The selected files changed; wait for their preview to load.".into(),
            ));
        }
        let slot = self.previews.slots.get_mut(&pane).ok_or_else(no_preview)?;
        slot.pending = Some((entries, Instant::now()));
        slot.stale = true;
        self.start_render(pane);
        Ok(())
    }

    /// The Preview pane a `preview.*` command acts on.
    pub(crate) fn preview_pane(&self, params: &Params) -> Result<PaneId, FmError> {
        let is_preview = |id: PaneId| {
            self.layout
                .state(id)
                .is_some_and(|s| s.kind == PaneKind::Preview)
        };
        if let Some(p) = object(params)?
            .and_then(|m| m.get("pane"))
            .and_then(Params::as_u64)
        {
            return is_preview(p).then_some(p).ok_or_else(no_preview);
        }
        let focused = self.layout.focused();
        if is_preview(focused) {
            return Ok(focused);
        }
        self.layout
            .panes()
            .into_iter()
            .find(|p| is_preview(*p))
            .ok_or_else(no_preview)
    }
}

pub(crate) fn no_preview() -> FmError {
    FmError::NotFound("no preview pane".to_string())
}

fn no_log() -> FmError {
    FmError::InvalidParams("the preview is not showing a log".to_string())
}

fn render_budgeted(
    vfs: Arc<dyn Vfs>,
    entry: Entry,
    recursive: bool,
    show_hidden: bool,
    logs: LogLimits,
    filters: crate::preview_types::PreviewFilters,
    budget: Duration,
) -> Preview {
    run_budgeted(budget, move || {
        let rules = match filters.compile() {
            Ok(rules) => rules,
            Err(error) => return Preview::Error(error.to_string()),
        };
        fm_preview::render_configured(
            &*vfs,
            &entry,
            &Limits::default(),
            fm_preview::RenderOpts {
                recursive,
                show_hidden,
            },
            &logs,
            rules.kind(&entry.name),
        )
    })
    .unwrap_or_else(|| Preview::Error("the preview took too long and was abandoned".into()))
}

pub(crate) fn has_preview(s: &Session) -> bool {
    s.layout
        .states
        .values()
        .any(|st| st.kind == PaneKind::Preview)
}

fn has_structured(s: &Session) -> bool {
    s.previews.slots.values().any(|sl| {
        matches!(&sl.body, Body::Static(p) if structured_doc(p, sl.source).is_some_and(|d| d.pretty.is_ok()))
    })
}

/// The document Pretty applies to: a structured file, or an SVG while its source is shown.
fn structured_doc(p: &Preview, source: bool) -> Option<&fm_preview::StructDoc> {
    match p {
        Preview::Structured(doc) => Some(doc),
        Preview::Svg(doc) if source => Some(&doc.source),
        _ => None,
    }
}

/// Markdown and SVG previews can switch between the rendered view and the source.
fn has_source_view(p: &Preview) -> bool {
    matches!(p, Preview::Markdown(_) | Preview::Svg(_))
}

fn has_markdown(s: &Session) -> bool {
    s.previews
        .slots
        .values()
        .any(|sl| matches!(&sl.body, Body::Static(p) if has_source_view(p)))
}

fn has_log(s: &Session) -> bool {
    s.previews.slots.values().any(|sl| sl.log().is_some())
}

fn str_param<'a>(params: &'a Params, key: &str) -> Option<&'a str> {
    object(params)
        .ok()
        .flatten()
        .and_then(|m| m.get(key))
        .and_then(Params::as_str)
}

fn bool_param(params: &Params, key: &str) -> Option<bool> {
    object(params)
        .ok()
        .flatten()
        .and_then(|m| m.get(key))
        .and_then(Params::as_bool)
}

/// `view.preview.toggle` — open a Preview pane to the right of the focused pane, or close it.
pub fn toggle() -> CommandSpec {
    CommandSpec {
        id: "view.preview.toggle",
        label: "Preview",
        menu: &["View", "Preview"],
        shortcut: Some("Ctrl+Shift+P"),
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            let existing = s.layout.panes().into_iter().find(|p| {
                s.layout
                    .state(*p)
                    .is_some_and(|st| st.kind == PaneKind::Preview)
            });
            if let Some(p) = existing {
                if s.layout.panes().len() > 1 {
                    return s.layout.close(p);
                }
                if let Some(st) = s.layout.state_mut(p) {
                    st.kind = PaneKind::Files;
                }
                return Ok(());
            }
            let origin = s.layout.focused();
            let new_id = s.layout.split(origin, Axis::Horizontal, 0.55)?;
            if let Some(st) = s.layout.state_mut(new_id) {
                st.kind = PaneKind::Preview;
                st.preview_source = PreviewSource::Follow;
            }
            s.layout.focus_pane(origin)
        },
    }
}

/// `pane.preview.source {pane: number|"focus", target?}`
pub fn source() -> CommandSpec {
    CommandSpec {
        id: "pane.preview.source",
        label: "Preview Source",
        menu: &["Panes", "Preview Source"],
        shortcut: None,
        params: "{source: number|\"focus\", pane?: number}",
        enabled: has_preview,
        run: |s, p: &Params| {
            let preview = s.preview_pane(p)?;
            let value = object(p)?.and_then(|m| m.get("source")).ok_or_else(|| {
                FmError::InvalidParams("source: number or \"focus\" required".into())
            })?;
            let src = match (value.as_u64(), value.as_str()) {
                (_, Some("focus")) => PreviewSource::Follow,
                (Some(id), _) => {
                    if s.layout.state(id).is_none_or(|st| !st.kind.is_files()) {
                        return Err(FmError::InvalidParams(format!(
                            "pane {id} is not a Files pane"
                        )));
                    }
                    PreviewSource::Pane(id)
                }
                _ => {
                    return Err(FmError::InvalidParams(
                        "source: number or \"focus\" required".into(),
                    ));
                }
            };
            if let Some(st) = s.layout.state_mut(preview) {
                st.preview_source = src;
            }
            if let Some(slot) = s.previews.slots.get_mut(&preview) {
                slot.key = None;
            }
            Ok(())
        },
    }
}

/// `preview.zoom {mode: "fit"|"actual"|"in"|"out"}`
pub fn zoom() -> CommandSpec {
    CommandSpec {
        id: "preview.zoom",
        label: "Preview Zoom",
        menu: &["View", "Preview Zoom"],
        shortcut: None,
        params: "{mode: \"fit\"|\"actual\"|\"in\"|\"out\", pane?: number}",
        enabled: has_preview,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let mode = str_param(p, "mode")
                .ok_or_else(|| FmError::InvalidParams("mode: string required".into()))?;
            let slot = s
                .previews
                .slots
                .entry(pane)
                .or_insert_with(PreviewSlot::new);
            let cur = match slot.zoom {
                Zoom::Scale(v) => v,
                _ => 1.0,
            };
            slot.zoom = match mode {
                "fit" => Zoom::Fit,
                "actual" => Zoom::Actual,
                "in" => Zoom::Scale((cur * 1.25).min(16.0)),
                "out" => Zoom::Scale((cur / 1.25).max(0.05)),
                _ => {
                    return Err(FmError::InvalidParams(
                        "mode must be fit, actual, in or out".into(),
                    ));
                }
            };
            if matches!(mode, "fit" | "actual") {
                slot.scale = 1.0;
            }
            Ok(())
        },
    }
}

/// `preview.toggle_wrap`
pub fn toggle_wrap() -> CommandSpec {
    CommandSpec {
        id: "preview.toggle_wrap",
        label: "Preview Word Wrap",
        menu: &["View", "Preview Word Wrap"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: has_preview,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let slot = s
                .previews
                .slots
                .entry(pane)
                .or_insert_with(PreviewSlot::new);
            slot.wrap = !slot.wrap;
            Ok(())
        },
    }
}

/// `preview.toggle_pretty` — JSON/YAML/TOML/XML re-indented; refused with the reason when invalid.
pub fn toggle_pretty() -> CommandSpec {
    CommandSpec {
        id: "preview.toggle_pretty",
        label: "Preview Pretty Print",
        menu: &["View", "Preview Pretty Print"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: has_structured,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let slot = s
                .previews
                .slots
                .entry(pane)
                .or_insert_with(PreviewSlot::new);
            let Body::Static(prev) = &slot.body else {
                return Err(FmError::InvalidParams("nothing to pretty-print".into()));
            };
            let Some(doc) = structured_doc(prev, slot.source) else {
                return Err(FmError::InvalidParams(
                    "only JSON, YAML, TOML, XML and SVG source can be pretty-printed".into(),
                ));
            };
            if let Err(why) = &doc.pretty {
                return Err(FmError::InvalidParams(why.clone()));
            }
            slot.pretty = !slot.pretty;
            Ok(())
        },
    }
}

/// `preview.toggle_source` — show a Markdown or SVG file's text instead of the rendered view.
pub fn toggle_source() -> CommandSpec {
    CommandSpec {
        id: "preview.toggle_source",
        label: "Preview Source",
        menu: &["View", "Preview Source"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: has_markdown,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let slot = s
                .previews
                .slots
                .entry(pane)
                .or_insert_with(PreviewSlot::new);
            let Body::Static(prev) = &slot.body else {
                return Err(FmError::InvalidParams(
                    "nothing to show the source of".into(),
                ));
            };
            if !has_source_view(prev) {
                return Err(FmError::InvalidParams(
                    "only Markdown and SVG previews have a source view".into(),
                ));
            }
            slot.source = !slot.source;
            if matches!(&**prev, Preview::Svg(_)) && !slot.source {
                // Pretty belongs to the source view; the picture has no such mode.
                slot.pretty = false;
            }
            Ok(())
        },
    }
}

pub(crate) fn log_cmd(
    pane_params: &Params,
    s: &mut Session,
    f: impl FnOnce(&mut LogView) -> Result<bool, FmError>,
) -> Result<(), FmError> {
    let pane = s.preview_pane(pane_params)?;
    let slot = s.previews.slots.get_mut(&pane).ok_or_else(no_preview)?;
    let view = slot.log_mut().ok_or_else(no_log)?;
    if f(view)? {
        s.start_filter(pane);
    }
    Ok(())
}

/// `preview.log.level {level: "debug"|…|"critical"|null}`
pub fn log_level() -> CommandSpec {
    CommandSpec {
        id: "preview.log.level",
        label: "Log: Minimum Level",
        menu: &["View", "Log Level"],
        shortcut: None,
        params: "{level: \"debug\"|\"info\"|\"notice\"|\"warning\"|\"error\"|\"critical\"|null, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let level = match str_param(p, "level") {
                None | Some("") | Some("all") => None,
                Some(l) => Some(Level::parse(l).ok_or_else(|| {
                    FmError::InvalidParams(
                        "level must be debug, info, notice, warning, error or critical".into(),
                    )
                })?),
            };
            log_cmd(p, s, |v| {
                v.filter.min_level = level;
                Ok(true)
            })
        },
    }
}

const MAX_QUERY: usize = 512;

/// `preview.log.search {query, regex?}`
pub fn log_search() -> CommandSpec {
    CommandSpec {
        id: "preview.log.search",
        label: "Log: Search",
        menu: &["View", "Log Search"],
        shortcut: None,
        params: "{query: string, regex?: bool, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let query = str_param(p, "query")
                .ok_or_else(|| FmError::InvalidParams("query: string required".into()))?;
            if query.len() > MAX_QUERY {
                return Err(FmError::InvalidParams(format!(
                    "query is longer than {MAX_QUERY} bytes"
                )));
            }
            let regex = bool_param(p, "regex").unwrap_or(false);
            let query = query.to_string();
            log_cmd(p, s, |v| {
                v.filter.query = query;
                v.filter.regex = regex;
                Ok(true)
            })
        },
    }
}

/// `preview.log.time_range {from?: "YYYY-MM-DD[ HH:MM[:SS]]", to?: …}` (empty clears)
pub fn log_time_range() -> CommandSpec {
    CommandSpec {
        id: "preview.log.time_range",
        label: "Log: Time Range",
        menu: &["View", "Log Time Range"],
        shortcut: None,
        params: "{from?: string, to?: string, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let bound = |key: &str| -> Result<Option<i64>, FmError> {
                match str_param(p, key) {
                    None | Some("") => Ok(None),
                    Some(t) => parse_bound(t).map(Some).ok_or_else(|| {
                        FmError::InvalidParams(format!("{key}: expected YYYY-MM-DD[ HH:MM[:SS]]"))
                    }),
                }
            };
            let (from, to) = (bound("from")?, bound("to")?);
            if let (Some(f), Some(t)) = (from, to)
                && f > t
            {
                return Err(FmError::InvalidParams("from is after to".into()));
            }
            log_cmd(p, s, |v| {
                v.filter.from = from;
                v.filter.to = to;
                Ok(true)
            })
        },
    }
}

/// `preview.log.follow {on?: bool}` — toggles when `on` is absent.
pub fn log_follow() -> CommandSpec {
    CommandSpec {
        id: "preview.log.follow",
        label: "Log: Follow",
        menu: &["View", "Log Follow"],
        shortcut: None,
        params: "{on?: bool, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let on = bool_param(p, "on");
            log_cmd(p, s, |v| {
                let want = on.unwrap_or(!v.follow);
                if want
                    && (!v.sources.is_empty()
                        || !v.doc.read().map(|d| d.can_follow()).unwrap_or(false))
                {
                    return Err(FmError::InvalidParams("this log cannot be followed".into()));
                }
                v.follow = want;
                v.next_follow = Instant::now();
                Ok(false)
            })
        },
    }
}

/// `preview.log.columns {on?: bool}` — table view for access logs.
pub fn log_columns() -> CommandSpec {
    CommandSpec {
        id: "preview.log.columns",
        label: "Log: Columns",
        menu: &["View", "Log Columns"],
        shortcut: None,
        params: "{on?: bool, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let on = bool_param(p, "on");
            log_cmd(p, s, |v| {
                v.columns = on.unwrap_or(!v.columns);
                Ok(false)
            })
        },
    }
}

/// `preview.log.fold {mode: "expand"|"collapse"}` or `{record: key}` to flip one record.
pub fn log_fold() -> CommandSpec {
    CommandSpec {
        id: "preview.log.fold",
        label: "Log: Fold Stack Traces",
        menu: &["View", "Log Fold"],
        shortcut: None,
        params: "{mode?: \"expand\"|\"collapse\", record?: number, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let record = object(p)?
                .and_then(|m| m.get("record"))
                .and_then(Params::as_u64);
            let mode = str_param(p, "mode");
            if record.is_none() && !matches!(mode, Some("expand" | "collapse")) {
                return Err(FmError::InvalidParams(
                    "mode must be expand or collapse, or give a record".into(),
                ));
            }
            log_cmd(p, s, |v| {
                if let Some(key) = record {
                    if !v.expanded.remove(&key) {
                        v.expanded.insert(key);
                    }
                } else {
                    v.expand_all = mode == Some("expand");
                    v.expanded.clear();
                }
                Ok(false)
            })
        },
    }
}

/// `preview.log.mark {clear?: bool}` — set a marker below the last loaded record, or
/// remove every marker with `clear`.
pub fn log_mark() -> CommandSpec {
    CommandSpec {
        id: "preview.log.mark",
        label: "Log: Set Marker",
        menu: &["View", "Log Marker"],
        shortcut: Some("Ctrl+Shift+M"),
        params: "{clear?: bool, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let clear = bool_param(p, "clear").unwrap_or(false);
            log_cmd(p, s, |v| {
                let mut d = v.doc.write().map_err(|_| no_log())?;
                if clear {
                    d.marks.clear();
                } else {
                    d.mark_here();
                }
                v.rev += 1;
                Ok(false)
            })
        },
    }
}

/// `preview.log.earlier` — load the chunk before the window.
pub fn log_earlier() -> CommandSpec {
    CommandSpec {
        id: "preview.log.earlier",
        label: "Log: Load Earlier",
        menu: &["View", "Log Load Earlier"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            s.start_earlier(pane)
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        toggle(),
        source(),
        zoom(),
        toggle_wrap(),
        toggle_pretty(),
        toggle_source(),
        log_level(),
        log_search(),
        log_time_range(),
        log_follow(),
        log_columns(),
        log_fold(),
        log_mark(),
        log_earlier(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::expect_used)]
    fn selection_change_stops_players_before_the_debounce_and_does_not_restart_them() {
        for video in [false, true] {
            let a = Entry::new(VfsUri::parse("/a.mp3").expect("uri"), EntryKind::File);
            let b = Entry::new(VfsUri::parse("/b.mp3").expect("uri"), EntryKind::File);
            let mut s = Session::default();
            let mut slot = PreviewSlot::new();
            slot.subject = Some(a.clone());
            slot.subjects = vec![a.clone()];
            slot.key = Some(vec![key_of(&a, false, false)]);
            slot.body = Body::Static(Arc::new(Preview::Meta(fm_preview::MetaDoc {
                rows: vec![],
                hint: None,
                poster: None,
                video: video.then(|| fm_preview::VideoInfo {
                    path: "/a.mp4".into(),
                    range: None,
                    width: 640,
                    height: 360,
                    fps: 25.0,
                    duration: 60.0,
                    audio: None,
                    audio_delay: 0.0,
                    tracks: Default::default(),
                }),
                audio: (!video).then(|| fm_preview::audio::AudioDoc {
                    path: "/a.mp3".into(),
                    duration: Some(60.0),
                }),
            })));
            s.previews.slots.insert(1, slot);
            s.audio_preview_tick();
            s.video_tick();
            let slot = s.previews.slot(1).expect("slot");
            assert_eq!(slot.audio.is_some(), !video);
            assert_eq!(slot.video.is_some(), video);

            s.advance_slot(1, vec![b], Instant::now());
            assert!(s.previews.slot(1).expect("slot").audio.is_none());
            s.prefs.preview_audio_autoplay = true;
            s.audio_preview_tick();
            s.video_tick();
            let slot = s.previews.slot(1).expect("slot");
            assert!(slot.pending.is_some());
            assert!(
                slot.audio.is_none(),
                "old audio must not restart while settling"
            );
            assert!(
                slot.video.is_none(),
                "old video must not restart while settling"
            );
        }
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn stale_renders_cannot_replace_cleared_or_reselected_previews() {
        let a = Entry::new(
            VfsUri::parse("/a.log").expect("uri"),
            fm_vfs::EntryKind::File,
        );
        let b = Entry::new(
            VfsUri::parse("/b.log").expect("uri"),
            fm_vfs::EntryKind::File,
        );
        let mut s = Session::default();
        let mut slot = PreviewSlot::new();
        slot.token = 7;
        slot.subject = Some(a.clone());
        slot.subjects = vec![a.clone()];
        slot.key = Some(vec![key_of(&a, false, false)]);
        slot.body = Body::Loading;
        s.previews.slots.insert(1, slot);
        let now = Instant::now();
        s.advance_slot(1, vec![b], now);
        s.advance_slot(1, vec![a.clone()], now + Duration::from_millis(1));
        let slot = s.previews.slot(1).expect("slot");
        assert_eq!(slot.pending.as_ref().expect("pending").0[0].uri, a.uri);
        PreviewHub::push(
            &s.previews.inbox,
            Msg::Rendered {
                pane: 1,
                token: 7,
                preview: Box::new(Preview::Error("stale".into())),
            },
        );
        s.drain_previews();
        assert!(matches!(
            s.previews.slot(1).expect("slot").body,
            Body::Loading
        ));
        s.advance_slot(1, Vec::new(), now);
        PreviewHub::push(
            &s.previews.inbox,
            Msg::BatchRendered {
                pane: 1,
                token: 7,
                body: Body::Grid(Vec::new()),
            },
        );
        s.drain_previews();
        assert!(matches!(
            s.previews.slot(1).expect("slot").body,
            Body::Empty
        ));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn mixed_servers_do_not_use_the_first_servers_backend() {
        let mut s = Session::default();
        let mut slot = PreviewSlot::new();
        slot.pending = Some((
            vec![
                Entry::new(
                    VfsUri::parse("sftp://alice@first/a.log").expect("uri"),
                    fm_vfs::EntryKind::File,
                ),
                Entry::new(
                    VfsUri::parse("sftp://alice@second/b.log").expect("uri"),
                    fm_vfs::EntryKind::File,
                ),
            ],
            Instant::now(),
        ));
        s.previews.slots.insert(1, slot);
        s.start_render(1);
        assert!(matches!(&s.previews.slot(1).expect("slot").body,
            Body::Static(p) if matches!(&**p, Preview::Error(e) if e.contains("single filesystem"))
        ));
    }

    #[test]
    fn budget_gives_up_on_slow_work() {
        let slow = run_budgeted(Duration::from_millis(30), || {
            std::thread::sleep(Duration::from_millis(400));
            1
        });
        assert_eq!(slow, None);
        assert_eq!(run_budgeted(Duration::from_secs(2), || 7), Some(7));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn live_tail_schedules_configured_interval_and_changes_reschedule() {
        let fs = fm_testkit::MemFs::new();
        fs.put_str("/a.log", b"2024-05-01 10:00:00 INFO a\n")
            .expect("file");
        let uri = VfsUri::parse("/a.log").expect("uri");
        let entry = fs.stat(&uri).expect("stat");
        let doc = log::open_doc(&fs, &entry, &Limits::default()).expect("doc");
        let mut s = Session::new().expect("session");
        s.backends.register(Arc::new(fs));
        let mut slot = PreviewSlot::new();
        slot.subject = Some(entry);
        slot.body = Body::Log(Box::new(LogView::new(doc)));
        slot.log_mut().expect("log").follow = true;
        s.previews.slots.insert(1, slot);
        s.dispatch("prefs.preview.log", &serde_json::json!({"poll_ms": 2500}))
            .expect("setting");
        let now = Instant::now();
        s.tick_log(1, now);
        let view = s
            .previews
            .slots
            .get_mut(&1)
            .expect("slot")
            .log_mut()
            .expect("log");
        assert_eq!(view.next_follow, now + Duration::from_millis(2500));
        view.busy = false;
        s.tick_log(1, now + Duration::from_millis(2499));
        assert!(
            !s.previews
                .slot(1)
                .expect("slot")
                .log()
                .expect("log")
                .is_busy()
        );
        s.dispatch("prefs.preview.log", &serde_json::json!({"poll_ms": 100}))
            .expect("setting");
        s.tick_log(1, Instant::now());
        assert!(
            s.previews
                .slot(1)
                .expect("slot")
                .log()
                .expect("log")
                .is_busy()
        );
    }
}
