//! File-operation jobs: the bridge between `fm-ops` pipelines and the job lanes.
//!
//! A request is submitted on the lane of its destination (source for delete/rename/mkdir).
//! The worker publishes progress and parked conflicts into a per-job shared slot; the
//! UI-side [`Session::poll`] copies them into the job table without ever blocking, and on
//! completion records the undo entry and invalidates every touched directory.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};

use fm_ops::{
    Backends, CancelFlag, Choice, Conflict, ConflictPolicy, Decision, OpCtx, OpKind, OpReport,
    Progress, UndoRecord,
};
use fm_vfs::{FmError, Vfs, VfsUri};

use crate::backends::{VfsRegistry, bulk_lane_for};
use crate::dircache::DirState;
use crate::jobs::{JobCtx, JobEvent, JobId};
use crate::session::Session;

/// Cap on the sources of one job (hostile or accidental selections).
pub const MAX_SOURCES: usize = 10_000;
/// Finished jobs kept in the table before the oldest are dropped.
const MAX_FINISHED: usize = 50;
/// Failed items kept per summary.
const MAX_FAILURES: usize = 100;
/// Progress events are throttled so a fast copy cannot flood the channel.
const PROGRESS_EVERY: Duration = Duration::from_millis(30);
/// How often a parked worker re-checks its cancel flag.
const PARK_TICK: Duration = Duration::from_millis(50);

impl Backends for VfsRegistry {
    fn vfs(&self, uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        self.require(uri)
    }
}

/// A mutating operation to run as a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpRequest {
    Copy {
        sources: Vec<VfsUri>,
        dest: VfsUri,
        policy: ConflictPolicy,
    },
    Move {
        sources: Vec<VfsUri>,
        dest: VfsUri,
        policy: ConflictPolicy,
    },
    /// Copy that numbers items already inside `dest` (`a (2).txt`) instead of skipping them.
    Duplicate {
        sources: Vec<VfsUri>,
        dest: VfsUri,
        policy: ConflictPolicy,
    },
    Trash {
        sources: Vec<VfsUri>,
    },
    DeletePermanent {
        sources: Vec<VfsUri>,
    },
    Mkdir {
        parent: VfsUri,
        name: String,
    },
    NewFile {
        parent: VfsUri,
        name: String,
    },
    Rename {
        uri: VfsUri,
        name: String,
    },
    Undo(UndoRecord),
    /// Set the permission bits of each item (not recursive, not undoable).
    Chmod {
        sources: Vec<VfsUri>,
        mode: u32,
    },
    /// Fetch a remote file to a private temp folder and hand it to the default application.
    OpenRemote {
        uri: VfsUri,
        name: String,
        size: Option<u64>,
    },
}

fn items(n: usize) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{n} items")
    }
}

fn leaf(uri: &VfsUri) -> String {
    uri.file_name().unwrap_or("/").to_string()
}

impl OpRequest {
    /// The kind recorded on the undo stack; `None` for undo itself.
    pub fn kind(&self) -> Option<OpKind> {
        Some(match self {
            Self::Copy { .. } | Self::Duplicate { .. } => OpKind::Copy,
            Self::Move { .. } => OpKind::Move,
            Self::Trash { .. } | Self::DeletePermanent { .. } => OpKind::Delete,
            Self::Mkdir { .. } => OpKind::Mkdir,
            Self::NewFile { .. } => OpKind::NewFile,
            Self::Rename { .. } => OpKind::Rename,
            Self::Undo(_) | Self::Chmod { .. } => return None,
            Self::OpenRemote { .. } => return None,
        })
    }

