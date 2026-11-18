//! Folder sync state (capability `file-sync`): saved profiles, one [`SyncState`] per profile
//! (latest plan, progress, report, watch), and the background tasks behind them.
//!
//! Scans and runs execute on a dedicated thread per task (like Find Files) and publish into a
//! shared slot; only [`Session::pump_sync`] applies results, so the UI thread never waits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use fm_ops::CancelFlag;
use fm_sync::{
    Plan, Profile, RunOptions, RunReport, Side, SnapshotStore, Status, SyncProgress, compare,
    profile::fresh_id,
};
use fm_vfs::{FmError, Scheme};
use notify::{RecursiveMode, Watcher as _};

use crate::fileops::SpeedMeter;
use crate::remote::ConnState;
use crate::session::Session;

const PROFILES_FILE: &str = "sync-profiles.json";
const SNAPSHOT_DIR: &str = "sync";
const MAX_PROFILES: usize = 256;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Scans and runs allowed at once across all profiles.
pub const MAX_TASKS: usize = 4;
const QUIET: Duration = Duration::from_millis(1500);
const MAX_WAIT: Duration = Duration::from_secs(10);

/// Turning on "Keep in sync": compare first, then wait for the user's go-ahead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Arming {
    #[default]
    Off,
    Comparing,
    Confirm,
}

/// Which of the two sync workflows the window shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// Guided questions, one screen at a time.
    #[default]
    Wizard,
    /// Every option on one screen.
    Advanced,
}

/// The wizard's last screen (the review); screens before it ask the questions.
pub const WIZARD_REVIEW: u8 = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Comparing { left: usize, right: usize },
    Ready,
    Running(SyncProgress),
    Failed(String),
}

/// Which rows the grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowFilter {
    #[default]
    All,
    /// Everything that is not equal.
    Changes,
    Conflicts,
    Equal,
}

impl RowFilter {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "all" => Self::All,
            "changes" => Self::Changes,
            "conflicts" => Self::Conflicts,
            "equal" => Self::Equal,
            _ => return None,
        })
    }

    fn shows(self, status: Status) -> bool {
        match self {
            Self::All => true,
            Self::Changes => status != Status::Equal,
            Self::Conflicts => matches!(status, Status::Conflict | Status::Unknown),
            Self::Equal => status == Status::Equal,
        }
    }
}

/// How the grid orders rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    /// Folder tree order (parents before their children).
    #[default]
    Path,
    Name,
    Size,
    Modified,
    Action,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "path" => Self::Path,
            "name" => Self::Name,
            "size" => Self::Size,
            "modified" => Self::Modified,
            "action" => Self::Action,
            _ => return None,
        })
    }

    /// Only the tree order keeps folders above their contents.
    pub fn is_tree(self) -> bool {
        self == Self::Path
    }
}

fn action_rank(a: fm_sync::Action) -> u8 {
    use fm_sync::Action as A;
    match a {
        A::CopyToRight => 0,
        A::CopyToLeft => 1,
        A::DeleteLeft => 2,
        A::DeleteRight => 3,
        A::Skip => 4,
        A::None => 5,
    }
}

fn leaf(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

enum Follow {
    Watch,
    AutoApply,
    Refresh,
}

enum TaskResult {
    Compared(Result<Box<Plan>, FmError>),
    Ran(Result<RunReport, FmError>),
}

#[derive(Default)]
struct Shared {
    scanned: (usize, usize),
    progress: SyncProgress,
    result: Option<TaskResult>,
}

struct Task {
    shared: Arc<Mutex<Shared>>,
    cancel: CancelFlag,
    running: bool,
    /// Started by the watcher with auto-apply on.
    auto: bool,
}

struct WatchState {
    dirty: Arc<AtomicBool>,
    first: Option<Instant>,
    last: Option<Instant>,
    next_poll: Option<Instant>,
    interval: Duration,
    /// Held so the OS watches stay alive.
    _watchers: Vec<notify::RecommendedWatcher>,
}

/// How a running sync is going: time spent, smoothed speed and an estimate of what is left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunStats {
    pub elapsed: Duration,
    pub bytes_per_sec: Option<f64>,
    pub eta_secs: Option<u64>,
}

