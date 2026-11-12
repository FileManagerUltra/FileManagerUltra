//! `Session` — the engine's application state and the single dispatch point.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use fm_layout::{Layout, PaneId, SortSpec};
use fm_ops::UndoStack;
use fm_vfs::{Entry, FmError, VfsUri};

use crate::backends::{VfsRegistry, lane_for};
use crate::commands;
use crate::dircache::{DirCache, DirState, ReadOutcome};
use crate::dnd_cmds::DeviceCache;
use crate::fileops::FileOps;
use crate::jobs::{JobEvent, JobId, JobQueue};
use crate::prefs::Prefs;
use crate::registry::{Params, Registry};

/// Cap: no command parameter payload may exceed this size (capability `security`).
pub const MAX_PARAMS_BYTES: usize = 1_048_576;
/// Cap: no parameter nesting may exceed this depth (capability `security`).
pub const MAX_PARAMS_DEPTH: u32 = 32;

/// A pane's sorted view of a cached directory: which entry indices to show,
/// in order, for a given cache revision and sort spec.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewIndex {
    pub revision: u64,
    pub sort: SortSpec,
    pub hidden: bool,
    /// The quick filter the order was built with (`None` or empty: no filter).
    pub filter: Option<String>,
    /// Indices into the cached entry slice.
    pub order: Arc<[u32]>,
}

/// Owns the command registry, pane layout, job queue, preferences and the flags the
/// UI observes. The UI dispatches into a `Session` and renders its state; it never
/// performs work itself.
#[derive(Debug)]
pub struct Session {
    /// Session-wide lease for the single active audio output.
    pub audio_owner: crate::audio_owner::AudioOwner,
    pub media: crate::media_cmds::MediaSession,
    /// Set while the Media pane fills the window (Media > Fullscreen).
    pub media_fullscreen: Option<crate::media_fullscreen_cmds::MediaFullscreen>,
    /// Requested native window fullscreen; the app applies it. Never persisted.
    pub window_fullscreen: bool,
    pub(crate) media_store: crate::media_store::MediaStore,
    pub(crate) playlist_job: crate::media_playlist_cmds::PlaylistJob,
    pub(crate) media_source: crate::media_source_cmds::MediaSourceJob,
    pub registry: Registry,
    pub prefs: Prefs,
    /// OS timezone captured at startup; conversions during rendering do no filesystem I/O.
    pub file_time_zone: jiff::tz::TimeZone,
    pub jobs: JobQueue,
    /// The active workspace tab's layout; inactive tabs park theirs in `workspaces`.
    pub layout: Layout,
    /// Open workspace tabs and the saved set (capability `workspaces`).
    pub workspaces: crate::workspace::Workspaces,
    /// Raised when quitting or closing a tab would lose unsaved workspace changes.
    pub unsaved_prompt: Option<crate::workspace::UnsavedPrompt>,
    /// The focused thumbnail grid's selection, published by the UI each frame.
    pub grid_selection: Option<crate::file_cmds::GridSelection>,
    /// Files cut or copied for a later paste.
    pub clipboard: Option<crate::clipboard_cmds::Clipboard>,
    /// Text for the app to put on the OS clipboard (taken once per frame).
    pub clipboard_outbox: Option<String>,
    /// `app.quit` flips this; the eframe loop stops on it.
    pub running: bool,
    /// Last command failure the user should see; the UI moves it into [`Session::messages`]
    /// with [`Session::drain_notices`].
    pub notice: Option<String>,
    /// Recent failures and warnings, shown in the jobs panel.
    pub messages: crate::messages::MessageLog,
    /// Short confirmation for the status bar; the UI hides it after a few seconds.
    pub info: Option<(String, std::time::Instant)>,
    /// Dot-files are hidden by default; `view.hidden.toggle` flips this for every pane.
    pub show_hidden: bool,
    /// Show `-> target` after symlink names; `view.links.toggle` flips it.
    pub show_link_targets: bool,
    /// The pane location being edited (`nav.goto`), with its suggestions.
    pub path_edit: Option<crate::goto_cmds::PathEdit>,
    /// Visited locations this session, most recent first (feeds path suggestions).
    pub path_history: Vec<String>,
    /// One shell per Terminal pane (capability `terminal-pane`).
    pub terminals: HashMap<PaneId, fm_term::TermSession>,
    /// Current terminal text selection; cleared on scroll/resize.
    pub terminal_selection: Option<crate::term_cmds::TermSelection>,
    /// Preview pane contents and their background jobs (capability `preview-pane`).
    pub previews: crate::preview_cmds::PreviewHub,
    pub preview_text_selection: Option<crate::preview_text_cmds::TextSelection>,
    /// Thumbnails for directory previews (own worker pool, memory + disk cache).
    pub thumbs: crate::thumbs::ThumbService,
    /// Images of the Markdown preview (own worker pools, byte-capped memory cache).
    pub md_images: crate::md_images::MdImageService,
    /// Text a command wants on the system clipboard; the app shell drains it.
    pub clipboard_out: Option<String>,
    /// Pixels a command wants on the system clipboard; the app shell drains it.
    pub clipboard_image: Option<crate::image_clip_cmds::ClipImage>,
    /// Background decodes for `image.copy`.
    pub image_clip: crate::image_clip_cmds::ImageClip,
    /// A pane-type switch waiting for the user to confirm ending a running terminal job.
    pub pending_kind: Option<(PaneId, fm_layout::PaneKind)>,
    /// `app.about` raises this flag; the UI shows the dialog and clears it.
    pub about_open: bool,
    /// Non-blocking release check and its dismissible notification.
    pub updates: crate::updates::Updates,
    pub ffmpeg: crate::ffmpeg_install::Ffmpeg,
    /// Set by `app.preferences`; the UI renders the dialog and clears it.
    pub preferences_open: bool,
    /// Shared directory listings (capability `directory-cache`).
    pub dirs: DirCache,
    /// Live watches on visible local directories (capability `directory-cache`).
    pub watch: crate::watch::Watcher,
    /// Git status per repository (capability `git-status`).
    pub git: crate::git::GitState,
    /// Diff state and loaded comparisons of Preview panes (capability `git-diff`).
    pub diffs: crate::diff_cmds::DiffHub,
    /// Scheme → backend; local disk is always registered.
    pub backends: VfsRegistry,
    /// Running and recently finished file jobs (capability `job-progress`).
    pub fileops: FileOps,
    /// SFTP connections, host keys and remote polling (capability `sftp-backend`).
    pub remote: crate::remote::RemoteHub,
    /// Trash manager panel state (capability `trash-and-undo`).
    pub trash: crate::trash_cmds::TrashState,
    /// Inverse records of finished jobs, newest last (capability `trash-and-undo`).
    pub undo: UndoStack,
    /// The Open With dialog and the (lazily scanned) installed applications.
    pub open_with: Option<crate::openwith_cmds::OpenWith>,
    /// The "Compare with…" file chooser, while open.
    pub compare: Option<crate::diff_compare_cmds::ComparePicker>,
    pub(crate) app_catalog: Option<Arc<Mutex<Option<Arc<crate::openwith_cmds::AppCatalog>>>>>,
    /// The Find Files window and its running search.
    pub search: Option<crate::search_cmds::Search>,
    /// The open Properties dialog and its folder-size walk.
    pub properties: Option<crate::props_cmds::Properties>,
    /// Home, Documents, Downloads… resolved at start-up (capability `places`).
    pub standard_places: crate::places_cmds::StandardPlaces,
    /// Mounted removable and user volumes (separate from the drag/drop device cache).
    pub volumes: crate::devices::Devices,
    pub(crate) devices: DeviceCache,
    /// Staged read results, written by workers, drained by [`Session::poll`].
    inbox: Arc<Mutex<HashMap<VfsUri, ReadOutcome>>>,
    /// In-flight read jobs: uri → job id (dedup) and job id → uri (drain).
    reads: HashMap<JobId, VfsUri>,
    inflight: HashMap<VfsUri, JobId>,
    /// Per-pane sorted view indexes, rebuilt when the revision or sort changes.
    indexes: HashMap<PaneId, ViewIndex>,
}