    pub fn label(&self) -> String {
        match self {
            Self::Copy { sources, dest, .. } | Self::Duplicate { sources, dest, .. } => {
                format!("Copy {} to {}", items(sources.len()), leaf(dest))
            }
            Self::Move { sources, dest, .. } => {
                format!("Move {} to {}", items(sources.len()), leaf(dest))
            }
            Self::Trash { sources } => format!("Trash {}", items(sources.len())),
            Self::DeletePermanent { sources } => format!("Delete {}", items(sources.len())),
            Self::Mkdir { name, .. } => format!("Create folder {name}"),
            Self::NewFile { name, .. } => format!("Create file {name}"),
            Self::Rename { uri, name } => format!("Rename {} to {name}", leaf(uri)),
            Self::Undo(_) => "Undo".to_string(),
            Self::Chmod { sources, mode } => {
                format!("Set permissions {mode:o} on {}", items(sources.len()))
            }
            Self::OpenRemote { name, .. } => format!("Open {name}"),
        }
    }

    fn lane_uri(&self) -> Option<VfsUri> {
        match self {
            Self::Copy { dest, .. } | Self::Move { dest, .. } | Self::Duplicate { dest, .. } => {
                Some(dest.clone())
            }
            Self::Trash { sources }
            | Self::DeletePermanent { sources }
            | Self::Chmod { sources, .. } => sources.first().cloned(),
            Self::Mkdir { parent, .. } | Self::NewFile { parent, .. } => Some(parent.clone()),
            Self::Rename { uri, .. } => Some(uri.clone()),
            Self::Undo(record) => undo_uris(record).into_iter().next(),
            Self::OpenRemote { uri, .. } => Some(uri.clone()),
        }
    }

    fn uris(&self) -> Vec<VfsUri> {
        match self {
            Self::Copy { sources, dest, .. }
            | Self::Move { sources, dest, .. }
            | Self::Duplicate { sources, dest, .. } => {
                let mut all = sources.clone();
                all.push(dest.clone());
                all
            }
            Self::Trash { sources }
            | Self::DeletePermanent { sources }
            | Self::Chmod { sources, .. } => sources.clone(),
            Self::Mkdir { parent, .. } | Self::NewFile { parent, .. } => vec![parent.clone()],
            Self::Rename { uri, .. } => vec![uri.clone()],
            Self::Undo(record) => undo_uris(record),
            Self::OpenRemote { uri, .. } => vec![uri.clone()],
        }
    }

    /// Directories whose listing changes when this job completes.
    pub fn touched_dirs(&self) -> Vec<VfsUri> {
        let mut dirs: Vec<VfsUri> = Vec::new();
        let mut add = |uri: Option<VfsUri>| {
            if let Some(uri) = uri
                && !dirs.contains(&uri)
            {
                dirs.push(uri);
            }
        };
        match self {
            Self::Copy { dest, .. } | Self::Duplicate { dest, .. } => add(Some(dest.clone())),
            Self::Move { sources, dest, .. } => {
                add(Some(dest.clone()));
                sources.iter().for_each(|s| add(s.parent()));
            }
            Self::Trash { sources }
            | Self::DeletePermanent { sources }
            | Self::Chmod { sources, .. } => {
                sources.iter().for_each(|s| add(s.parent()));
            }
            Self::Mkdir { parent, .. } | Self::NewFile { parent, .. } => add(Some(parent.clone())),
            Self::Rename { uri, .. } => add(uri.parent()),
            Self::OpenRemote { .. } => {}
            Self::Undo(record) => undo_uris(record).iter().for_each(|u| add(u.parent())),
        }
        dirs
    }
}