/// Start time and speed sampling of the current run.
#[derive(Debug, Default)]
struct RunTiming {
    started: Option<Instant>,
    speed: SpeedMeter,
}

pub struct SyncState {
    /// Working copy; edited by `sync.profile.set`, stored by `sync.profile.save`.
    pub profile: Profile,
    pub saved: bool,
    pub phase: Phase,
    pub plan: Option<Plan>,
    pub report: Option<RunReport>,
    pub filter: RowFilter,
    /// Show only rows with this action (the Action column's filter).
    pub action_filter: Option<fm_sync::Action>,
    pub sort: SortKey,
    pub descending: bool,
    /// Plan indices shown under the current filter.
    pub rows: Vec<u32>,
    /// Last watch/auto-apply message for the user.
    pub watch_note: Option<String>,
    /// The run needs the user to confirm deletes.
    pub confirm: bool,
    pub arming: Arming,
    pub view: View,
    /// Current wizard screen, `0..=WIZARD_REVIEW`.
    pub step: u8,
    /// Compare pressed while a folder's server was not connected: it runs after sign-in.
    pub awaiting_login: bool,
    timing: RunTiming,
    task: Option<Task>,
    watch: Option<WatchState>,
}

impl SyncState {
    fn new(profile: Profile, saved: bool) -> Self {
        Self {
            profile,
            saved,
            phase: Phase::Idle,
            plan: None,
            report: None,
            filter: RowFilter::default(),
            action_filter: None,
            sort: SortKey::default(),
            descending: false,
            rows: Vec::new(),
            watch_note: None,
            confirm: false,
            arming: Arming::Off,
            view: View::Wizard,
            step: 0,
            awaiting_login: false,
            timing: RunTiming::default(),
            task: None,
            watch: None,
        }
    }

    /// Elapsed time, speed and time left while a run is in progress.
    pub fn run_stats(&self) -> Option<RunStats> {
        let Phase::Running(p) = &self.phase else {
            return None;
        };
        let started = self.timing.started?;
        Some(RunStats {
            elapsed: started.elapsed(),
            bytes_per_sec: self.timing.speed.bytes_per_sec(),
            eta_secs: self
                .timing
                .speed
                .eta_secs(p.bytes_total.saturating_sub(p.bytes_done)),
        })
    }

    pub fn busy(&self) -> bool {
        self.task.as_ref().is_some_and(|t| t.running)
    }

    pub fn watching(&self) -> bool {
        self.watch.is_some()
    }

    pub fn refresh_rows(&mut self) {
        let (filter, action) = (self.filter, self.action_filter);
        let Some(plan) = self.plan.as_ref() else {
            self.rows.clear();
            return;
        };
        let mut rows: Vec<u32> = plan
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| filter.shows(i.status))
            .filter(|(_, i)| action.is_none_or(|a| i.action == a))
            .filter_map(|(n, _)| u32::try_from(n).ok())
            .collect();
        let item = |n: &u32| plan.items.get(usize::try_from(*n).unwrap_or(usize::MAX));
        let size = |n: &u32| {
            item(n).map_or(0, |i| {
                i.left
                    .iter()
                    .chain(i.right.iter())
                    .map(|x| x.size)
                    .max()
                    .unwrap_or(0)
            })
        };
        let time = |n: &u32| {
            item(n).and_then(|i| {
                i.left
                    .iter()
                    .chain(i.right.iter())
                    .filter_map(|x| x.mtime)
                    .max()
            })
        };
        match self.sort {
            SortKey::Path => {}
            SortKey::Name => rows.sort_by_cached_key(|n| {
                item(n)
                    .map(|i| leaf(&i.rel).to_lowercase())
                    .unwrap_or_default()
            }),
            SortKey::Size => rows.sort_by_key(size),
            SortKey::Modified => rows.sort_by_key(time),
            SortKey::Action => rows.sort_by_key(|n| item(n).map_or(0, |i| action_rank(i.action))),
        }
        if self.descending {
            rows.reverse();
        }
        self.rows = rows;
    }
}