impl Default for Session {
    fn default() -> Self {
        let first_layout = Layout::single(VfsUri::root_file());
        let remote = crate::remote::RemoteHub::default();
        let mut backends = VfsRegistry::with_local();
        backends.register(Arc::new(crate::servers::ServersFs::new(
            remote.servers.clone(),
        )));
        Self {
            standard_places: crate::places_cmds::StandardPlaces::detect(),
            volumes: Default::default(),
            properties: None,
            search: None,
            open_with: None,
            compare: None,
            app_catalog: None,
            registry: Registry::new(),
            audio_owner: crate::audio_owner::AudioOwner::None,
            media: Default::default(),
            media_fullscreen: None,
            window_fullscreen: false,
            media_store: Default::default(),
            playlist_job: Default::default(),
            media_source: Default::default(),
            prefs: Prefs::default(),
            file_time_zone: jiff::tz::TimeZone::UTC,
            jobs: JobQueue::new(),
            layout: first_layout.clone(),
            workspaces: crate::workspace::Workspaces::new(&first_layout),
            unsaved_prompt: None,
            grid_selection: None,
            clipboard: None,
            clipboard_outbox: None,
            running: true,
            notice: None,
            messages: Default::default(),
            info: None,
            show_hidden: false,
            show_link_targets: true,
            path_edit: None,
            path_history: Vec::new(),
            terminals: HashMap::new(),
            terminal_selection: None,
            previews: Default::default(),
            preview_text_selection: None,
            thumbs: Default::default(),
            md_images: Default::default(),
            clipboard_out: None,
            clipboard_image: None,
            image_clip: Default::default(),
            pending_kind: None,
            about_open: false,
            updates: Default::default(),
            ffmpeg: Default::default(),
            preferences_open: false,
            dirs: DirCache::new(),
            watch: Default::default(),
            git: Default::default(),
            diffs: Default::default(),
            backends,
            fileops: FileOps::default(),
            remote,
            trash: Default::default(),
            undo: UndoStack::new(),
            devices: DeviceCache::default(),
            inbox: Arc::new(Mutex::new(HashMap::new())),
            reads: HashMap::new(),
            inflight: HashMap::new(),
            indexes: HashMap::new(),
        }
    }
}

