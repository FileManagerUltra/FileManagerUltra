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

use fm_fs_sftp::{
    ConnectParams, HostKeyInfo, HostKeyVerdict, HostKeyVerifier, SecretString, SftpFs,
};
use fm_fs_smb::{ConnectParams as SmbParams, SmbFs};
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

/// One remote connection: its state and the in-memory login for a reconnect.
#[derive(Debug)]
pub struct Connection {
    pub scheme: Scheme,
    pub state: ConnState,
    login: Option<Login>,
    /// The live SMB session, kept so Disconnect can close it (the SFTP link closes with
    /// its backend).
    smb: Option<SmbFs>,
}

/// Credentials kept in memory only, per protocol.
#[derive(Clone)]
pub enum Login {
    Sftp(ConnectParams),
    Smb(SmbParams),
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sftp(p) => p.fmt(f),
            Self::Smb(p) => p.fmt(f),
        }
    }
}

impl Login {
    pub fn scheme(&self) -> Scheme {
        match self {
            Self::Sftp(_) => Scheme::Sftp,
            Self::Smb(_) => Scheme::Smb,
        }
    }

    pub fn authority(&self) -> Authority {
        match self {
            Self::Sftp(p) => p.authority(),
            Self::Smb(p) => Authority {
                user: (!p.anonymous_uri).then(|| p.user.clone()),
                host: p.host.clone(),
                port: p.port,
            },
        }
    }
}

/// The key of a connection: SFTP and SMB to the same host and user stay separate.
pub fn conn_key(scheme: Scheme, authority: &Authority) -> String {
    format!("{}://{authority}", scheme.as_str())
}

impl Connection {
    /// Whether a reconnect has credentials to reuse.
    pub fn can_reconnect(&self) -> bool {
        self.login.is_some()
    }

    /// The kept SMB login (for pre-filling the sign-in popup after a refusal).
    pub(crate) fn smb_login(&self) -> Option<&SmbParams> {
        match &self.login {
            Some(Login::Smb(p)) => Some(p),
            _ => None,
        }
    }

    /// For an SMB connection: whether the session is encrypted (otherwise it is signed).
    pub fn smb_encrypted(&self) -> Option<bool> {
        self.smb.as_ref().map(SmbFs::is_encrypted)
    }
}

struct ConnectJob {
    key: String,
    scheme: Scheme,
    authority: Authority,
    navigate: bool,
    /// Where a Network-folder sign-in was headed (and where a refused one asks again).
    landing: Option<VfsUri>,
    /// Silent saved-server attempts can raise a retry dialog on failure.
    retry_prompt: Option<SftpPrompt>,
}

/// A freshly opened backend, staged by a connect job.
enum Opened {
    Sftp(SftpFs),
    Smb(SmbFs),
}

struct SpaceSlot {
    at: Instant,
    info: Option<SpaceInfo>,
    job: Option<JobId>,
}

type StagedConnect = Arc<Mutex<HashMap<JobId, Result<Opened, FmError>>>>;
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
    net_tick: Option<Instant>,
    local_tick: Option<Instant>,
    /// A changed host key was refused; the UI shows a card until dismissed.
    pub host_key_alert: Option<String>,
    /// The "Authentication Required" popup, when a Network-folder host wants a login.
    pub login_prompt: Option<crate::network::LoginPrompt>,
    /// The SFTP connect popup (Connect…, a saved server, a typed `sftp://` address).
    pub sftp_prompt: Option<SftpPrompt>,
    /// Saved SFTP servers as `servers:///` lists them (kept in step with `Prefs.profiles`).
    pub servers: crate::servers::ServerList,
    /// Passwords kept for this run only, per `user@host:port` ("Remember until you quit").
    pub(crate) sftp_creds: HashMap<String, SecretString>,
    pub(crate) creds: crate::network::CredCache,
}