fn undo_uris(record: &UndoRecord) -> Vec<VfsUri> {
    match record {
        UndoRecord::Trashed { originals } => originals.clone(),
        UndoRecord::Moved { pairs } => pairs
            .iter()
            .flat_map(|(a, b)| [a.clone(), b.clone()])
            .collect(),
        UndoRecord::Renamed { from, to } => vec![from.clone(), to.clone()],
        UndoRecord::Created { uris } => uris.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpState {
    Running,
    Finished,
    Cancelled,
    Failed(String),
}

impl OpState {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// What a finished job did, with per-item errors for the jobs panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpSummary {
    pub done: u64,
    pub skipped: u64,
    pub failed_count: usize,
    /// `(item, error)`, capped at [`MAX_FAILURES`].
    pub failed: Vec<(String, String)>,
}

impl From<&OpReport> for OpSummary {
    fn from(report: &OpReport) -> Self {
        Self {
            done: report.done,
            skipped: report.skipped,
            failed_count: report.failed.len(),
            failed: report
                .failed
                .iter()
                .take(MAX_FAILURES)
                .map(|(uri, err)| (uri.to_string(), err.to_string()))
                .collect(),
        }
    }
}

#[derive(Debug, Default)]
struct Shared {
    progress: Progress,
    conflict: Option<Conflict>,
    report: Option<OpReport>,
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|p| p.into_inner())
}

/// Smoothed transfer rate of a running job, sampled from its byte progress.
#[derive(Debug, Clone, Default)]
pub struct SpeedMeter {
    last: Option<(Instant, u64)>,
    bytes_per_sec: Option<f64>,
}

impl SpeedMeter {
    const MIN_WINDOW: Duration = Duration::from_millis(500);
    const SMOOTHING: f64 = 0.3;

    /// Feed the bytes done so far at `now`; cheap enough to call every frame.
    pub fn sample(&mut self, now: Instant, done: u64) {
        let Some((at, was)) = self.last else {
            self.last = Some((now, done));
            return;
        };
        let dt = now.saturating_duration_since(at);
        if dt < Self::MIN_WINDOW {
            return;
        }
        let rate = done.saturating_sub(was) as f64 / dt.as_secs_f64();
        self.bytes_per_sec = Some(match self.bytes_per_sec {
            Some(old) => old + Self::SMOOTHING * (rate - old),
            None => rate,
        });
        self.last = Some((now, done));
    }

    /// Bytes per second, once a first window has elapsed.
    pub fn bytes_per_sec(&self) -> Option<f64> {
        self.bytes_per_sec
    }

    /// Seconds left for `remaining` bytes at the current rate.
    pub fn eta_secs(&self, remaining: u64) -> Option<u64> {
        let rate = self.bytes_per_sec.filter(|r| *r >= 1.0)?;
        Some((remaining as f64 / rate).ceil() as u64)
    }
}
/// One row of the jobs table.
#[derive(Debug)]
pub struct OpJob {
    pub id: JobId,
    pub label: String,
    pub state: OpState,
    pub progress: Progress,
    pub speed: SpeedMeter,
    /// Set while the worker is parked waiting for `job.resolve_conflict`.
    pub conflict: Option<Conflict>,
    pub summary: Option<OpSummary>,
    kind: Option<OpKind>,
    touched: Vec<VfsUri>,
    /// When the touched directories were last re-read while the job runs.
    last_refresh: Option<Instant>,
    /// Sources and destinations, including read-only sources, for safe device removal.
    uris: Vec<VfsUri>,
    shared: Arc<Mutex<Shared>>,
    resolver: Sender<Decision>,
}

/// The jobs table (insertion order).
#[derive(Debug, Default)]
pub struct FileOps {
    jobs: Vec<OpJob>,
}

impl FileOps {
    pub(crate) fn uses_volume(&self, mount: &VfsUri) -> bool {
        self.jobs.iter().any(|job| {
            job.state.is_running()
                && job.uris.iter().any(|uri| {
                    mount.is_same_or_descendant_of(uri) || uri.is_same_or_descendant_of(mount)
                })
        })
    }
    pub fn jobs(&self) -> &[OpJob] {
        &self.jobs
    }

    pub fn get(&self, id: JobId) -> Option<&OpJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    fn get_mut(&mut self, id: JobId) -> Option<&mut OpJob> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn running(&self) -> usize {
        self.jobs.iter().filter(|j| j.state.is_running()).count()
    }

    /// The first job waiting on a conflict decision, if any.
    pub fn pending_conflict(&self) -> Option<&OpJob> {
        self.jobs
            .iter()
            .find(|j| j.state.is_running() && j.conflict.is_some())
    }

    /// Remove a finished job (dismissing its report). Running jobs stay.
    pub fn dismiss(&mut self, id: JobId) -> bool {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.id != id || j.state.is_running());
        self.jobs.len() != before
    }

    fn trim(&mut self) {
        let mut finished = self.jobs.iter().filter(|j| !j.state.is_running()).count();
        while finished > MAX_FINISHED {
            let Some(pos) = self.jobs.iter().position(|j| !j.state.is_running()) else {
                break;
            };
            self.jobs.remove(pos);
            finished -= 1;
        }
    }
}