impl Session {
    /// Register the command catalogue and load the OS timezone before the UI starts.
    pub fn new() -> Result<Self, FmError> {
        // Audio formats Symphonia cannot read fall back to ffmpeg, found like video tools.
        fm_media::set_tool_locator(fm_preview::video::find_tool);
        let (file_time_zone, notice) = match jiff::tz::TimeZone::try_system() {
            Ok(zone) => (zone, None),
            Err(err) => {
                let message = format!(
                    "Cannot detect the system timezone; file times are shown in UTC: {err}"
                );
                eprintln!("{message}");
                (jiff::tz::TimeZone::UTC, Some(message))
            }
        };
        Ok(Self {
            registry: Registry::from_specs(commands::specs())?,
            file_time_zone,
            notice,
            ..Default::default()
        })
    }

    /// The single entry point for all behaviour. Untrusted `params` are capped
    /// *before* any command sees them; an unknown id is an error, not a crash.
    pub fn dispatch(&mut self, id: &str, params: &Params) -> Result<(), FmError> {
        validate_params(params)?;
        let run = self
            .registry
            .get(id)
            .ok_or_else(|| FmError::UnknownCommand(id.to_string()))?
            .run;
        run(self, params)
    }

    /// Move a pending [`Session::notice`] into the message log; true when one was moved.
    pub fn drain_notices(&mut self) -> bool {
        match self.notice.take() {
            Some(text) => {
                self.messages.push(&text);
                true
            }
            None => false,
        }
    }

    /// Dispatch on behalf of the UI: a failure becomes the visible [`Session::notice`]
    /// instead of vanishing. Returns whether the command succeeded.
    pub fn dispatch_or_notice(&mut self, id: &str, params: &Params) -> bool {
        match self.dispatch(id, params) {
            Ok(()) => true,
            Err(err) => {
                self.notice = Some(err.to_string());
                false
            }
        }
    }

    /// `enabled` predicate for a command id (`false` when unknown).
    pub fn command_enabled(&self, id: &str) -> bool {
        self.registry
            .get(id)
            .is_some_and(|spec| (spec.enabled)(self))
    }

    /// Read `uri` unless it is already fresh or in flight. Returns the job id
    /// when a read was submitted.
    pub fn ensure_read(&mut self, uri: VfsUri) -> Result<Option<JobId>, FmError> {
        if self.volumes.unavailable(&uri) {
            if !matches!(self.dirs.state(&uri), DirState::Failed { .. }) {
                self.dirs.set_failed(
                    &uri,
                    "This volume was disconnected. Reconnect it or choose another folder.".into(),
                );
            }
            return Ok(None);
        }
        match self.dirs.state(&uri) {
            DirState::Ready { .. } | DirState::Loading { .. } => Ok(None),
            DirState::Absent | DirState::Failed { .. } => {
                if self.needs_login(&uri) || self.needs_sftp_login(&uri) {
                    return Ok(None);
                }
                self.submit_read(uri).map(Some)
            }
        }
    }

    /// Submit a directory read for `uri` on the scheme's lane. Returns
    /// immediately; dedups in-flight reads of the same uri (capability
    /// `directory-cache`).
    pub fn submit_read(&mut self, uri: VfsUri) -> Result<JobId, FmError> {
        if self.volumes.unavailable(&uri) {
            return Err(FmError::backend("devices", "This volume was disconnected"));
        }
        if self.volumes.blocks(&uri) {
            return Err(FmError::backend("devices", "This volume is being removed"));
        }
        if let Some(id) = self.inflight.get(&uri) {
            return Ok(*id);
        }
        if self.needs_login(&uri) || self.needs_sftp_login(&uri) {
            return Err(FmError::PermissionDenied(uri.to_string()));
        }
        let backend = self.backends.require(&uri)?;
        let lane = lane_for(uri.scheme());
        let inbox = self.inbox.clone();
        let lost = self.remote.lost.clone();
        let staged_uri = uri.clone();
        let job = self.jobs.submit(lane, move |_ctx| {
            // One blocking call on the lane; cancellation is checked by the
            // lane itself around the job, not inside std::fs.
            let outcome = match backend.read_dir(&staged_uri) {
                Ok(entries) => ReadOutcome::Ok(entries.into()),
                Err(err) => {
                    if crate::remote::is_transport_failure(staged_uri.scheme(), &err) {
                        lost.lock().unwrap_or_else(|p| p.into_inner()).push(
                            crate::remote::conn_key(staged_uri.scheme(), staged_uri.authority()),
                        );
                    }
                    ReadOutcome::Err(err.to_string())
                }
            };
            if let Ok(mut guard) = inbox.lock() {
                guard.insert(staged_uri.clone(), outcome);
            }
            Ok(())
        });
        self.dirs.begin_load(uri.clone(), job);
        self.reads.insert(job, uri.clone());
        self.inflight.insert(uri, job);
        Ok(job)
    }

