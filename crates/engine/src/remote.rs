//! Remote (SFTP) connections: the per-authority state machine, the trust-on-first-use
//! host-key store, connect/reconnect jobs, the background refresh of visible remote
//! panes and the cached free-space lookup (capability `sftp-backend`).
//!
//! Credentials live only in memory, in the connection table. They are never written to
//! disk and never formatted into an error message.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fm_fs_sftp::{ConnectParams, HostKeyInfo, HostKeyVerdict, HostKeyVerifier, SftpFs};
use fm_vfs::{Authority, Entry, FmError, Scheme, SpaceInfo, VfsUri};
use serde::{Deserialize, Serialize};

use crate::jobs::{JobCtx, JobEvent, JobId, JobLane};
use crate::session::Session;

/// File name inside the config directory.
pub const KNOWN_HOSTS_FILE: &str = "known_hosts.json";
/// Most trusted hosts kept; more are refused rather than evicting a trusted key.
pub const MAX_KNOWN_HOSTS: usize = 1000;
/// Longest accepted field of a known-host record.
pub const MAX_KNOWN_FIELD: usize = 4096;
const MAX_KNOWN_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// How long an unanswered host-key prompt keeps a connect job waiting.
const HOSTKEY_WAIT: Duration = Duration::from_secs(120);
/// Interval of the background refresh of visible remote panes.
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How long a free-space answer stays fresh.
pub const SPACE_TTL: Duration = Duration::from_secs(30);
const MAX_SPACE_SLOTS: usize = 64;

/// One trusted host key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownHost {
    /// `host:port`, host lower-cased.
    pub host: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub openssh: String,
}

/// Result of comparing a presented key with the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCheck {
    Match,
    /// A different key is on file; carries the trusted fingerprint.
    Changed {
        known: String,
    },
    Unknown,
}

/// The trusted host keys (`known_hosts.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownHosts {
    entries: Vec<KnownHost>,
}

fn host_id(info: &HostKeyInfo) -> String {
    format!("{}:{}", info.host.to_ascii_lowercase(), info.port)
}

impl KnownHosts {
    /// Load `dir/known_hosts.json`. Missing, oversized or corrupt files yield an empty
    /// store; individual corrupt or oversized records are skipped.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(KNOWN_HOSTS_FILE);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() <= MAX_KNOWN_FILE_BYTES => {}
            _ => return Self::default(),
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        let entries = doc
            .get("hosts")
            .and_then(|v| v.as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| serde_json::from_value::<KnownHost>(row.clone()).ok())
                    .filter(|h| {
                        [&h.host, &h.algorithm, &h.fingerprint, &h.openssh]
                            .iter()
                            .all(|f| f.len() <= MAX_KNOWN_FIELD)
                    })
                    .take(MAX_KNOWN_HOSTS)
                    .collect()
            })
            .unwrap_or_default();
        Self { entries }
    }

    /// Persist owner-only through a temp file and atomic rename.
    pub fn save(&self, dir: &Path) -> Result<(), FmError> {
        let doc = serde_json::json!({ "hosts": self.entries });
        let text = serde_json::to_string_pretty(&doc)
            .map_err(|err| FmError::Io(std::io::Error::other(err)))?;
        crate::prefs::write_atomic(dir, KNOWN_HOSTS_FILE, text.as_bytes())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The trusted hosts, in the order they were added.
    pub fn entries(&self) -> &[KnownHost] {
        &self.entries
    }

    /// Forget the host `id` (`host:port`). Returns whether it was on file.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|h| !h.host.eq_ignore_ascii_case(id));
        self.entries.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn check(&self, info: &HostKeyInfo) -> HostCheck {
        let id = host_id(info);
        match self.entries.iter().find(|h| h.host == id) {
            None => HostCheck::Unknown,
            Some(h) if h.fingerprint == info.fingerprint => HostCheck::Match,
            Some(h) => HostCheck::Changed {
                known: h.fingerprint.clone(),
            },
        }
    }

    /// Trust `info`. A host already on file is never replaced (v1 has no replace path).
    pub fn add(&mut self, info: &HostKeyInfo) -> Result<(), FmError> {
        let id = host_id(info);
        if self.entries.iter().any(|h| h.host == id) {
            return Err(FmError::InvalidParams(
                "a key for this host is already on file".to_string(),
            ));
        }
        if self.entries.len() >= MAX_KNOWN_HOSTS {
            return Err(FmError::InvalidParams(
                "too many known hosts; remove some from known_hosts.json".to_string(),
            ));
        }
        let row = KnownHost {
            host: id,
            algorithm: info.algorithm.clone(),
            fingerprint: info.fingerprint.clone(),
            openssh: info.openssh.clone(),
        };
        if [&row.host, &row.algorithm, &row.fingerprint, &row.openssh]
            .iter()
            .any(|f| f.len() > MAX_KNOWN_FIELD)
        {
            return Err(FmError::InvalidParams(
                "host key is too large to store".to_string(),
            ));
        }
        self.entries.push(row);
        Ok(())
    }
}