fn run(
    request: &OpRequest,
    backends: &VfsRegistry,
    ctx: &JobCtx,
    shared: &Mutex<Shared>,
    decisions: &Receiver<Decision>,
    retarget_links: bool,
) -> Result<(), FmError> {
    let cancel = CancelFlag::from_shared(ctx.cancel_shared());
    if let OpRequest::OpenRemote { uri, name, size } = request {
        let vfs = backends.require(uri)?;
        let mut last_sent: Option<Instant> = None;
        let path = crate::open_cmds::download_to_temp(
            vfs.as_ref(),
            uri,
            name,
            *size,
            &|| ctx.is_cancelled(),
            &mut |done, total| {
                {
                    let mut guard = lock(shared);
                    guard.progress.bytes = fm_ops::TransferProgress { done, total };
                    guard.progress.current = name.clone();
                }
                if last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    last_sent = Some(Instant::now());
                    ctx.report(done, total);
                }
            },
        )?;
        return crate::open_cmds::launch_default(&path);
    }
    let mut last_sent: Option<Instant> = None;
    let mut on_progress = |p: &Progress| {
        lock(shared).progress = p.clone();
        let finished = p.items.total > 0 && p.items.done >= p.items.total;
        let due = last_sent.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
        if finished || due {
            last_sent = Some(Instant::now());
            ctx.report(p.items.done, p.items.total);
        }
    };
    let park_cancel = cancel.clone();
    let mut on_conflict = |conflict: &Conflict| {
        lock(shared).conflict = Some(conflict.clone());
        ctx.notify_conflict();
        let decision = loop {
            if park_cancel.is_cancelled() {
                break Decision {
                    choice: Choice::Cancel,
                    apply_to_all: false,
                };
            }
            match decisions.recv_timeout(PARK_TICK) {
                Ok(decision) => break decision,
                Err(RecvTimeoutError::Timeout) => {}
                // The job table dropped us: nobody can answer, so cancel.
                Err(RecvTimeoutError::Disconnected) => {
                    break Decision {
                        choice: Choice::Cancel,
                        apply_to_all: false,
                    };
                }
            }
        };
        lock(shared).conflict = None;
        decision
    };
    let policy = match request {
        OpRequest::Copy { policy, .. }
        | OpRequest::Move { policy, .. }
        | OpRequest::Duplicate { policy, .. } => *policy,
        _ => ConflictPolicy::Ask,
    };
    let mut octx = OpCtx::new(policy, &mut on_progress, &mut on_conflict)
        .with_cancel(cancel)
        .with_retarget_links(retarget_links);

    let report = match request {
        OpRequest::Copy { sources, dest, .. } => fm_ops::copy(backends, sources, dest, &mut octx)?,
        OpRequest::Duplicate { sources, dest, .. } => {
            fm_ops::duplicate(backends, sources, dest, &mut octx)?
        }
        OpRequest::Move { sources, dest, .. } => fm_ops::move_(backends, sources, dest, &mut octx)?,
        OpRequest::Trash { sources } => fm_ops::trash_items(sources, &mut octx),
        OpRequest::DeletePermanent { sources } => {
            fm_ops::delete_permanent(backends, sources, &mut octx)?
        }
        OpRequest::Mkdir { parent, name } => {
            let created = fm_ops::mkdir(backends, parent, name)?;
            OpReport {
                done: 1,
                created: vec![created],
                ..OpReport::default()
            }
        }
        OpRequest::NewFile { parent, name } => {
            let created = fm_ops::create_file(backends, parent, name)?;
            OpReport {
                done: 1,
                created: vec![created],
                ..OpReport::default()
            }
        }
        OpRequest::Rename { uri, name } => {
            let to = fm_ops::rename(backends, uri, name)?;
            OpReport {
                done: 1,
                moved: vec![(uri.clone(), to)],
                ..OpReport::default()
            }
        }
        OpRequest::Undo(record) => {
            fm_ops::undo::apply(backends, record)?;
            OpReport {
                done: 1,
                ..OpReport::default()
            }
        }
        OpRequest::Chmod { sources, mode } => {
            fm_ops::set_permissions(backends, sources, *mode, &octx)
        }
        OpRequest::OpenRemote { .. } => return Ok(()),
    };
    let cancelled = report.cancelled;
    lock(shared).report = Some(report);
    if cancelled {
        Err(FmError::Cancelled)
    } else {
        Ok(())
    }
}