/// What the SFTP connect popup starts with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SftpPrompt {
    /// The folder whose read asked for the login, opened once connected.
    pub landing: Option<VfsUri>,
    /// Open the server in the focused pane when connected.
    pub navigate: bool,
    pub host: String,
    pub port: Option<u16>,
    pub user: String,
    pub key_path: String,
    pub start_dir: String,
    /// Name of the saved profile (the one being edited, or the one opened).
    pub save_as: String,
    pub editing: bool,
    /// Why the last attempt failed; the popup stays open and shows it.
    pub error: Option<String>,
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
            net_tick: None,
            local_tick: None,
            host_key_alert: None,
            login_prompt: None,
            sftp_prompt: None,
            servers: Arc::default(),
            sftp_creds: HashMap::new(),
            creds: Default::default(),
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

    pub fn state(&self, scheme: Scheme, authority: &Authority) -> Option<&ConnState> {
        self.conns
            .get(&conn_key(scheme, authority))
            .map(|c| &c.state)
    }

    pub fn connection(&self, scheme: Scheme, authority: &Authority) -> Option<&Connection> {
        self.conns.get(&conn_key(scheme, authority))
    }

    /// Record a connection state without connecting; lets UI snapshots draw each state.
    #[doc(hidden)]
    pub fn set_state_for_tests(&mut self, scheme: Scheme, authority: &Authority, state: ConnState) {
        self.conns.insert(
            conn_key(scheme, authority),
            Connection {
                scheme,
                state,
                login: None,
                smb: None,
            },
        );
    }

    /// Every known connection, by `scheme://authority` text.
    pub fn connections(&self) -> impl Iterator<Item = (&str, &Connection)> {
        self.conns.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Whether a login for `host` is remembered until quit (the password is never exposed).
    pub fn remembers_login(&self, host: &str, port: Option<u16>) -> bool {
        self.creds.contains(host, port)
    }

    /// The unknown host key the user must answer, if any.
    pub fn pending_host_key(&self) -> Option<HostKeyInfo> {
        lock(&self.pending).as_ref().map(|p| p.info.clone())
    }

    /// A connect, poll or space job is in flight.
    pub fn is_busy(&self) -> bool {
        !self.connects.is_empty() || !self.polls.is_empty() || !self.space_jobs.is_empty()
    }

    /// A connect job is in flight (background polls do not count).
    pub fn is_connecting(&self) -> bool {
        !self.connects.is_empty()
    }

    /// Any connection is up, so the app must wake every [`POLL_INTERVAL`] to poll.
    pub fn has_ready(&self) -> bool {
        self.conns.values().any(|c| c.state == ConnState::Ready)
    }
}

/// Schemes served by a connection (as opposed to local disk).
pub fn is_remote(scheme: Scheme) -> bool {
    matches!(scheme, Scheme::Sftp | Scheme::Smb)
}

/// Whether `err` means the link itself is gone, for `scheme`'s backend.
pub fn is_transport_failure(scheme: Scheme, err: &FmError) -> bool {
    match scheme {
        Scheme::Sftp => fm_fs_sftp::is_transport_error(err),
        Scheme::Smb => fm_fs_smb::is_transport_error(err),
        Scheme::File | Scheme::Network | Scheme::Servers => false,
    }
}