/// An unknown host key waiting for the user's answer.
#[derive(Debug)]
pub struct PendingHostKey {
    pub info: HostKeyInfo,
    reply: mpsc::Sender<bool>,
}

type PendingSlot = Arc<Mutex<Option<PendingHostKey>>>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Marker text of the refusal for a changed host key (kept in one place so the UI card
/// and the verifier cannot drift apart).
const CHANGED_KEY: &str = "host key of";

/// Whether a connect failure is the refusal of a changed host key.
pub fn is_changed_key_message(message: &str) -> bool {
    message.contains(CHANGED_KEY) && message.contains("changed")
}

/// Trust-on-first-use policy: known keys pass, changed keys are refused, unknown keys
/// ask the user (`remote.hostkey.answer`).
struct Tofu {
    known: Arc<Mutex<KnownHosts>>,
    pending: PendingSlot,
    dir: Option<PathBuf>,
    ctx: JobCtx,
}

impl HostKeyVerifier for Tofu {
    fn verify(&self, info: &HostKeyInfo) -> HostKeyVerdict {
        match lock(&self.known).check(info) {
            HostCheck::Match => return HostKeyVerdict::Trust,
            HostCheck::Changed { known } => {
                return HostKeyVerdict::Reject(format!(
                    "the {CHANGED_KEY} {}:{} changed (trusted {known}, presented {}); \
                     refusing to connect — this may be an attack",
                    info.host, info.port, info.fingerprint
                ));
            }
            HostCheck::Unknown => {}
        }
        let (tx, rx) = mpsc::channel();
        {
            let mut slot = lock(&self.pending);
            if slot.is_some() {
                return HostKeyVerdict::Reject(
                    "another host key confirmation is pending".to_string(),
                );
            }
            *slot = Some(PendingHostKey {
                info: info.clone(),
                reply: tx,
            });
        }
        self.ctx.notify_conflict();
        let deadline = Instant::now() + HOSTKEY_WAIT;
        let accepted = loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => break answer,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.ctx.is_cancelled() || Instant::now() >= deadline {
                        *lock(&self.pending) = None;
                        break false;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break false,
            }
        };
        if !accepted {
            return HostKeyVerdict::Reject("host key was not accepted".to_string());
        }
        let mut known = lock(&self.known);
        if let Err(err) = known.add(info) {
            return HostKeyVerdict::Reject(err.to_string());
        }
        if let Some(dir) = &self.dir
            && let Err(err) = known.save(dir)
        {
            return HostKeyVerdict::Reject(format!("could not save the host key: {err}"));
        }
        HostKeyVerdict::Trust
    }
}

/// Where a connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    Connecting,
    Ready,
    Failed(String),
}

/// One remote authority: its state and the in-memory credentials for a reconnect.
#[derive(Debug)]
pub struct Connection {
    pub state: ConnState,
    params: Option<ConnectParams>,
}

impl Connection {
    /// Whether a reconnect has credentials to reuse.
    pub fn can_reconnect(&self) -> bool {
        self.params.is_some()
    }
}

struct ConnectJob {
    key: String,
    authority: Authority,
    navigate: bool,
}

struct SpaceSlot {
    at: Instant,
    info: Option<SpaceInfo>,
    job: Option<JobId>,
}