impl Session {
    /// Validate and enqueue a mutating job. Returns immediately; the work runs on the
    /// lane of the destination (or source) and reports through [`Session::poll`].
    pub fn submit_op(&mut self, request: OpRequest) -> Result<JobId, FmError> {
        let uris = request.uris();
        if uris.iter().any(|uri| self.volumes.blocks(uri)) {
            return Err(FmError::backend(
                "devices",
                "This volume is disconnected or being removed. Reconnect it or wait for removal to finish.",
            ));
        }
        if uris.len() > MAX_SOURCES + 1 {
            return Err(FmError::InvalidParams(format!(
                "too many items (limit {MAX_SOURCES})"
            )));
        }
        let lane_uri = request
            .lane_uri()
            .ok_or_else(|| FmError::InvalidParams("nothing to do".to_string()))?;
        let lane = bulk_lane_for(lane_uri.scheme());
        for uri in &uris {
            self.backends.require(uri)?;
        }
        let backends = self.backends.clone();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (resolver, decisions) = unbounded::<Decision>();
        let label = request.label();
        let kind = request.kind();
        let touched = request.touched_dirs();
        let worker_shared = shared.clone();
        let retarget_links = self.prefs.retarget_links;
        let id = self.jobs.submit(lane, move |ctx| {
            run(
                &request,
                &backends,
                ctx,
                &worker_shared,
                &decisions,
                retarget_links,
            )
        });
        self.fileops.jobs.push(OpJob {
            id,
            label,
            state: OpState::Running,
            progress: Progress::default(),
            speed: SpeedMeter::default(),
            conflict: None,
            summary: None,
            kind,
            touched,
            last_refresh: None,
            uris,
            shared,
            resolver,
        });
        Ok(id)
    }

    /// Request cooperative cancellation of a file job (also frees a parked worker).
    pub fn cancel_op(&mut self, id: JobId) -> Result<(), FmError> {
        if self.fileops.get(id).is_none() {
            return Err(FmError::NotFound(format!("job {id}")));
        }
        self.jobs.cancel(id);
        Ok(())
    }

    /// Answer the conflict a parked job is waiting on.
    pub fn resolve_conflict(&mut self, id: JobId, decision: Decision) -> Result<(), FmError> {
        let job = self
            .fileops
            .get_mut(id)
            .ok_or_else(|| FmError::NotFound(format!("job {id}")))?;
        if !job.state.is_running() || job.conflict.is_none() {
            return Err(FmError::InvalidParams(format!(
                "job {id} is not waiting for a decision"
            )));
        }
        job.resolver
            .send(decision)
            .map_err(|_| FmError::Cancelled)?;
        job.conflict = None;
        Ok(())
    }