/// Whether a failed connect was a refused login (so the stored credentials are useless).
pub(crate) fn is_login_refused(scheme: Scheme, err: &FmError) -> bool {
    match scheme {
        Scheme::Smb => {
            matches!(err, FmError::PermissionDenied(_))
                || err.to_string().contains("authentication failed")
        }
        _ => matches!(err, FmError::PermissionDenied(_)),
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
        if !is_remote(pane.location.scheme()) {
            return true;
        }
        match self
            .remote
            .state(pane.location.scheme(), pane.location.authority())
        {
            Some(state) => *state == ConnState::Ready,
            None => true,
        }
    }

    /// Start connecting an SFTP server on the `sftp` lane; returns at once. `navigate`
    /// opens the server's start directory in the focused pane when the link comes up.
    pub fn start_connect(
        &mut self,
        params: ConnectParams,
        navigate: bool,
    ) -> Result<JobId, FmError> {
        self.connect_sftp_to(params, navigate, None)
    }

    /// Like [`Self::start_connect`]; `landing` is the folder whose read triggered it.
    pub(crate) fn connect_sftp_to(
        &mut self,
        params: ConnectParams,
        navigate: bool,
        landing: Option<VfsUri>,
    ) -> Result<JobId, FmError> {
        params.validate()?;
        self.prefs.apply_sftp();
        // The file can change under a running app (edited by hand, or by another instance).
        if let Some(dir) = &self.remote.config_dir {
            *lock(&self.remote.known) = KnownHosts::load(dir);
        }
        let login = Login::Sftp(params.clone());
        let retry_prompt = if self.remote.sftp_prompt.is_none() && landing.is_some() {
            Some(self.sftp_retry_prompt(&params, navigate, landing.clone()))
        } else {
            None
        };
        let (key, authority) = self.begin_connect(login)?;
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
            let result = fm_fs_sftp::connect(&params, verifier).map(Opened::Sftp);
            lock(&inbox).insert(ctx.id(), result);
            Ok(())
        });
        self.remote.connects.insert(
            job,
            ConnectJob {
                key,
                scheme: Scheme::Sftp,
                authority,
                navigate,
                landing,
                retry_prompt,
            },
        );
        Ok(job)
    }

    /// Start connecting an SMB server on the `smb` lane; returns at once.
    pub fn start_connect_smb(
        &mut self,
        params: SmbParams,
        navigate: bool,
    ) -> Result<JobId, FmError> {
        self.connect_smb_to(params, navigate, None)
    }

    pub(crate) fn connect_smb_to(
        &mut self,
        mut params: SmbParams,
        navigate: bool,
        landing: Option<VfsUri>,
    ) -> Result<JobId, FmError> {
        params.require_encryption |= self.prefs.smb_require_encryption;
        let (key, authority) = self.begin_connect(Login::Smb(params.clone()))?;
        let inbox = self.remote.connect_inbox.clone();
        let job = self.jobs.submit(JobLane::Smb, move |ctx| {
            let result = fm_fs_smb::connect(&params).map(Opened::Smb);
            lock(&inbox).insert(ctx.id(), result);
            Ok(())
        });
        self.remote.connects.insert(
            job,
            ConnectJob {
                key,
                scheme: Scheme::Smb,
                authority,
                navigate,
                landing,
                retry_prompt: None,
            },
        );
        Ok(job)
    }

    /// Mark a connection as connecting and keep its login for a reconnect.
    fn begin_connect(&mut self, login: Login) -> Result<(String, Authority), FmError> {
        let scheme = login.scheme();
        let authority = login.authority();
        let key = conn_key(scheme, &authority);
        if matches!(
            self.remote.conns.get(&key).map(|c| &c.state),
            Some(ConnState::Connecting)
        ) {
            return Err(FmError::InvalidParams(format!(
                "already connecting to {key}"
            )));
        }
        if let Some(old) = self.remote.conns.remove(&key).and_then(|c| c.smb) {
            old.disconnect();
        }
        self.remote.conns.insert(
            key.clone(),
            Connection {
                scheme,
                state: ConnState::Connecting,
                login: Some(login),
                smb: None,
            },
        );
        Ok((key, authority))
    }

    /// Reconnect with the credentials kept in memory.
    pub fn reconnect(&mut self, scheme: Scheme, authority: &Authority) -> Result<JobId, FmError> {
        let login = self
            .remote
            .connection(scheme, authority)
            .and_then(|c| c.login.clone())
            .ok_or_else(|| {
                FmError::InvalidParams(format!(
                    "no saved credentials for {authority}; use Connect to Server"
                ))
            })?;
        match login {
            Login::Sftp(p) => self.start_connect(p, false),
            Login::Smb(p) => self.start_connect_smb(p, false),
        }
    }

    /// Close a connection and move its panes back to local disk.
    pub fn disconnect(&mut self, scheme: Scheme, authority: &Authority) -> Result<(), FmError> {
        let key = conn_key(scheme, authority);
        let Some(conn) = self.remote.conns.remove(&key) else {
            return Err(FmError::NotFound(format!("connection {key}")));
        };
        if let Some(smb) = conn.smb {
            smb.disconnect();
        }
        self.backends.unregister_at(scheme, authority);
        let ours = |u: &VfsUri| u.scheme() == scheme && u.authority() == authority;
        // Forget everything listed from the server, so a reconnect starts from a clean slate.
        self.dirs.invalidate_where(ours);
        let home = crate::workspace::home_location();
        let mut sftp_panes = Vec::new();
        let mut virtual_roots = Vec::new();
        let mut local_panes = Vec::new();
        for (id, st) in self.layout.states.iter_mut() {
            let on_server = ours(&st.location) || ours(&st.tree.root);
            let expanded: Vec<VfsUri> = st.tree.expanded().filter(|u| ours(u)).cloned().collect();
            if !on_server && expanded.is_empty() {
                continue;
            }
            for uri in &expanded {
                st.tree.collapse(uri);
            }
            if st.kind == fm_layout::PaneKind::Sftp {
                sftp_panes.push(*id);
            } else if matches!(st.tree.root.scheme(), Scheme::Network | Scheme::Servers) {
                if ours(&st.location) {
                    let root = st.tree.root.clone();
                    st.navigate(root.clone());
                    virtual_roots.push(root);
                }
            } else if on_server {
                local_panes.push(*id);
            }
        }
        for pane in sftp_panes {
            self.open_servers(pane)?;
        }
        for pane in local_panes {
            if let Some(state) = self.layout.state_mut(pane) {
                state.tree = fm_layout::TreeState::new(VfsUri::root_file());
                state.navigate(home.clone());
            }
        }
        for root in virtual_roots {
            self.ensure_read(root)?;
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
                // A local folder that vanished shows the read error instead of a stale listing.
                Some(Err(FmError::NotFound(_))) if uri.scheme() == Scheme::File => {
                    self.watch.forget(&uri);
                    if let Err(err) = self.refresh_read(uri) {
                        self.notice = Some(err.to_string());
                    }
                }
                Some(Err(FmError::NotFound(_) | FmError::PermissionDenied(_))) => {}
                Some(Err(_)) => {
                    lock(&self.remote.lost).push(conn_key(uri.scheme(), uri.authority()))
                }
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

    fn finish_connect(&mut self, job: ConnectJob, outcome: Result<Opened, FmError>) {
        match outcome {
            Ok(opened) => {
                let (start, smb) = match opened {
                    Opened::Sftp(fs) => {
                        let start = fs.start_uri();
                        self.backends.register_at(&job.authority, Arc::new(fs));
                        (start, None)
                    }
                    Opened::Smb(fs) => {
                        let start = fs.start_uri();
                        self.backends
                            .register_at(&job.authority, Arc::new(fs.clone()));
                        (start, Some(fs))
                    }
                };
                if let Some(conn) = self.remote.conns.get_mut(&job.key) {
                    conn.state = ConnState::Ready;
                    conn.smb = smb;
                }
                self.info = Some((format!("Connected to {}", job.key), Instant::now()));
                if job.scheme == Scheme::Sftp {
                    self.remote.sftp_prompt = None;
                }
                // A saved server opens at `sftp://user@host/`; land in the start folder (home).
                let mut to_start = false;
                let landing = job.landing.map(|l| match &start {
                    Ok(start) if job.scheme == Scheme::Sftp && l.path_str() == "/" => {
                        to_start = true;
                        start.clone()
                    }
                    _ => l,
                });
                // Connecting from a server row in the tree opens its start folder in the pane.
                let from_servers = to_start
                    && self
                        .layout
                        .state(self.layout.focused())
                        .is_some_and(|st| st.location.scheme() == Scheme::Servers);
                if let Some(landing) = landing {
                    self.dirs.invalidate(&landing);
                    if job.navigate || from_servers {
                        self.open_in_focused_files_pane(landing);
                    } else if let Err(err) = self.ensure_read(landing) {
                        self.notice = Some(err.to_string());
                    }
                } else if job.navigate
                    && let Ok(start) = start
                {
                    self.open_in_focused_files_pane(start);
                } else {
                    self.refresh_remote_panes();
                }
            }
            Err(err) => {
                let message = err.to_string();
                if job.scheme == Scheme::Sftp && self.remote.sftp_prompt.is_none() {
                    self.remote.sftp_prompt = job.retry_prompt;
                }
                let in_popup = job.scheme == Scheme::Sftp && self.remote.sftp_prompt.is_some();
                if in_popup {
                    if let Some(prompt) = self.remote.sftp_prompt.as_mut() {
                        prompt.error = Some(message.clone());
                    }
                } else if let Some(landing) = job.landing.clone() {
                    self.network_login_failed(landing, &err, job.navigate);
                }
                if let Some(conn) = self.remote.conns.get_mut(&job.key) {
                    conn.state = ConnState::Failed(message.clone());
                    // Only a refused login makes the stored credentials useless.
                    if is_login_refused(job.scheme, &err) {
                        conn.login = None;
                    }
                }
                if !in_popup
                    && (job.landing.is_none()
                        || job.scheme != Scheme::Smb
                        || !is_login_refused(job.scheme, &err))
                {
                    self.notice = Some(message.clone());
                }
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
        // A tree rooted at the Network folder keeps every host it reaches.
        let in_network = matches!(tree_root.scheme(), Scheme::Network | Scheme::Servers);
        if !in_network
            && (tree_root.scheme() != root.scheme()
                || tree_root.authority().to_string() != root.authority().to_string())
        {
            pane.tree = fm_layout::TreeState::new(root);
        }
        pane.navigate(uri.clone());
        // Connecting is not a tree keystroke: scroll the tree to the landing folder even
        // when the tree had keyboard focus (e.g. the connect came from clicking its row).
        pane.tree.reveal = pane.location != pane.tree.root;
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
        if is_transport_failure(uri.scheme(), err) {
            lock(&self.remote.lost).push(conn_key(uri.scheme(), uri.authority()));
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
        self.network_tick();
        self.local_tick();
        if !self.remote.has_ready() {
            return;
        }
        if self
            .remote
            .last_tick
            .is_some_and(|t| t.elapsed() < self.prefs.remote_poll())
        {
            return;
        }
        self.remote.last_tick = Some(Instant::now());
        self.refresh_remote_panes();
    }

    /// Keep OS watches on the visible local folders and re-read the ones that changed.
    /// Folders without a live watch (network mounts, watch limits) are polled every 2 s.
    fn local_tick(&mut self) {
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
            if state.kind.is_files() && loc.scheme() == Scheme::File && !uris.contains(loc) {
                uris.push(loc.clone());
            }
        }
        let mounts: Vec<String> = self
            .volumes
            .entries()
            .iter()
            .filter(|d| crate::watch::is_network_filesystem(&d.filesystem))
            .filter_map(|d| d.mount.local_path_string())
            .collect();
        if let Some(notice) = self.watch.sync(&uris, &mounts) {
            self.notice = Some(notice);
        }
        for uri in self.watch.drain(Instant::now()) {
            self.poll_location(uri);
        }
        if self
            .remote
            .local_tick
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(2))
        {
            return;
        }
        self.remote.local_tick = Some(Instant::now());
        for uri in uris {
            if !self.watch.covers(&uri) {
                self.poll_location(uri);
            }
        }
    }

    /// Discovery keeps finding hosts after the first listing: re-read the Network folder
    /// twice a second while a visible pane shows it.
    fn network_tick(&mut self) {
        if self
            .remote
            .net_tick
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(500))
        {
            return;
        }
        if self.shows_network() {
            self.remote.net_tick = Some(Instant::now());
            self.poll_location(VfsUri::network());
        }
    }

    /// Whether a visible pane shows the Network folder (location or tree root).
    pub fn shows_network(&self) -> bool {
        let panes = match self.layout.zoomed {
            Some(p) => vec![p],
            None => self.layout.panes(),
        };
        let net = VfsUri::network();
        panes.into_iter().any(|id| {
            self.layout
                .state(id)
                .is_some_and(|st| st.location == net || st.tree.root == net)
        })
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
                || !is_remote(loc.scheme())
                || self.remote.state(loc.scheme(), loc.authority()) != Some(&ConnState::Ready)
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
                let job = self
                    .jobs
                    .submit(crate::backends::lane_for(uri.scheme()), move |ctx| {
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