type StagedConnect = Arc<Mutex<HashMap<JobId, Result<SftpFs, FmError>>>>;
type StagedEntries = Arc<Mutex<HashMap<JobId, Result<Arc<[Entry]>, FmError>>>>;
type StagedSpace = Arc<Mutex<HashMap<JobId, Result<Option<SpaceInfo>, FmError>>>>;

/// All remote-connection state of a [`Session`].
pub struct RemoteHub {
    config_dir: Option<PathBuf>,
    conns: BTreeMap<String, Connection>,
    known: Arc<Mutex<KnownHosts>>,
    pending: PendingSlot,
    connect_inbox: StagedConnect,
    connects: HashMap<JobId, ConnectJob>,
    pub(crate) lost: Arc<Mutex<Vec<String>>>,
    poll_inbox: StagedEntries,
    polls: HashMap<JobId, VfsUri>,
    space_inbox: StagedSpace,
    space_jobs: HashMap<JobId, VfsUri>,
    space: HashMap<VfsUri, SpaceSlot>,
    last_tick: Option<Instant>,
    /// A changed host key was refused; the UI shows a card until dismissed.
    pub host_key_alert: Option<String>,
}

impl std::fmt::Debug for RemoteHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteHub")
            .field("connections", &self.conns.len())
            .finish_non_exhaustive()
    }
}

impl Default for RemoteHub {
    fn default() -> Self {
        Self {
            config_dir: None,
            conns: BTreeMap::new(),
            known: Arc::new(Mutex::new(KnownHosts::default())),
            pending: Arc::new(Mutex::new(None)),
            connect_inbox: Arc::default(),
            connects: HashMap::new(),
            lost: Arc::default(),
            poll_inbox: Arc::default(),
            polls: HashMap::new(),
            space_inbox: Arc::default(),
            space_jobs: HashMap::new(),
            space: HashMap::new(),
            last_tick: None,
            host_key_alert: None,
        }
    }
}

impl RemoteHub {
    /// Point the hub at the config directory and load the trusted host keys.
    pub fn set_config_dir(&mut self, dir: PathBuf) {
        *lock(&self.known) = KnownHosts::load(&dir);
        self.config_dir = Some(dir);
    }

    pub fn known_hosts(&self) -> KnownHosts {
        lock(&self.known).clone()
    }

    pub fn state(&self, authority: &Authority) -> Option<&ConnState> {
        self.conns.get(&authority.to_string()).map(|c| &c.state)
    }

    pub fn connection(&self, authority: &Authority) -> Option<&Connection> {
        self.conns.get(&authority.to_string())
    }

    /// Record a connection state without connecting; lets UI snapshots draw each state.
    #[doc(hidden)]
    pub fn set_state_for_tests(&mut self, authority: &Authority, state: ConnState) {
        self.conns.insert(
            authority.to_string(),
            Connection {
                state,
                params: None,
            },
        );
    }

    /// Every known connection, by authority text.
    pub fn connections(&self) -> impl Iterator<Item = (&str, &Connection)> {
        self.conns.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The unknown host key the user must answer, if any.
    pub fn pending_host_key(&self) -> Option<HostKeyInfo> {
        lock(&self.pending).as_ref().map(|p| p.info.clone())
    }

    /// A connect, poll or space job is in flight.
    pub fn is_busy(&self) -> bool {
        !self.connects.is_empty() || !self.polls.is_empty() || !self.space_jobs.is_empty()
    }

    /// Any connection is up, so the app must wake every [`POLL_INTERVAL`] to poll.
    pub fn has_ready(&self) -> bool {
        self.conns.values().any(|c| c.state == ConnState::Ready)
    }
}

fn io_err(message: &str) -> FmError {
    FmError::Io(std::io::Error::other(message.to_string()))
}

impl Session {
    /// Remove a trusted host key (`host:port`) from memory and from `known_hosts.json`; the
    /// next connect asks again.
    pub fn forget_host_key(&mut self, id: &str) -> Result<(), FmError> {
        let mut known = match &self.remote.config_dir {
            Some(dir) => KnownHosts::load(dir),
            None => lock(&self.remote.known).clone(),
        };
        if !known.remove(id) {
            return Err(FmError::NotFound(format!("known host {id}")));
        }
        if let Some(dir) = &self.remote.config_dir {
            known.save(dir)?;
        }
        *lock(&self.remote.known) = known;
        Ok(())
    }

