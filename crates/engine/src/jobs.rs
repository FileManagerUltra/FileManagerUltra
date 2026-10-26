//! Job queue — the engine's background lanes. The UI thread never blocks: commands
//! enqueue work here and return; progress and completion arrive as [`JobEvent`]s.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

use fm_vfs::FmError;

/// Stable id of a submitted job.
pub type JobId = u64;

/// Which lane a job runs on. One dedicated thread per lane; SMB, SFTP and local
/// work never share a lane, so a stalled share cannot starve local disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobLane {
    Local,
    Smb,
    Sftp,
}

/// Events flowing back to the UI-facing poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEvent {
    Started(JobId),
    Progress {
        id: JobId,
        done: u64,
        total: u64,
    },
    /// A parked file job needs a decision; the conflict itself sits in the job table.
    Conflict(JobId),
    Done(JobId),
    Failed {
        id: JobId,
        message: String,
    },
    Cancelled(JobId),
}

impl JobEvent {
    /// The job this event belongs to.
    pub fn job_id(&self) -> JobId {
        match self {
            Self::Started(id) | Self::Conflict(id) | Self::Done(id) | Self::Cancelled(id) => *id,
            Self::Progress { id, .. } | Self::Failed { id, .. } => *id,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done(_) | Self::Failed { .. } | Self::Cancelled(_)
        )
    }
}

/// Context handed to a running job: cancellation flag + progress reporting.
#[derive(Clone)]
pub struct JobCtx {
    id: JobId,
    cancel: Arc<AtomicBool>,
    events: Sender<JobEvent>,
}

impl JobCtx {
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Long-running work must poll this between units of work.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Shared cancel flag, for pipelines that take their own handle.
    pub fn cancel_shared(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Wake the UI poll: this job is parked waiting for a decision.
    pub fn notify_conflict(&self) {
        let _ = self.events.send(JobEvent::Conflict(self.id));
    }

    /// Report progress; the UI poll picks these up.
    pub fn report(&self, done: u64, total: u64) {
        let _ = self.events.send(JobEvent::Progress {
            id: self.id,
            done,
            total,
        });
    }
}

type JobFn = Box<dyn FnOnce(&JobCtx) -> Result<(), FmError> + Send + 'static>;

struct Job {
    id: JobId,
    cancel: Arc<AtomicBool>,
    work: JobFn,
}

type CancelMap = Arc<Mutex<HashMap<JobId, Arc<AtomicBool>>>>;

/// The queue: three lane threads, one events channel, cancellation by id.
pub struct JobQueue {
    lanes: Vec<(Sender<Job>, JoinHandle<()>)>,
    events_rx: Receiver<JobEvent>,
    events_tx: Sender<JobEvent>,
    next_id: AtomicU64,
    cancel_flags: CancelMap,
}

impl std::fmt::Debug for JobQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobQueue")
            .field("lanes", &self.lanes.len())
            .finish_non_exhaustive()
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue {
    pub fn new() -> Self {
        let (events_tx, events_rx) = unbounded();
        let cancel_flags: CancelMap = Arc::new(Mutex::new(HashMap::new()));
        let lanes = [JobLane::Local, JobLane::Smb, JobLane::Sftp]
            .into_iter()
            .map(|lane| Self::spawn_lane(lane, events_tx.clone(), cancel_flags.clone()))
            .collect();
        Self {
            lanes,
            events_rx,
            events_tx,
            next_id: AtomicU64::new(1),
            cancel_flags,
        }
    }

    fn spawn_lane(
        lane: JobLane,
        events: Sender<JobEvent>,
        cancel_flags: CancelMap,
    ) -> (Sender<Job>, JoinHandle<()>) {
        let (tx, rx) = unbounded::<Job>();
        let name = format!(
            "fm-job-{}",
            match lane {
                JobLane::Local => "local",
                JobLane::Smb => "smb",
                JobLane::Sftp => "sftp",
            }
        );
        let events_for_thread = events.clone();
        let built = std::thread::Builder::new().name(name).spawn(move || {
            let events = events_for_thread;
            while let Ok(job) = rx.recv() {
                let ctx = JobCtx {
                    id: job.id,
                    cancel: job.cancel.clone(),
                    events: events.clone(),
                };
                let _ = events.send(JobEvent::Started(job.id));
                let outcome = (job.work)(&ctx);
                let terminal = match outcome {
                    Ok(()) => JobEvent::Done(job.id),
                    Err(FmError::Cancelled) => JobEvent::Cancelled(job.id),
                    Err(err) => JobEvent::Failed {
                        id: job.id,
                        message: err.to_string(),
                    },
                };
                cancel_flags
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&job.id);
                let _ = events.send(terminal);
            }
        });
        match built {
            Ok(handle) => (tx, handle),
            Err(_) => {
                // The OS refused a thread: degrade to a lane that fails every job
                // with an event instead of panicking.
                let (dead_tx, dead_rx) = unbounded::<Job>();
                let fallback_events = events.clone();
                let handle = std::thread::spawn(move || {
                    while let Ok(job) = dead_rx.recv() {
                        let _ = fallback_events.send(JobEvent::Started(job.id));
                        let _ = fallback_events.send(JobEvent::Failed {
                            id: job.id,
                            message: "job lane unavailable".to_string(),
                        });
                    }
                });
                (dead_tx, handle)
            }
        }
    }