    /// Copy the latest worker-side progress and conflicts into the job table.
    /// Returns `true` when anything observable changed.
    pub(crate) fn sync_ops(&mut self) -> bool {
        let mut changed = false;
        for job in self
            .fileops
            .jobs
            .iter_mut()
            .filter(|j| j.state.is_running())
        {
            let guard = lock(&job.shared);
            job.speed.sample(Instant::now(), guard.progress.bytes.done);
            if guard.progress != job.progress {
                job.progress = guard.progress.clone();
                changed = true;
            }
            if guard.conflict != job.conflict {
                job.conflict = guard.conflict.clone();
                changed = true;
            }
        }
        changed |= self.refresh_while_running();
        changed
    }

    /// Re-read the directories a running job writes into (and any pane inside them) about
    /// once a second, so new items show up while the job runs rather than only at its end.
    fn refresh_while_running(&mut self) -> bool {
        const EVERY: Duration = Duration::from_secs(1);
        let now = Instant::now();
        let mut roots: Vec<VfsUri> = Vec::new();
        for job in self
            .fileops
            .jobs
            .iter_mut()
            .filter(|j| j.state.is_running())
        {
            if job
                .last_refresh
                .is_some_and(|at| now.duration_since(at) < EVERY)
            {
                continue;
            }
            job.last_refresh = Some(now);
            roots.extend(job.touched.iter().cloned());
        }
        if roots.is_empty() {
            return false;
        }
        let mut dirs = roots.clone();
        dirs.extend(
            self.layout
                .states
                .values()
                .map(|state| state.location.clone())
                .filter(|loc| roots.iter().any(|root| loc.is_same_or_descendant_of(root))),
        );
        dirs.dedup();
        for dir in dirs {
            if matches!(self.dirs.state(&dir), DirState::Ready { .. }) {
                self.poll_location(dir);
            }
        }
        false
    }

    /// Handle a terminal event of a file job: record the summary and the undo entry,
    /// then invalidate the touched directories so their panes re-read once.
    /// Returns `false` when `ev` does not belong to a file job.
    pub(crate) fn finish_op(&mut self, ev: &JobEvent) -> bool {
        let id = ev.job_id();
        let Some(job) = self.fileops.get_mut(id) else {
            return false;
        };
        let state = match ev {
            JobEvent::Done(_) => OpState::Finished,
            JobEvent::Cancelled(_) => OpState::Cancelled,
            JobEvent::Failed { message, .. } => OpState::Failed(message.clone()),
            _ => return true,
        };
        let (report, progress) = {
            let mut guard = lock(&job.shared);
            (guard.report.take(), guard.progress.clone())
        };
        job.progress = progress;
        job.state = state;
        job.conflict = None;
        job.summary = report.as_ref().map(OpSummary::from);
        let kind = job.kind;
        let touched = std::mem::take(&mut job.touched);
        if let (Some(kind), Some(report)) = (kind, report.as_ref()) {
            self.undo.push_report(kind, report);
        }
        for dir in touched {
            self.dir_changed(&dir);
            if !matches!(self.dirs.state(&dir), DirState::Absent) {
                let _ = self.refresh_read(dir);
            }
        }
        self.fileops.trim();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_meter_smooths_and_estimates_eta() {
        let t0 = Instant::now();
        let mut m = SpeedMeter::default();
        m.sample(t0, 0);
        assert_eq!(m.bytes_per_sec(), None);
        m.sample(t0 + Duration::from_millis(100), 999);
        assert_eq!(m.bytes_per_sec(), None, "window too short");
        m.sample(t0 + Duration::from_secs(1), 1_000_000);
        assert_eq!(m.bytes_per_sec().map(|r| r.round()), Some(1_000_000.0));
        assert_eq!(m.eta_secs(3_000_000), Some(3));
        // A stall pulls the rate down without jumping straight to zero.
        m.sample(t0 + Duration::from_secs(2), 1_000_000);
        let slowed = m.bytes_per_sec().unwrap_or_default();
        assert!(slowed > 0.0 && slowed < 1_000_000.0);
        // Progress going backwards (a new item) never underflows.
        m.sample(t0 + Duration::from_secs(3), 10);
    }
}