    /// Answer the pending host-key prompt.
    pub fn answer_host_key(&mut self, accept: bool) -> Result<(), FmError> {
        let pending = lock(&self.remote.pending).take();
        match pending {
            Some(p) => {
                let _ = p.reply.send(accept);
                Ok(())
            }
            None => Err(FmError::NotFound("pending host key".to_string())),
        }
    }

    /// Whether the focused pane's remote (if any) is connected. Mutating commands are
    /// disabled while this is `false`.
    pub fn focused_remote_ready(&self) -> bool {
        let Some(pane) = self.layout.state(self.layout.focused()) else {
            return true;
        };
        if pane.location.scheme() != Scheme::Sftp {
            return true;
        }
        match self.remote.state(pane.location.authority()) {
            Some(state) => *state == ConnState::Ready,
            None => true,
        }
    }

    /// Start connecting on the `sftp` lane; returns at once. `navigate` opens the
    /// server's start directory in the focused pane when the link comes up.
    pub fn start_connect(
        &mut self,
        params: ConnectParams,
        navigate: bool,
    ) -> Result<JobId, FmError> {
        params.validate()?;
        self.prefs.apply_sftp();
        // The file can change under a running app (edited by hand, or by another instance).
        if let Some(dir) = &self.remote.config_dir {
            *lock(&self.remote.known) = KnownHosts::load(dir);
        }
        let authority = params.authority();
        let key = authority.to_string();
        if matches!(
            self.remote.conns.get(&key).map(|c| &c.state),
            Some(ConnState::Connecting)
        ) {
            return Err(FmError::InvalidParams(format!(
                "already connecting to {key}"
            )));
        }
        self.remote.conns.insert(
            key.clone(),
            Connection {
                state: ConnState::Connecting,
                params: Some(params.clone()),
            },
        );
        let known = self.remote.known.clone();
        let pending = self.remote.pending.clone();
        let dir = self.remote.config_dir.clone();
        let inbox = self.remote.connect_inbox.clone();
        let job = self.jobs.submit(JobLane::Sftp, move |ctx| {
            let verifier = Arc::new(Tofu {
                known,
                pending,
                dir,
                ctx: ctx.clone(),
            });
            let result = fm_fs_sftp::connect(&params, verifier);
            lock(&inbox).insert(ctx.id(), result);
            Ok(())
        });
        self.remote.connects.insert(
            job,
            ConnectJob {
                key,
                authority,
                navigate,
            },
        );
        Ok(job)
    }

    /// Reconnect with the credentials kept in memory.
    pub fn reconnect(&mut self, authority: &Authority) -> Result<JobId, FmError> {
        let params = self
            .remote
            .connection(authority)
            .and_then(|c| c.params.clone())
            .ok_or_else(|| {
                FmError::InvalidParams(format!(
                    "no saved credentials for {authority}; use Connect to Server"
                ))
            })?;
        self.start_connect(params, false)
    }

    /// Close a connection and move its panes back to local disk.
    pub fn disconnect(&mut self, authority: &Authority) -> Result<(), FmError> {
        let key = authority.to_string();
        if self.remote.conns.remove(&key).is_none() {
            return Err(FmError::NotFound(format!("connection {key}")));
        }
        self.backends.unregister_at(Scheme::Sftp, authority);
        let home = VfsUri::root_file();
        let panes: Vec<_> = self
            .layout
            .states
            .iter()
            .filter(|(_, s)| {
                s.location.scheme() == Scheme::Sftp && s.location.authority() == authority
            })
            .map(|(id, _)| *id)
            .collect();
        for pane in panes {
            if let Some(state) = self.layout.state_mut(pane) {
                state.navigate(home.clone());
            }
        }
        self.ensure_read(home)?;
        Ok(())
    }