    /// Enqueue work on `lane`; returns immediately with the job id.
    pub fn submit<F>(&self, lane: JobLane, work: F) -> JobId
    where
        F: FnOnce(&JobCtx) -> Result<(), FmError> + Send + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, cancel.clone());
        let job = Job {
            id,
            cancel,
            work: Box::new(work),
        };
        let idx = match lane {
            JobLane::Local => 0,
            JobLane::Smb => 2,
            JobLane::Sftp => 2,
        };
        if self.lanes[idx].0.send(job).is_err() {
            let _ = self.events_tx.send(JobEvent::Failed {
                id,
                message: "job lane unavailable".to_string(),
            });
        }
        id
    }

    /// Request cancellation; the job observes it via `JobCtx::is_cancelled` and the
    /// lane emits `Cancelled` when the work function returns `FmError::Cancelled`.
    pub fn cancel(&self, id: JobId) {
        let flags = self.cancel_flags.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(flag) = flags.get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Drain pending events (non-blocking).
    pub fn poll(&self) -> Vec<JobEvent> {
        self.events_rx.try_iter().collect()
    }

    /// Block until `id` reaches a terminal event (test helper; the UI uses `poll`).
    pub fn wait_for_terminal(&self, id: JobId, timeout: Duration) -> Option<JobEvent> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            for event in self.poll() {
                if event.job_id() == id && event.is_terminal() {
                    return Some(event);
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn job_runs_and_reports_done() {
        let q = JobQueue::new();
        let id = q.submit(JobLane::Local, |ctx| {
            ctx.report(1, 2);
            ctx.report(2, 2);
            Ok(())
        });
        match q.wait_for_terminal(id, Duration::from_secs(5)) {
            Some(JobEvent::Done(got)) => assert_eq!(got, id),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn job_returns_before_work_finishes() {
        let q = JobQueue::new();
        let (release_tx, release_rx) = channel();
        let id = q.submit(JobLane::Local, move |_ctx| {
            // Block until the test releases us — proves submit returned immediately.
            release_rx.recv().map_err(|_| FmError::Cancelled)?;
            Ok(())
        });
        // The job cannot have finished: it is waiting for the release message.
        assert!(
            q.poll()
                .iter()
                .all(|e| e.job_id() != id || !matches!(e, JobEvent::Done(_)))
        );
        release_tx.send(()).expect("release");
        match q.wait_for_terminal(id, Duration::from_secs(5)) {
            Some(JobEvent::Done(_)) => {}
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn failed_job_reports_message() {
        let q = JobQueue::new();
        let id = q.submit(JobLane::Smb, |_ctx| Err(FmError::Unsupported("test")));
        match q.wait_for_terminal(id, Duration::from_secs(5)) {
            Some(JobEvent::Failed { message, .. }) => {
                assert!(message.contains("test"), "{message}")
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn cancel_stops_a_running_job() {
        let q = JobQueue::new();
        let id = q.submit(JobLane::Local, |ctx| {
            while !ctx.is_cancelled() {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(FmError::Cancelled)
        });
        // Wait for Start so the job is definitely running before we cancel.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if q.poll()
                .iter()
                .any(|e| matches!(e, JobEvent::Started(i) if *i == id))
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "job never started");
            std::thread::sleep(Duration::from_millis(2));
        }
        q.cancel(id);
        match q.wait_for_terminal(id, Duration::from_secs(5)) {
            Some(JobEvent::Cancelled(got)) => assert_eq!(got, id),
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn lanes_are_independent() {
        let q = JobQueue::new();
        let (block_tx, block_rx) = channel::<()>();
        // Fill the local lane with a blocking job…
        let blocker = q.submit(JobLane::Local, move |_ctx| {
            block_rx.recv().map_err(|_| FmError::Cancelled)?;
            Ok(())
        });
        // …and check the smb lane still completes.
        let smb = q.submit(JobLane::Smb, |_ctx| Ok(()));
        match q.wait_for_terminal(smb, Duration::from_secs(5)) {
            Some(JobEvent::Done(_)) => {}
            other => panic!("smb lane stuck: {other:?}"),
        }
        block_tx.send(()).expect("release");
        q.wait_for_terminal(blocker, Duration::from_secs(5));
    }
}