#[derive(Default)]
pub struct SyncHub {
    pub profiles: Vec<Profile>,
    /// Profile shown in the sync window; `None` while it is closed.
    pub current: Option<String>,
    /// The window is hidden and the sync lives in the Activity panel.
    pub minimized: bool,
    pub states: BTreeMap<String, SyncState>,
    config_dir: Option<PathBuf>,
}

impl SyncHub {
    /// Point the hub at the config directory and load the saved profiles.
    pub fn set_config_dir(&mut self, dir: PathBuf) -> Option<String> {
        let (profiles, problem) = load_profiles(&dir);
        self.profiles = profiles;
        self.config_dir = Some(dir);
        problem
    }

    pub fn store(&self) -> Option<SnapshotStore> {
        self.config_dir
            .as_ref()
            .map(|d| SnapshotStore::new(d.join(SNAPSHOT_DIR)))
    }

    pub fn state(&self, id: &str) -> Option<&SyncState> {
        self.states.get(id)
    }

    pub fn current_state(&self) -> Option<&SyncState> {
        self.current.as_deref().and_then(|id| self.states.get(id))
    }

    pub(crate) fn ids(&self) -> std::collections::BTreeSet<String> {
        self.profiles
            .iter()
            .map(|p| p.id.clone())
            .chain(self.states.keys().cloned())
            .collect()
    }

    /// Put (a copy of) a saved profile or a fresh draft into view.
    pub(crate) fn open_profile(&mut self, profile: Profile, saved: bool) {
        let id = profile.id.clone();
        self.states.entry(id.clone()).or_insert_with(|| {
            let mut st = SyncState::new(profile, saved);
            // Saved profiles are already set up; new ones start with the questions.
            if saved {
                st.view = View::Advanced;
            }
            st
        });
        self.current = Some(id);
        self.minimized = false;
    }

    /// Whether any sync is scanning or copying.
    pub fn any_busy(&self) -> bool {
        self.states.values().any(SyncState::busy)
    }

    /// When the UI must wake without input: soon while working, slowly while watching.
    pub fn wake(&self) -> Option<Duration> {
        if self.any_busy() {
            Some(Duration::from_millis(100))
        } else if self.states.values().any(SyncState::watching) {
            Some(Duration::from_millis(500))
        } else {
            None
        }
    }

    pub(crate) fn persist(&self) -> Result<(), FmError> {
        let Some(dir) = &self.config_dir else {
            return Ok(());
        };
        let file = StoredProfiles {
            version: 1,
            profiles: self.profiles.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|e| FmError::backend("sync", format!("profiles encode: {e}")))?;
        crate::prefs::write_atomic(dir, PROFILES_FILE, &bytes)
    }

    pub(crate) fn upsert(&mut self, profile: Profile) -> Result<(), FmError> {
        profile.validate()?;
        match self.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(slot) => *slot = profile,
            None => {
                if self.profiles.len() >= MAX_PROFILES {
                    return Err(FmError::InvalidParams(format!(
                        "at most {MAX_PROFILES} sync profiles"
                    )));
                }
                self.profiles.push(profile);
            }
        }
        Ok(())
    }

    fn running_tasks(&self) -> usize {
        self.states.values().filter(|s| s.busy()).count()
    }

    pub(crate) fn fresh_id(&self) -> String {
        fresh_id(&self.ids())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct StoredProfiles {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    profiles: Vec<Profile>,
}

fn load_profiles(dir: &Path) -> (Vec<Profile>, Option<String>) {
    let path = dir.join(PROFILES_FILE);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return (Vec::new(), None),
    };
    if meta.len() > MAX_FILE_BYTES {
        return (
            Vec::new(),
            Some("The saved sync profiles file is too large and was ignored".into()),
        );
    }
    let parsed = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<StoredProfiles>(&b).ok());
    let Some(file) = parsed else {
        return (
            Vec::new(),
            Some("The saved sync profiles could not be read and were ignored".into()),
        );
    };
    let mut seen = std::collections::BTreeSet::new();
    let profiles: Vec<Profile> = file
        .profiles
        .into_iter()
        .filter(|p| seen.insert(p.id.clone()))
        .take(MAX_PROFILES)
        .collect();
    (profiles, None)
}