    pub(crate) fn forget_volume_reads(&mut self, mount: &VfsUri) {
        self.reads
            .retain(|_, uri| !mount.is_same_or_descendant_of(uri));
        self.inflight
            .retain(|uri, _| !mount.is_same_or_descendant_of(uri));
        self.dirs
            .invalidate_where(|uri| mount.is_same_or_descendant_of(uri));
    }

    /// Invalidate `uri` and re-read it (nav.refresh / watcher path).
    pub fn refresh_read(&mut self, uri: VfsUri) -> Result<JobId, FmError> {
        if let Some(old) = self.inflight.remove(&uri) {
            self.reads.remove(&old);
        }
        self.dirs.invalidate(&uri);
        self.dir_changed(&uri);
        self.submit_read(uri)
    }

    /// Whether background work is in flight, so the app must keep polling without user input.
    pub fn is_busy(&self) -> bool {
        self.volumes.removing()
            || self.updates.checking()
            || self.ffmpeg.installing()
            || self.updates.browser_job.is_some()
            || !self.inflight.is_empty()
            || self.watch.has_pending()
            || self.git_busy()
            || self.diff_busy()
            || self.remote.is_busy()
            || self.previews.is_busy()
            || self.media.is_busy()
            || self.media_store_busy()
            || self.playlist_job.is_busy()
            || self.media_source.is_busy()
            || self.thumbs.is_busy()
            || self.image_clip.is_busy()
            || self.fileops.jobs().iter().any(|j| j.state.is_running())
    }

    /// Drain job events; apply staged directory reads into the cache, prune
    /// vanished selections and rebuild affected view indexes. Returns `true`
    /// when anything observable changed (the UI should repaint).
    pub fn poll(&mut self) -> bool {
        let events = self.jobs.poll();
        let (term_changed, _) = self.pump_terminals();
        let term_changed = self.close_exited_terminals() || term_changed;
        let mut changed = !events.is_empty() || term_changed;
        changed |= self.pump_updates();
        changed |= self.ffmpeg.poll();
        changed |= self.pump_devices();
        changed |= self.pump_remote_lost();
        changed |= self.pump_trash();
        changed |= self.pump_image_clip();
        changed |= self.pump_git();
        changed |= self.pump_diffs();
        self.sync_servers();
        self.normalize_sftp_panes();
        self.remote_tick();
        changed |= self.refresh_previews();
        changed |= self.media_tick();
        changed |= self.media_store_tick();
        changed |= self.media_playlist_tick();
        changed |= self.media_source_tick();
        self.reconcile_audio_owner();
        for ev in &events {
            if self.finish_device_removal(ev) {
                continue;
            }
            if self.updates.finish_browser(ev) {
                if let JobEvent::Failed { message, .. } = ev {
                    self.notice = Some(message.clone());
                }
                continue;
            }
            if ev.is_terminal() && (self.finish_op(ev) || self.finish_remote(ev)) {
                continue;
            }
            let job = match ev {
                JobEvent::Done(id) | JobEvent::Cancelled(id) => *id,
                JobEvent::Failed { id, .. } => *id,
                JobEvent::Started(_) | JobEvent::Progress { .. } | JobEvent::Conflict(_) => {
                    continue;
                }
            };
            let Some(uri) = self.reads.remove(&job) else {
                continue;
            };
            self.inflight.remove(&uri);
            let outcome = self.inbox.lock().ok().and_then(|mut g| g.remove(&uri));
            let outcome = match outcome {
                Some(o) => o,
                None => match ev {
                    JobEvent::Failed { message, .. } => ReadOutcome::Err(message.clone()),
                    // Lane died without staging: surface as a failed read.
                    _ => ReadOutcome::Err("directory read did not complete".to_string()),
                },
            };
            if self.dirs.apply(&uri, outcome).is_some() {
                self.after_read(&uri);
                changed = true;
            }
        }
        changed |= self.sync_ops();
        changed
    }