    /// Handle terminal events of connect, poll and space jobs. `true` when `ev` was one.
    pub(crate) fn finish_remote(&mut self, ev: &JobEvent) -> bool {
        let id = ev.job_id();
        if let Some(job) = self.remote.connects.remove(&id) {
            let staged = lock(&self.remote.connect_inbox).remove(&id);
            let outcome = match (staged, ev) {
                (Some(result), _) => result,
                (None, JobEvent::Failed { message, .. }) => Err(io_err(message)),
                (None, _) => Err(FmError::Cancelled),
            };
            self.finish_connect(job, outcome);
            return true;
        }
        if let Some(uri) = self.remote.polls.remove(&id) {
            let staged = lock(&self.remote.poll_inbox).remove(&id);
            match staged {
                Some(Ok(entries)) => {
                    if self.dirs.refresh(&uri, entries).is_some() {
                        self.after_read(&uri);
                    }
                }
                // A folder that listed fine a moment ago and now fails for any reason other
                // than "gone" or "denied" means the link is in trouble.
                Some(Err(FmError::NotFound(_) | FmError::PermissionDenied(_))) => {}
                Some(Err(_)) => lock(&self.remote.lost).push(uri.authority().to_string()),
                None => {}
            }
            return true;
        }
        if let Some(uri) = self.remote.space_jobs.remove(&id) {
            let staged = lock(&self.remote.space_inbox).remove(&id);
            let info = match staged {
                Some(Ok(info)) => info,
                Some(Err(err)) => {
                    self.note_transport(&uri, &err);
                    None
                }
                None => None,
            };
            if let Some(slot) = self.remote.space.get_mut(&uri) {
                slot.job = None;
                slot.at = Instant::now();
                slot.info = info;
            }
            return true;
        }
        false
    }

    fn finish_connect(&mut self, job: ConnectJob, outcome: Result<SftpFs, FmError>) {
        match outcome {
            Ok(fs) => {
                let start = fs.start_uri();
                self.backends
                    .register_at(&job.authority, std::sync::Arc::new(fs));
                if let Some(conn) = self.remote.conns.get_mut(&job.key) {
                    conn.state = ConnState::Ready;
                }
                self.info = Some((format!("Connected to {}", job.key), Instant::now()));
                if job.navigate
                    && let Ok(start) = start
                {
                    self.open_in_focused_files_pane(start);
                } else {
                    self.refresh_remote_panes();
                }
            }
            Err(err) => {
                let message = err.to_string();
                if let Some(conn) = self.remote.conns.get_mut(&job.key) {
                    conn.state = ConnState::Failed(message.clone());
                    // Only a refused login makes the stored credentials useless.
                    if matches!(err, FmError::PermissionDenied(_)) {
                        conn.params = None;
                    }
                }
                self.notice = Some(message.clone());
                if is_changed_key_message(&message) {
                    self.remote.host_key_alert = Some(message);
                }
            }
        }
    }

    fn open_in_focused_files_pane(&mut self, uri: VfsUri) {
        let focused = self.layout.focused();
        let Some(pane) = self.layout.state_mut(focused) else {
            return;
        };
        if !pane.kind.is_files() {
            return;
        }
        let root = {
            let mut top = uri.clone();
            while let Some(parent) = top.parent() {
                top = parent;
            }
            top
        };
        let tree_root = &pane.tree.root;
        if tree_root.scheme() != root.scheme()
            || tree_root.authority().to_string() != root.authority().to_string()
        {
            pane.tree = fm_layout::TreeState::new(root);
        }
        pane.navigate(uri.clone());
        if !pane.tree.active && pane.location != pane.tree.root {
            let loc = pane.location.clone();
            pane.tree.expand(loc);
        }
        self.note_visit(&uri);
        if let Err(err) = self.ensure_read(uri) {
            self.notice = Some(err.to_string());
        }
    }

    /// A transport failure on `uri`'s connection flips it to `Failed` (once).
    pub(crate) fn note_transport(&mut self, uri: &VfsUri, err: &FmError) {
        if uri.scheme() == Scheme::Sftp && fm_fs_sftp::is_transport_error(err) {
            lock(&self.remote.lost).push(uri.authority().to_string());
        }
    }