impl Session {
    /// Scan both folders and build a plan (never writes). `auto` marks a watcher-started run.
    pub(crate) fn sync_compare(&mut self, id: &str, auto: bool) -> Result<(), FmError> {
        if self.sync.running_tasks() >= MAX_TASKS {
            return Err(FmError::InvalidParams(
                "too many syncs are running; wait for one to finish".into(),
            ));
        }
        if !auto && self.sync_sign_in_first(id)? {
            return Ok(());
        }
        let store = self.sync.store();
        let state = self
            .sync
            .states
            .get_mut(id)
            .ok_or_else(|| FmError::NotFound("sync profile".into()))?;
        if state.busy() {
            return Err(FmError::InvalidParams(
                "this sync is already running".into(),
            ));
        }
        if !auto {
            state.action_filter = None;
        }
        let profile = state.profile.clone();
        profile.validate()?;
        let (lu, ru) = (profile.left_uri()?, profile.right_uri()?);
        let (left, right) = (self.backends.require(&lu)?, self.backends.require(&ru)?);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = CancelFlag::new();
        let (t_shared, t_cancel) = (shared.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("fm-sync-scan".into())
            .spawn(move || {
                let mut on = |side: Side, n: usize| {
                    let mut g = lock(&t_shared);
                    match side {
                        Side::Left => g.scanned.0 = n,
                        Side::Right => g.scanned.1 = n,
                    }
                };
                let res = compare(&profile, &left, &right, store.as_ref(), &t_cancel, &mut on);
                lock(&t_shared).result = Some(TaskResult::Compared(res.map(Box::new)));
            })
            .map_err(FmError::Io)?;
        let state = self.sync.states.get_mut(id);
        if let Some(state) = state {
            state.phase = Phase::Comparing { left: 0, right: 0 };
            state.confirm = false;
            state.task = Some(Task {
                shared,
                cancel,
                running: true,
                auto,
            });
        }
        Ok(())
    }

    /// Execute the current plan. Deletes that need confirmation are refused unless `confirmed`.
    pub(crate) fn sync_run(&mut self, id: &str, confirmed: bool) -> Result<(), FmError> {
        if self.sync.running_tasks() >= MAX_TASKS {
            return Err(FmError::InvalidParams(
                "too many syncs are running; wait for one to finish".into(),
            ));
        }
        let store = self.sync.store();
        let state = self
            .sync
            .states
            .get_mut(id)
            .ok_or_else(|| FmError::NotFound("sync profile".into()))?;
        if state.busy() {
            return Err(FmError::InvalidParams(
                "this sync is already running".into(),
            ));
        }
        let plan = state
            .plan
            .clone()
            .ok_or_else(|| FmError::InvalidParams("compare the folders first".into()))?;
        if plan.guards.confirmation_required() && !confirmed {
            state.confirm = true;
            return Ok(());
        }
        let (left, right) = (
            self.backends.require(&plan.left_root)?,
            self.backends.require(&plan.right_root)?,
        );
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = CancelFlag::new();
        let (t_shared, t_cancel) = (shared.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("fm-sync-run".into())
            .spawn(move || {
                let opts = RunOptions {
                    confirmed,
                    now: SystemTime::now(),
                    cancel: t_cancel,
                    store: store.as_ref(),
                };
                let mut on = |p: &SyncProgress| lock(&t_shared).progress = *p;
                let res = fm_sync::run(&plan, left, right, opts, &mut on);
                lock(&t_shared).result = Some(TaskResult::Ran(res));
            })
            .map_err(FmError::Io)?;
        if let Some(state) = self.sync.states.get_mut(id) {
            state.confirm = false;
            state.phase = Phase::Running(SyncProgress::default());
            state.timing = RunTiming {
                started: Some(Instant::now()),
                speed: SpeedMeter::default(),
            };
            state.task = Some(Task {
                shared,
                cancel,
                running: true,
                auto: false,
            });
        }
        Ok(())
    }

    pub(crate) fn sync_cancel(&mut self, id: &str) {
        if let Some(t) = self.sync.states.get(id).and_then(|s| s.task.as_ref()) {
            t.cancel.cancel();
        }
    }

    pub(crate) fn sync_watch(&mut self, id: &str, on: bool) -> Result<(), FmError> {
        let state = self
            .sync
            .states
            .get_mut(id)
            .ok_or_else(|| FmError::NotFound("sync profile".into()))?;
        if !on {
            state.watch = None;
            state.watch_note = None;
            state.arming = Arming::Off;
            return Ok(());
        }
        state.profile.validate()?;
        let (lu, ru) = (state.profile.left_uri()?, state.profile.right_uri()?);
        let dirty = Arc::new(AtomicBool::new(false));
        let mut watchers = Vec::new();
        let mut remote = false;
        let mut note = None;
        for uri in [&lu, &ru] {
            let local = uri.scheme() == Scheme::File
                && uri.local_path_string().is_some()
                && !self
                    .backends
                    .require(uri)
                    .is_ok_and(|v| !v.is_native_local());
            let path = uri.local_path_string().filter(|_| local);
            let Some(path) = path else {
                remote = true;
                continue;
            };
            let flag = dirty.clone();
            let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                // Reading a folder (our own scan) raises Access events; reacting to them loops.
                if res.is_ok_and(|e| !matches!(e.kind, notify::EventKind::Access(_))) {
                    flag.store(true, Ordering::Relaxed);
                }
            })
            .and_then(|mut w| {
                w.watch(Path::new(&path), RecursiveMode::Recursive)
                    .map(|()| w)
            });
            match w {
                Ok(w) => watchers.push(w),
                Err(_) => {
                    // No OS watch (limit reached, network mount): poll instead.
                    remote = true;
                    note = Some("Live watching is unavailable here; checking periodically".into());
                }
            }
        }
        let interval = Duration::from_secs(u64::from(state.profile.watch.poll_secs));
        state.watch_note = note;
        state.watch = Some(WatchState {
            dirty,
            first: None,
            last: None,
            next_poll: remote.then(|| Instant::now() + interval),
            interval,
            _watchers: watchers,
        });
        Ok(())
    }