    /// Selections prune against fresh entries; indexes for panes on `uri` drop.
    pub(crate) fn after_read(&mut self, uri: &VfsUri) {
        let entries = self.dirs.entries(uri);
        // A host opened by address (`\\192.168.0.5`) joins the Network folder, so the
        // Network tree can show it even when discovery never announced it.
        if entries.is_some()
            && self
                .backends
                .network()
                .is_some_and(|net| net.remember_host(uri))
            && self.shows_network()
        {
            self.poll_location(VfsUri::network());
        }
        let panes: Vec<PaneId> = self
            .layout
            .states
            .iter()
            .filter(|(_, s)| &s.location == uri)
            .map(|(id, _)| *id)
            .collect();
        let names: Option<BTreeSet<String>> = entries
            .as_ref()
            .map(|e| e.iter().map(|en| en.name.clone()).collect());
        for pane in panes {
            self.indexes.remove(&pane);
            if let (Some(state), Some(names)) = (self.layout.state_mut(pane), names.as_ref()) {
                state.listing.prune(names);
            }
        }
    }

    /// The sorted view of the pane's directory, built lazily and cached per
    /// (pane, revision, sort). `None` while the read is loading or failed.
    pub fn view_index(&mut self, pane: PaneId) -> Option<ViewIndex> {
        let (loc, sort, filter) = {
            let state = self.layout.state(pane)?;
            let filter = state
                .listing
                .filter
                .clone()
                .filter(|f| !f.trim().is_empty());
            (state.location.clone(), state.listing.sort.clone(), filter)
        };
        let show_hidden = self.show_hidden;
        let (revision, entries) = match self.dirs.state(&loc) {
            DirState::Ready { revision, entries } => (revision, entries),
            _ => return None,
        };
        if let Some(ix) = self.indexes.get(&pane)
            && ix.revision == revision
            && ix.sort == sort
            && ix.hidden == show_hidden
            && ix.filter == filter
        {
            return Some(ix.clone());
        }
        // An unusable filter (too long) shows nothing rather than everything.
        let test = filter.as_deref().map(crate::glob::NameTest::parse);
        let mut order: Vec<u32> = (0..entries.len() as u32)
            .filter(|i| {
                entries.get(*i as usize).is_some_and(|e| {
                    (show_hidden || !is_hidden(e))
                        && match &test {
                            None => true,
                            Some(Ok(test)) => test.matches(&e.name),
                            Some(Err(_)) => false,
                        }
                })
            })
            .collect();
        order.sort_by(|a, b| {
            let ea = entries.get(*a as usize);
            let eb = entries.get(*b as usize);
            match (ea, eb) {
                (Some(a), Some(b)) => sort.compare(a, b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
        let ix = ViewIndex {
            revision,
            sort,
            hidden: show_hidden,
            filter,
            order: order.into(),
        };
        self.indexes.insert(pane, ix.clone());
        Some(ix)
    }

    /// Entries + sorted index for a pane in one call (what the listing widget wants).
    pub fn pane_view(&mut self, pane: PaneId) -> Option<(Arc<[Entry]>, ViewIndex)> {
        let ix = self.view_index(pane)?;
        let loc = self.layout.state(pane)?.location.clone();
        let entries = self.dirs.entries(&loc)?;
        Some((entries, ix))
    }

    /// Whether the pane's directory listing has arrived.
    pub fn pane_view_ready(&mut self, pane: PaneId) -> bool {
        self.view_index(pane).is_some()
    }

    /// Drop a pane's cached sort index (called when the sort spec changes).
    pub fn invalidate_view(&mut self, pane: PaneId) {
        self.indexes.remove(&pane);
    }
}

/// Hidden on any platform: a leading dot, or the backend reported the OS hidden flag.
pub fn is_hidden(entry: &fm_vfs::Entry) -> bool {
    entry.hidden || entry.name.starts_with('.')
}

/// Validate an untrusted parameter payload against the caps. Total over any input:
/// an explicit stack, no recursion, early exit as soon as a cap is exceeded.
pub fn validate_params(params: &Params) -> Result<(), FmError> {
    // Depth 0 = the payload root; a value nested in N arrays has depth N.
    let mut stack: Vec<(&Params, u32)> = vec![(params, 0)];
    let mut size: usize = 0;
    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_PARAMS_DEPTH {
            return Err(FmError::InvalidParams(
                "parameters nested too deeply".to_string(),
            ));
        }
        match value {
            Params::Null | Params::Bool(_) => size += 1,
            Params::Number(n) => size += n.to_string().len(),
            Params::String(s) => {
                size += s.len();
                if size > MAX_PARAMS_BYTES {
                    return Err(FmError::InvalidParams("parameters too large".to_string()));
                }
            }
            Params::Array(items) => {
                for item in items {
                    size += 2;
                    if size > MAX_PARAMS_BYTES {
                        return Err(FmError::InvalidParams("parameters too large".to_string()));
                    }
                    stack.push((item, depth + 1));
                }
            }
            Params::Object(map) => {
                for (key, item) in map {
                    size += key.len() + 4;
                    if size > MAX_PARAMS_BYTES {
                        return Err(FmError::InvalidParams("parameters too large".to_string()));
                    }
                    stack.push((item, depth + 1));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::CommandSpec;

    #[test]
    fn file_times_follow_os_timezone_at_startup() {
        const CHILD: &str = "FM_FILE_TIME_ZONE_TEST";
        if let Ok(case) = std::env::var(CHILD) {
            let session = Session::new().expect("session");
            let timestamp: jiff::Timestamp = "2026-10-29T20:30:00Z".parse().expect("timestamp");
            let local = session.file_time_zone.to_datetime(timestamp);
            if case == "local" {
                assert_eq!(local.to_string(), "2026-10-30T02:15:00");
                assert!(session.notice.is_none());
            } else {
                assert_eq!(local.to_string(), "2026-10-29T20:30:00");
                assert!(
                    session
                        .notice
                        .as_deref()
                        .is_some_and(|s| s.contains("file times are shown in UTC"))
                );
            }
            return;
        }
        for (case, zone) in [("local", "NPT-5:45"), ("error", "not a valid timezone")] {
            let output =
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args([
                        "--exact",
                        "session::tests::file_times_follow_os_timezone_at_startup",
                        "--nocapture",
                    ])
                    .env(CHILD, case)
                    .env("TZ", zone)
                    .output()
                    .expect("child test");
            assert!(
                output.status.success(),
                "timezone {zone}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn new_registers_the_seed_catalogue() {
        let session = Session::new().expect("session");
        for id in [
            "app.quit",
            "app.about",
            "view.theme.dark",
            "view.theme.light",
            "view.theme.system",
            "view.theme.toggle",
        ] {
            assert!(session.registry.contains(id), "missing command {id}");
        }
    }

    #[test]
    fn unknown_id_is_an_error_and_session_is_unchanged() {
        let mut session = Session::new().expect("session");
        let before = session.prefs.clone();
        let err = session
            .dispatch("does.not.exist", &Params::Null)
            .expect_err("unknown id");
        assert!(matches!(err, FmError::UnknownCommand(_)));
        assert!(err.to_string().contains("does.not.exist"));
        assert_eq!(session.prefs, before, "session must be unchanged");
        assert!(session.running);
    }

    /// A params doc whose every field is marked `name?:` accepts empty params.
    fn all_optional(doc: &str) -> bool {
        doc.split(',').all(|field| field.contains("?:"))
    }

    #[test]
    fn empty_params_are_accepted_for_seed_commands() {
        let mut session = Session::new().expect("session");
        for id in session.registry.ids().collect::<Vec<_>>() {
            let needs_params = session
                .registry
                .get(id)
                .is_some_and(|spec| !spec.params.starts_with("none") && !all_optional(spec.params));
            let enabled = session
                .registry
                .get(id)
                .is_some_and(|spec| (spec.enabled)(&session));
            let result = session.dispatch(id, &Params::Null);
            if !enabled && result.is_err() {
                continue;
            }
            if needs_params {
                // Commands with required params must fail gracefully, not panic.
                assert!(result.is_err(), "dispatch({id}) should reject Null params");
            } else {
                result.unwrap_or_else(|e| panic!("dispatch({id}) failed: {e}"));
            }
        }
    }

    #[test]
    fn params_over_one_mib_are_rejected_before_running() {
        let huge = Params::String("a".repeat(MAX_PARAMS_BYTES + 1));
        let err = validate_params(&huge).expect_err("too large");
        assert!(matches!(err, FmError::InvalidParams(_)));
    }

    #[test]
    fn nesting_over_32_is_rejected() {
        let mut deep = Params::Null;
        for _ in 0..(MAX_PARAMS_DEPTH + 1) {
            deep = Params::Array(vec![deep]);
        }
        let err = validate_params(&deep).expect_err("too deep");
        assert!(matches!(err, FmError::InvalidParams(_)));

        let mut ok = Params::Null;
        for _ in 0..MAX_PARAMS_DEPTH {
            ok = Params::Array(vec![ok]);
        }
        validate_params(&ok).expect("depth 32 is fine");
    }

    #[test]
    fn wide_but_small_arrays_are_fine() {
        let wide = Params::Array(vec![Params::Null; 1000]);
        validate_params(&wide).expect("small array");
    }

    #[test]
    fn command_enabled_is_total() {
        let session = Session::new().expect("session");
        assert!(session.command_enabled("app.quit"));
        assert!(!session.command_enabled("no.such.command"));
    }

    #[test]
    fn test_command_can_enqueue_a_job() {
        let mut session = Session::new().expect("session");
        session
            .registry
            .register(CommandSpec {
                id: "test.enqueue",
                label: "Test enqueue",
                menu: &["Test"],
                shortcut: None,
                params: "none",
                enabled: |_s| true,
                run: |s, _p| {
                    let id = s.jobs.submit(crate::jobs::JobLane::Local, |ctx| {
                        ctx.report(1, 1);
                        Ok(())
                    });
                    assert!(id > 0);
                    Ok(())
                },
            })
            .expect("register");
        session
            .dispatch("test.enqueue", &Params::Null)
            .expect("dispatch");
        // The job runs in the background; the dispatch call already returned.
        let terminal = session
            .jobs
            .wait_for_terminal(1, std::time::Duration::from_secs(5));
        assert!(matches!(terminal, Some(crate::jobs::JobEvent::Done(_))));
    }

    // ---- directory-cache integration (capability `directory-cache`) ----

    use fm_testkit::MemFs;
    use fm_vfs::{Scheme, SpaceInfo, Vfs, WriteOpts};
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// A `MemFs` whose `read_dir` sleeps first — proves the UI path never blocks.
    struct SlowFs {
        inner: MemFs,
        delay: Duration,
    }

    impl Vfs for SlowFs {
        fn scheme(&self) -> Scheme {
            Scheme::File
        }
        fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
            std::thread::sleep(self.delay);
            self.inner.read_dir(uri)
        }
        fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
            self.inner.stat(uri)
        }
        fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
            self.inner.open_read(uri)
        }
        fn open_write(
            &self,
            uri: &VfsUri,
            opts: WriteOpts,
        ) -> Result<Box<dyn Write + Send>, FmError> {
            self.inner.open_write(uri, opts)
        }
        fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
            self.inner.create_dir(uri)
        }
        fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
            self.inner.remove(uri)
        }
        fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
            self.inner.rename(from, to)
        }
        fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
            self.inner.same_device(a, b)
        }
        fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
            self.inner.space(uri)
        }
    }

    fn mem_session() -> (Session, MemFs, VfsUri) {
        let mut s = Session::new().expect("session");
        let mem = MemFs::new();
        mem.mkdirs_str("/data").expect("mkdirs");
        mem.put_str("/data/b.txt", b"bb").expect("put");
        mem.put_str("/data/a.txt", b"a").expect("put");
        mem.mkdirs_str("/data/sub").expect("mkdirs");
        s.backends.register(Arc::new(mem.clone()));
        let uri = VfsUri::parse("/data").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        (s, mem, uri)
    }

    fn wait_ready(s: &mut Session, uri: &VfsUri) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs.state(uri).is_ready() {
                return;
            }
            assert!(Instant::now() < deadline, "read never became ready");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn read_returns_before_entries_land_and_then_applies() {
        let mut s = Session::new().expect("session");
        let mem = MemFs::new();
        mem.mkdirs_str("/slow").expect("mkdirs");
        mem.put_str("/slow/x", b"1").expect("put");
        s.backends.register(Arc::new(SlowFs {
            inner: mem,
            delay: Duration::from_millis(150),
        }));
        let uri = VfsUri::parse("/slow").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        // Immediately: loading, no view — the submit path returned already.
        assert!(matches!(s.dirs.state(&uri), DirState::Loading { .. }));
        assert!(s.pane_view(1).is_none());
        wait_ready(&mut s, &uri);
        let (entries, ix) = s.pane_view(1).expect("view");
        assert_eq!(entries.len(), 1);
        assert_eq!(ix.order.len(), 1);
    }

    #[test]
    fn inflight_reads_of_the_same_uri_are_deduped() {
        let (mut s, _mem, uri) = mem_session();
        let a = s.submit_read(uri.clone()).expect("a");
        let b = s.submit_read(uri.clone()).expect("b");
        assert_eq!(a, b, "second submit must reuse the in-flight job");
    }

    #[test]
    fn failed_read_surfaces_and_view_stays_none() {
        let (mut s, _mem, _uri) = mem_session();
        let missing = VfsUri::parse("/nope").expect("uri");
        s.layout.state_mut(1).expect("pane").location = missing.clone();
        s.submit_read(missing.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if let DirState::Failed { message, .. } = s.dirs.state(&missing) {
                assert!(message.contains("not found"), "{message}");
                break;
            }
            assert!(Instant::now() < deadline, "failure never landed");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(s.pane_view(1).is_none());
    }

    #[test]
    fn poll_prunes_selection_of_vanished_entries() {
        let (mut s, _mem, uri) = mem_session();
        s.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut s, &uri);
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.listing.select_only(Some("a.txt"), 0);
            pane.listing.toggle("b.txt", 1);
        }
        assert_eq!(s.layout.state(1).expect("pane").listing.selected_count(), 2);
        let backend = s.backends.require(&uri).expect("backend");
        backend
            .remove(&uri.join("a.txt").expect("join"))
            .expect("remove a.txt");
        s.refresh_read(uri.clone()).expect("refresh");
        wait_ready(&mut s, &uri);
        let listing = &s.layout.state(1).expect("pane").listing;
        assert!(!listing.is_selected("a.txt"), "vanished name pruned");
        assert!(listing.is_selected("b.txt"), "survivor kept");
    }

