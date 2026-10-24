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
pub const FOLLOW_EVERY: Duration = Duration::from_millis(500);
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
    busy: bool,
    next_follow: Instant,
    filter_rev: u64,
}

impl LogView {
    fn new(doc: LogDoc) -> Self {
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
            busy: false,
            next_follow: Instant::now(),
            filter_rev: 0,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }
}

#[derive(Debug)]
pub enum Body {
    Empty,
    Loading,
    Static(Arc<Preview>),
    Log(Box<LogView>),
}

type Key = (VfsUri, Option<SystemTime>, Option<u64>, bool);

fn key_of(e: &Entry, recursive: bool) -> Key {
    (e.uri.clone(), e.modified, e.size, recursive)
}

#[derive(Debug)]
pub struct PreviewSlot {
    /// What the body is about.
    pub subject: Option<Entry>,
    pub body: Body,
    pub pretty: bool,
    pub wrap: bool,
    pub zoom: Zoom,
    /// Directory thumbnail grid instead of the summary card.
    pub thumbs: bool,
    /// Index into [`fm_preview::thumbs::EDGES`].
    pub thumb_size: usize,
    /// Entries the grid last asked for: `(first, count)`.
    pub thumb_window: Option<(usize, usize)>,
    /// Playback of a video shown by this pane.
    pub video: Option<crate::video_cmds::VideoState>,
    /// The Files pane being previewed (for the header).
    pub target: Option<PaneId>,
    /// Something changed inside the previewed folder; re-render it in place.
    pub stale: bool,
    token: u64,
    key: Option<Key>,
    /// Stops a running recursive folder scan when the slot moves on or is dropped.
    scan: Option<ScanCancel>,
    pending: Option<(Entry, Instant)>,
}

impl PreviewSlot {
    pub(crate) fn new() -> Self {
        Self {
            subject: None,
            body: Body::Empty,
            pretty: false,
            wrap: false,
            zoom: Zoom::Fit,
            thumbs: false,
            thumb_size: 0,
            thumb_window: None,
            video: None,
            target: None,
            stale: false,
            token: 0,
            key: None,
            scan: None,
            pending: None,
        }
    }

    pub fn log(&self) -> Option<&LogView> {
        match &self.body {
            Body::Log(l) => Some(l),
            _ => None,
        }
    }

    fn log_mut(&mut self) -> Option<&mut LogView> {
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
    Rendered {
        pane: PaneId,
        token: u64,
        preview: Preview,
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
            let desired = target.and_then(|t| self.cursor_entry(t));
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

    fn advance_slot(&mut self, pane: PaneId, desired: Option<Entry>, now: Instant) -> bool {
        let mut changed = false;
        let recursive = self.prefs.recursive_preview;
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return false;
        };
        match desired {
            None => {
                if slot.key.is_some() || slot.pending.is_some() {
                    slot.key = None;
                    slot.scan = None;
                    slot.pending = None;
                    slot.subject = None;
                    slot.body = Body::Empty;
                    slot.pretty = false;
                    slot.thumb_window = None;
                    self.previews.token();
                    changed = true;
                }
            }
            Some(e) => {
                let same_subject = slot.subject.as_ref().is_some_and(|s| s.uri == e.uri);
                let same_log =
                    slot.log().is_some() && slot.subject.as_ref().is_some_and(|s| s.uri == e.uri);
                if slot.stale && same_subject && slot.pending.is_none() {
                    slot.pending = Some((e, now));
                } else if slot.key.as_ref() == Some(&key_of(&e, recursive)) || same_log {
                    slot.pending = None;
                } else if slot.pending.as_ref().map(|(p, _)| key_of(p, recursive))
                    != Some(key_of(&e, recursive))
                {
                    slot.pending = Some((e, now + DEBOUNCE));
                    // The cursor moved on: stop the old folder scan right away.
                    slot.scan = None;
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
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return false;
        };
        let Some((entry, _)) = slot.pending.take() else {
            return false;
        };
        let refresh = slot.stale && slot.subject.as_ref().is_some_and(|s| s.uri == entry.uri);
        slot.stale = false;
        if slot.subject.as_ref().is_none_or(|s| s.uri != entry.uri) {
            slot.pretty = false;
            slot.thumb_window = None;
            self.thumbs.cancel(pane);
        }
        slot.token = token;
        slot.scan = None;
        slot.key = Some(key_of(&entry, recursive));
        slot.subject = Some(entry.clone());
        if !refresh {
            slot.body = Body::Loading;
        }
        let vfs = match self.backends.require(&entry.uri) {
            Ok(v) => v,
            Err(e) => {
                slot.body = Body::Static(Arc::new(Preview::Error(e.to_string())));
                return true;
            }
        };
        let inbox = self.previews.inbox.clone();
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
                                    preview: Preview::Dir(doc),
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
                                preview,
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
            let preview = render_budgeted(vfs, entry, recursive);
            PreviewHub::push(
                &inbox,
                Msg::Rendered {
                    pane,
                    token,
                    preview,
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
        view.next_follow = now + FOLLOW_EVERY;
        let doc = view.doc.clone();
        let inbox = self.previews.inbox.clone();
        self.jobs.submit(lane_for(entry.uri.scheme()), move |_ctx| {
            let (limits, lim) = (Limits::default(), LogLimits::default());
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
        if view.filter.is_empty() {
            view.filtered = None;
            view.filter_error = None;
            return;
        }
        let (doc, filter, rev) = (view.doc.clone(), view.filter.clone(), token);
        let inbox = self.previews.inbox.clone();
        self.jobs.submit(crate::jobs::JobLane::Local, move |_ctx| {
            let result = match doc.read() {
                Ok(d) => apply_filter(&d, &filter),
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
        for msg in msgs {
            match msg {
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
                    slot.body = match preview {
                        Preview::Log(doc) => Body::Log(Box::new(LogView::new(doc))),
                        other => Body::Static(Arc::new(other)),
                    };
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
        changed
    }

    pub fn preview_slot(&self, pane: PaneId) -> Option<&PreviewSlot> {
        self.previews.slot(pane)
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

fn render_budgeted(vfs: Arc<dyn Vfs>, entry: Entry, recursive: bool) -> Preview {
    run_budgeted(RENDER_BUDGET, move || {
        fm_preview::render_with(
            &*vfs,
            &entry,
            &Limits::default(),
            fm_preview::RenderOpts { recursive },
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
        matches!(&sl.body, Body::Static(p) if matches!(&**p, Preview::Structured(d) if d.pretty.is_ok()))
    })
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
                    if s.layout
                        .state(id)
                        .is_none_or(|st| st.kind != PaneKind::Files)
                    {
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
            let Preview::Structured(doc) = &**prev else {
                return Err(FmError::InvalidParams(
                    "only JSON, YAML, TOML and XML can be pretty-printed".into(),
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

fn log_cmd(
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
        shortcut: Some("Ctrl+Shift+F"),
        params: "{on?: bool, pane?: number}",
        enabled: has_log,
        run: |s, p: &Params| {
            let on = bool_param(p, "on");
            log_cmd(p, s, |v| {
                let want = on.unwrap_or(!v.follow);
                if want && !v.doc.read().map(|d| d.can_follow()).unwrap_or(false) {
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
    fn budget_gives_up_on_slow_work() {
        let slow = run_budgeted(Duration::from_millis(30), || {
            std::thread::sleep(Duration::from_millis(400));
            1
        });
        assert_eq!(slow, None);
        assert_eq!(run_budgeted(Duration::from_secs(2), || 7), Some(7));
    }
}