    /// A folder on a server that is not connected yet: open its sign-in (or reconnect
    /// with remembered credentials) and remember to compare once it is connected,
    /// instead of failing with "no backend registered". Returns whether it is waiting.
    fn sync_sign_in_first(&mut self, id: &str) -> Result<bool, FmError> {
        let Some(state) = self.sync.states.get(id) else {
            return Err(FmError::NotFound("sync profile".into()));
        };
        let profile = state.profile.clone();
        profile.validate()?;
        let mut waiting = false;
        for uri in [profile.left_uri()?, profile.right_uri()?] {
            if self.needs_sftp_login_to(&uri, false) || self.needs_login_to(&uri, false) {
                waiting = true;
            }
        }
        if let Some(state) = self.sync.states.get_mut(id) {
            state.awaiting_login = waiting;
            if waiting {
                state.phase = Phase::Idle;
            }
        }
        Ok(waiting)
    }

    /// Start compares that were waiting for a sign-in once both folders are reachable;
    /// a cancelled or failed sign-in ends the wait with an error in the sync window.
    fn resume_compares_after_login(&mut self) -> bool {
        let waiting: Vec<String> = self
            .sync
            .states
            .iter()
            .filter(|(_, st)| st.awaiting_login)
            .map(|(id, _)| id.clone())
            .collect();
        if waiting.is_empty() {
            return false;
        }
        let prompt_open = self.remote.login_prompt.is_some() || self.remote.sftp_prompt.is_some();
        for id in waiting {
            let Some(profile) = self.sync.states.get(&id).map(|st| st.profile.clone()) else {
                continue;
            };
            let (Ok(lu), Ok(ru)) = (profile.left_uri(), profile.right_uri()) else {
                if let Some(st) = self.sync.states.get_mut(&id) {
                    st.awaiting_login = false;
                }
                continue;
            };
            let remote = |u: &fm_vfs::VfsUri| matches!(u.scheme(), Scheme::Sftp | Scheme::Smb);
            let missing: Vec<&fm_vfs::VfsUri> = [&lu, &ru]
                .into_iter()
                .filter(|u| remote(u) && self.backends.get_for(u).is_none())
                .collect();
            if missing.is_empty() {
                if let Some(st) = self.sync.states.get_mut(&id) {
                    st.awaiting_login = false;
                }
                if let Err(e) = self.sync_compare(&id, false)
                    && let Some(st) = self.sync.states.get_mut(&id)
                {
                    st.phase = Phase::Failed(e.to_string());
                }
                continue;
            }
            let connecting = missing.iter().any(|u| {
                matches!(
                    self.remote.state(u.scheme(), u.authority()),
                    Some(ConnState::Connecting)
                )
            });
            if !prompt_open && !connecting {
                let host = missing
                    .first()
                    .map(|u| u.authority().host.clone())
                    .unwrap_or_default();
                if let Some(st) = self.sync.states.get_mut(&id) {
                    st.awaiting_login = false;
                    st.phase = Phase::Failed(format!(
                        "Not connected to {host}: sign in, then press Compare again"
                    ));
                }
            }
        }
        true
    }