    #[test]
    fn toggling_hidden_keeps_the_cursor_on_the_same_entry() {
        let (mut s, mem, uri) = mem_session();
        mem.put_str(&format!("{}/.aaa", uri.path_str()), b"x").ok();
        mem.put_str(&format!("{}/zzz_target", uri.path_str()), b"x")
            .ok();
        s.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut s, &uri);
        let name_at = |s: &mut Session| {
            let row = s.layout.state(1).expect("pane").listing.focused_row;
            let (entries, ix) = s.pane_view(1).expect("view");
            let i = *ix.order.get(row).expect("row");
            entries.get(i as usize).expect("entry").name.clone()
        };
        let (entries, ix) = s.pane_view(1).expect("view");
        let row = ix
            .order
            .iter()
            .position(|i| {
                entries
                    .get(*i as usize)
                    .is_some_and(|e| e.name == "zzz_target")
            })
            .expect("target visible");
        s.layout.state_mut(1).expect("pane").listing.focused_row = row;
        s.dispatch("view.hidden.toggle", &serde_json::Value::Null)
            .expect("on");
        assert_eq!(name_at(&mut s), "zzz_target");
        s.dispatch("view.hidden.toggle", &serde_json::Value::Null)
            .expect("off");
        assert_eq!(name_at(&mut s), "zzz_target");
    }

    #[test]
    fn hidden_entries_are_filtered_until_toggled() {
        let (mut s, mem, uri) = mem_session();
        mem.put_str("/d/.secret", b"x").ok();
        mem.put_str(&format!("{}/.secret", uri.path_str()), b"x")
            .ok();
        s.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut s, &uri);
        let count = |s: &mut Session| s.view_index(1).expect("ix").order.len();
        let hidden_off = count(&mut s);
        s.show_hidden = true;
        assert!(count(&mut s) >= hidden_off);
        let (entries, ix) = s.pane_view(1).expect("view");
        let names: Vec<&str> = ix
            .order
            .iter()
            .filter_map(|i| entries.get(*i as usize))
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&".secret"));
        s.show_hidden = false;
        let (entries, ix) = s.pane_view(1).expect("view");
        assert!(
            !ix.order
                .iter()
                .filter_map(|i| entries.get(*i as usize))
                .any(|e| e.name == ".secret")
        );
    }

    #[test]
    fn view_index_sorts_by_the_pane_spec() {
        let (mut s, _mem, uri) = mem_session();
        s.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut s, &uri);
        // Default sort: dirs first, then name → sub, a.txt, b.txt
        let ix = s.view_index(1).expect("index");
        let entries = s.dirs.entries(&uri).expect("entries");
        let names: Vec<_> = ix
            .order
            .iter()
            .map(|i| entries.get(*i as usize).expect("entry").name.as_str())
            .collect();
        assert_eq!(names, vec!["sub", "a.txt", "b.txt"]);
        // Switch to size sort: a.txt(1) before b.txt(2); dir size 0 first
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.listing.sort.key = fm_layout::SortKey::Size;
        }
        s.invalidate_view(1);
        let ix2 = s.view_index(1).expect("index2");
        let names2: Vec<_> = ix2
            .order
            .iter()
            .map(|i| entries.get(*i as usize).expect("entry").name.as_str())
            .collect();
        assert_eq!(names2, vec!["sub", "a.txt", "b.txt"]);
        // b.txt bigger than a.txt → descending size puts b first among files
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.listing.sort.dirs_first = false;
            pane.listing.sort.dir = fm_layout::SortDir::Desc;
        }
        s.invalidate_view(1);
        let ix3 = s.view_index(1).expect("index3");
        let names3: Vec<_> = ix3
            .order
            .iter()
            .map(|i| entries.get(*i as usize).expect("entry").name.as_str())
            .collect();
        assert_eq!(names3, vec!["b.txt", "a.txt", "sub"]);
    }

    #[test]
    fn scheme_without_a_backend_is_an_error() {
        let s = Session::new().expect("session");
        // No SFTP backend registered in a fresh session.
        assert!(s.backends.get(Scheme::Sftp).is_none());
        let mut s = s;
        let err = mut_session_sftp_read(&mut s);
        // The read asks for a login instead of failing outright.
        assert!(matches!(err, FmError::PermissionDenied(_)), "{err:?}");
        assert!(s.remote.sftp_prompt.is_some());
    }

    fn mut_session_sftp_read(s: &mut Session) -> FmError {
        let uri = VfsUri::parse("sftp://host/dir").expect("uri");
        s.submit_read(uri).expect_err("no backend")
    }
}