    /// Apply queued connection losses; `true` when a state changed.
    pub(crate) fn pump_remote_lost(&mut self) -> bool {
        let lost: Vec<String> = std::mem::take(&mut *lock(&self.remote.lost));
        let mut changed = false;
        for key in lost {
            if let Some(conn) = self.remote.conns.get_mut(&key)
                && conn.state == ConnState::Ready
            {
                conn.state = ConnState::Failed("connection lost".to_string());
                self.notice = Some(format!("Connection to {key} lost"));
                changed = true;
            }
        }
        changed
    }

    /// Background refresh of visible remote panes, every [`POLL_INTERVAL`].
    pub(crate) fn remote_tick(&mut self) {
        if !self.remote.has_ready() {
            return;
        }
        if self
            .remote
            .last_tick
            .is_some_and(|t| t.elapsed() < self.prefs.sftp_poll())
        {
            return;
        }
        self.remote.last_tick = Some(Instant::now());
        self.refresh_remote_panes();
    }

    /// Re-read the locations of visible Files panes on connected remotes. Hidden panes
    /// (other tabs, behind a zoomed pane) are skipped. Call on window focus regain.
    pub fn refresh_remote_panes(&mut self) {
        let panes = match self.layout.zoomed {
            Some(p) => vec![p],
            None => self.layout.panes(),
        };
        let mut uris: Vec<VfsUri> = Vec::new();
        for id in panes {
            let Some(state) = self.layout.state(id) else {
                continue;
            };
            let loc = &state.location;
            if !state.kind.is_files()
                || loc.scheme() != Scheme::Sftp
                || self.remote.state(loc.authority()) != Some(&ConnState::Ready)
                || uris.contains(loc)
            {
                continue;
            }
            uris.push(loc.clone());
        }
        for uri in uris {
            self.poll_location(uri);
        }
    }

    fn poll_location(&mut self, uri: VfsUri) {
        use crate::dircache::DirState;
        match self.dirs.state(&uri) {
            DirState::Loading { .. } => {}
            DirState::Absent | DirState::Failed { .. } => {
                if let Err(err) = self.submit_read(uri) {
                    self.notice = Some(err.to_string());
                }
            }
            DirState::Ready { .. } => {
                if self.remote.polls.values().any(|u| *u == uri) {
                    return;
                }
                let Some(backend) = self.backends.get_for(&uri) else {
                    return;
                };
                let inbox = self.remote.poll_inbox.clone();
                let target = uri.clone();
                let job = self.jobs.submit(JobLane::Sftp, move |ctx| {
                    let result = backend.read_dir(&target).map(Arc::<[Entry]>::from);
                    lock(&inbox).insert(ctx.id(), result);
                    Ok(())
                });
                self.remote.polls.insert(job, uri);
            }
        }
    }

    /// Free space for `uri`'s volume: the cached answer now, a refresh job when it is
    /// older than [`SPACE_TTL`]. Never blocks.
    pub fn free_space(&mut self, uri: &VfsUri) -> Option<SpaceInfo> {
        let now = Instant::now();
        let fresh = self
            .remote
            .space
            .get(uri)
            .is_some_and(|s| s.job.is_some() || now.duration_since(s.at) < SPACE_TTL);
        if !fresh && let Some(backend) = self.backends.get_for(uri) {
            if self.remote.space.len() >= MAX_SPACE_SLOTS {
                self.remote.space.retain(|_, s| s.job.is_some());
            }
            let inbox = self.remote.space_inbox.clone();
            let target = uri.clone();
            let lane = crate::backends::lane_for(uri.scheme());
            let job = self.jobs.submit(lane, move |ctx| {
                lock(&inbox).insert(ctx.id(), backend.space(&target));
                Ok(())
            });
            self.remote.space_jobs.insert(job, uri.clone());
            let prev = self.remote.space.get(uri).and_then(|s| s.info);
            self.remote.space.insert(
                uri.clone(),
                SpaceSlot {
                    at: now,
                    info: prev,
                    job: Some(job),
                },
            );
        }
        self.remote.space.get(uri).and_then(|s| s.info)
    }
}