    /// Apply finished tasks and let watchers start new ones. Returns whether anything changed.
    pub(crate) fn pump_sync(&mut self) -> bool {
        if self.sync.states.is_empty() {
            return false;
        }
        let mut changed = self.resume_compares_after_login();
        let ids: Vec<String> = self.sync.states.keys().cloned().collect();
        let mut follow: Vec<(String, Follow)> = Vec::new();
        let mut notices: Vec<String> = Vec::new();
        let mut infos: Vec<String> = Vec::new();
        for id in &ids {
            let Some(state) = self.sync.states.get_mut(id) else {
                continue;
            };
            if let Some(task) = &state.task {
                let mut g = lock(&task.shared);
                match &mut state.phase {
                    Phase::Comparing { left, right } if (*left, *right) != g.scanned => {
                        (*left, *right) = g.scanned;
                        changed = true;
                    }
                    Phase::Running(p) => {
                        state
                            .timing
                            .speed
                            .sample(Instant::now(), g.progress.bytes_done);
                        if *p != g.progress {
                            *p = g.progress;
                            changed = true;
                        }
                    }
                    _ => {}
                }
                let result = g.result.take();
                drop(g);
                let auto = task.auto;
                if let Some(result) = result {
                    state.task = None;
                    changed = true;
                    match result {
                        TaskResult::Compared(Ok(plan)) => {
                            state.plan = Some(*plan);
                            state.phase = Phase::Ready;
                            state.refresh_rows();
                            if state.arming == Arming::Comparing {
                                state.arming = Arming::Confirm;
                                // The user is about to approve this list: show what changes.
                                state.filter = RowFilter::Changes;
                                state.refresh_rows();
                            }
                            if auto {
                                follow.push((id.clone(), Follow::AutoApply));
                            }
                        }
                        TaskResult::Compared(Err(FmError::Cancelled)) => {
                            // Cancelling the compare that "Keep in sync" started undoes the choice.
                            if state.arming == Arming::Comparing {
                                state.profile.watch.enabled = false;
                            }
                            state.arming = Arming::Off;
                            state.phase = if state.plan.is_some() {
                                Phase::Ready
                            } else {
                                Phase::Idle
                            };
                        }
                        TaskResult::Compared(Err(e)) => {
                            if state.arming == Arming::Comparing {
                                state.profile.watch.enabled = false;
                            }
                            state.arming = Arming::Off;
                            state.phase = Phase::Failed(e.to_string());
                            if auto {
                                notices.push(format!("Sync \"{}\": {e}", state.profile.name));
                            }
                        }
                        TaskResult::Ran(Ok(report)) => {
                            let bad = report.failed.len();
                            let summary = format!(
                                "Sync \"{}\": {} copied, {} deleted, {} failed",
                                state.profile.name, report.copied, report.deleted, bad
                            );
                            if bad > 0 {
                                notices.push(summary);
                            } else if report.cancelled {
                                // Stopping it yourself is not an error.
                                infos.push(format!("{summary} (stopped)"));
                            }
                            state.report = Some(report);
                            state.plan = None;
                            state.rows.clear();
                            state.phase = Phase::Idle;
                            // Show what is left after the run.
                            follow.push((id.clone(), Follow::Refresh));
                        }
                        TaskResult::Ran(Err(FmError::Cancelled)) => {
                            state.phase = Phase::Idle;
                            infos.push(format!("Sync \"{}\" stopped", state.profile.name));
                        }
                        TaskResult::Ran(Err(e)) => {
                            state.phase = Phase::Failed(e.to_string());
                            notices.push(format!("Sync \"{}\": {e}", state.profile.name));
                        }
                    }
                }
            }
            // Watcher: debounce events, or poll on the interval.
            let Some(state) = self.sync.states.get_mut(id) else {
                continue;
            };
            let now = Instant::now();
            let idle = !state.busy();
            if let Some(w) = &mut state.watch {
                if w.dirty.swap(false, Ordering::Relaxed) {
                    w.first.get_or_insert(now);
                    w.last = Some(now);
                }
                let quiet = w.last.is_some_and(|l| now.duration_since(l) >= QUIET)
                    || w.first.is_some_and(|f| now.duration_since(f) >= MAX_WAIT);
                let polled = w.next_poll.is_some_and(|t| now >= t);
                if idle && (quiet || polled) {
                    w.first = None;
                    w.last = None;
                    if polled {
                        w.next_poll = Some(now + w.interval);
                    }
                    follow.push((id.clone(), Follow::Watch));
                    state.watch_note = None;
                }
            }
        }
        for (id, what) in follow {
            changed = true;
            match what {
                Follow::Watch => {
                    if let Err(e) = self.sync_compare(&id, true) {
                        notices.push(e.to_string());
                    }
                }
                Follow::AutoApply => self.sync_auto_apply(&id, &mut notices),
                // Best-effort: shows what is left after a run.
                Follow::Refresh => drop(self.sync_compare(&id, false)),
            }
        }
        for n in notices {
            self.notice = Some(n);
        }
        if let Some(n) = infos.pop() {
            self.info = Some((n, Instant::now()));
        }
        changed
    }

    /// Apply the plan the user just confirmed, under the same safeguards as later runs.
    pub(crate) fn sync_apply_confirmed(&mut self, id: &str) {
        let mut notices = Vec::new();
        self.sync_auto_apply(id, &mut notices);
        if let Some(n) = notices.pop() {
            self.notice = Some(n);
        }
    }

    fn sync_auto_apply(&mut self, id: &str, notices: &mut Vec<String>) {
        let Some(state) = self.sync.states.get_mut(id) else {
            return;
        };
        let Some(plan) = &state.plan else { return };
        let work = !plan.is_empty_work();
        if !work {
            return;
        }
        if plan.summary.conflicts > 0 || plan.guards.confirmation_required() {
            state.watch_note = Some("Changes need your review before they can be applied".into());
            notices.push(format!(
                "Sync \"{}\": changes need your review",
                state.profile.name
            ));
            return;
        }
        if let Err(e) = self.sync_run(id, false) {
            notices.push(e.to_string());
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl std::fmt::Debug for SyncHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncHub")
            .field("profiles", &self.profiles.len())
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}
