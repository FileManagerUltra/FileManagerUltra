//! The Network folder: opening it, and the "Authentication Required" sign-in for hosts
//! found there (capability `network-browsing`).
//!
//! Passwords are only ever held as `SecretString`s in memory; nothing here touches disk.

use std::collections::HashMap;
use std::time::Duration;

use fm_fs_smb::{ConnectParams as SmbParams, SecretString};
use fm_layout::{PaneId, PaneKind, TreeState};
use fm_vfs::{FmError, Scheme, VfsUri};

use crate::remote::{ConnState, conn_key, is_login_refused};
use crate::session::Session;

pub const DEFAULT_DOMAIN: &str = "WORKGROUP";

/// The open sign-in popup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginPrompt {
    /// Where the user was headed; the connect lands there.
    pub uri: VfsUri,
    pub host: String,
    pub port: Option<u16>,
    /// Why the previous attempt failed, shown inside the popup.
    pub error: Option<String>,
    pub user: String,
    pub domain: String,
    /// The focused pane was on `uri`, so it should follow the connect.
    pub navigate: bool,
    /// Connect to Server: the popup also asks for the host, port, share and encryption.
    pub manual: bool,
}

/// The extra fields of a manual connect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manual {
    pub host: String,
    pub port: Option<u16>,
    pub share: String,
    pub require_encryption: bool,
}

/// How long a typed password may be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remember {
    /// Use it for this connect only.
    Forget,
    /// Keep it in memory until the app quits (never written to disk).
    Session,
}

impl Remember {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "forget" => Some(Self::Forget),
            "session" => Some(Self::Session),
            _ => None,
        }
    }
}

struct Cred {
    user: String,
    domain: Option<String>,
    password: SecretString,
}

/// Logins remembered "until you quit", per `host[:port]`.
#[derive(Default)]
pub struct CredCache(HashMap<String, Cred>);

impl std::fmt::Debug for CredCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredCache")
            .field("hosts", &self.0.len())
            .finish_non_exhaustive()
    }
}

fn cache_key(host: &str, port: Option<u16>) -> String {
    match port {
        Some(p) => format!("{}:{p}", host.to_lowercase()),
        None => host.to_lowercase(),
    }
}

impl CredCache {
    fn put(&mut self, p: &SmbParams) {
        self.0.insert(
            cache_key(&p.host, p.port),
            Cred {
                user: p.user.clone(),
                domain: p.domain.clone(),
                password: p.password.clone(),
            },
        );
    }

    pub fn forget(&mut self, host: &str, port: Option<u16>) {
        self.0.remove(&cache_key(host, port));
    }

    pub fn contains(&self, host: &str, port: Option<u16>) -> bool {
        self.0.contains_key(&cache_key(host, port))
    }

    fn params(&self, host: &str, port: Option<u16>, timeout: Duration) -> Option<SmbParams> {
        let c = self.0.get(&cache_key(host, port))?;
        Some(SmbParams {
            host: host.to_string(),
            port,
            share: String::new(),
            user: c.user.clone(),
            domain: c.domain.clone(),
            password: c.password.clone(),
            timeout,
            require_encryption: false,
            anonymous_uri: true,
        })
    }
}

impl Session {
    fn smb_timeout(&self) -> Duration {
        Duration::from_secs(self.prefs.sftp_timeout_secs.clamp(1, 600))
    }

    /// Point `pane` at the Network folder (tree rooted there).
    pub fn open_network(&mut self, pane: PaneId) -> Result<(), FmError> {
        let net = VfsUri::network();
        let state = self
            .layout
            .state_mut(pane)
            .ok_or_else(|| FmError::NotFound(format!("pane {pane}")))?;
        state.tree = TreeState::new(net.clone());
        state.navigate(net.clone());
        self.refresh_read(net).map(|_| ())
    }

    /// An unconnected SFTP host opened from the Network folder asks first (popup) before the
    /// pane turns into an SFTP pane. `true` when it took over.
    pub(crate) fn open_sftp_form(&mut self, uri: &VfsUri) -> bool {
        let in_network = self.layout.state(self.layout.focused()).is_some_and(|st| {
            st.tree.root.scheme() == Scheme::Network && st.kind != PaneKind::Sftp
        });
        if uri.scheme() != Scheme::Sftp
            || uri.authority().user.is_some()
            || self.backends.get_for(uri).is_some()
            || !in_network
        {
            return false;
        }
        self.remote.sftp_prompt = Some(uri.clone());
        true
    }

    /// Popup answer "Open SFTP Pane": the focused pane becomes an SFTP pane on the host.
    pub fn accept_sftp_prompt(&mut self) -> Result<(), FmError> {
        let uri = self
            .remote
            .sftp_prompt
            .take()
            .ok_or_else(|| FmError::NotFound("SFTP prompt".to_string()))?;
        let focused = self.layout.focused();
        self.dispatch("pane.kind.set", &serde_json::json!({"kind": "sftp"}))?;
        if let Some(state) = self.layout.state_mut(focused) {
            state.navigate(uri);
        }
        Ok(())
    }

    /// Popup answer "Cancel": the node goes back to closed.
    pub fn cancel_sftp_prompt(&mut self) {
        if let Some(uri) = self.remote.sftp_prompt.take() {
            self.abandon(&uri);
        }
    }

    /// A read of `uri` found no backend. For a user-less `smb://` host this signs in: with a
    /// remembered login straight away, otherwise through the popup. `true` when handled.
    pub(crate) fn needs_login(&mut self, uri: &VfsUri) -> bool {
        let auth = uri.authority();
        if uri.scheme() != Scheme::Smb || self.backends.get_for(uri).is_some() {
            return false;
        }
        if self.remote.login_prompt.is_some()
            || matches!(
                self.remote.state(Scheme::Smb, auth),
                Some(ConnState::Connecting)
            )
        {
            return true;
        }
        let navigate = self
            .layout
            .state(self.layout.focused())
            .is_some_and(|st| st.location == *uri);
        if auth.user.is_none()
            && let Some(params) =
                self.remote
                    .creds
                    .params(&auth.host, auth.port, self.smb_timeout())
        {
            if let Err(err) = self.connect_smb_to(params, navigate, Some(uri.clone())) {
                self.notice = Some(err.to_string());
            }
            return true;
        }
        self.remote.login_prompt = Some(LoginPrompt {
            uri: uri.clone(),
            host: auth.host.clone(),
            port: auth.port,
            error: None,
            user: auth.user.clone().unwrap_or_default(),
            domain: DEFAULT_DOMAIN.to_string(),
            navigate,
            manual: false,
        });
        true
    }

    /// Open the popup for a server that was not discovered (Connect to Server).
    pub fn open_manual_login(&mut self) {
        self.remote.login_prompt = Some(LoginPrompt {
            uri: VfsUri::network(),
            host: String::new(),
            port: None,
            error: None,
            user: String::new(),
            domain: DEFAULT_DOMAIN.to_string(),
            navigate: true,
            manual: true,
        });
    }

    /// Submit the popup. A bad login keeps the popup open.
    pub fn submit_login(
        &mut self,
        user: String,
        password: String,
        domain: Option<String>,
        remember: Remember,
        manual: Option<Manual>,
    ) -> Result<(), FmError> {
        let prompt = self
            .remote
            .login_prompt
            .clone()
            .ok_or_else(|| FmError::NotFound("sign-in prompt".to_string()))?;
        let (host, port, share, require_encryption, landing) = if prompt.manual {
            let m = manual.ok_or_else(|| FmError::InvalidParams("host is required".to_string()))?;
            let shown_port = m.port.map(|p| format!(":{p}")).unwrap_or_default();
            let landing = VfsUri::parse(&format!("smb://{}{shown_port}/{}", m.host, m.share))
                .map_err(|_| FmError::InvalidParams("invalid host or share".to_string()))?;
            (m.host, m.port, m.share, m.require_encryption, landing)
        } else {
            (
                prompt.host.clone(),
                prompt.port,
                String::new(),
                false,
                prompt.uri.clone(),
            )
        };
        let params = SmbParams {
            host,
            port,
            share,
            user,
            domain: domain.filter(|d| !d.is_empty()),
            password: SecretString::from(password),
            timeout: self.smb_timeout(),
            require_encryption,
            // A typed `smb://user@host/` keeps its user in the address.
            anonymous_uri: landing.authority().user.is_none(),
        };
        params.validate()?;
        if remember == Remember::Session {
            self.remote.creds.put(&params);
        }
        self.remote.login_prompt = None;
        self.connect_smb_to(params, prompt.navigate, Some(landing))
            .map(|_| ())
    }

    /// Close the popup without connecting; the node the user opened goes back to closed.
    pub fn cancel_login(&mut self) {
        if let Some(prompt) = self.remote.login_prompt.take() {
            self.abandon(&prompt.uri);
        }
    }

    fn abandon(&mut self, uri: &VfsUri) {
        let net = VfsUri::network();
        for state in self.layout.states.values_mut() {
            state.tree.collapse(uri);
            if state.location == *uri {
                if state.tree.root.scheme() == Scheme::Network {
                    state.navigate(net.clone());
                } else {
                    state.go_back();
                }
            }
        }
        if let Err(err) = self.ensure_read(net) {
            self.notice = Some(err.to_string());
        }
    }

    /// A Network-folder connect failed: a refused login asks again (with the reason),
    /// anything else gives the node up and lets the caller show the error.
    pub(crate) fn network_login_failed(&mut self, landing: VfsUri, err: &FmError, navigate: bool) {
        let auth = landing.authority().clone();
        let key = conn_key(Scheme::Smb, &auth);
        let (user, domain) = self
            .remote
            .connections()
            .find(|(k, _)| *k == key)
            .and_then(|(_, c)| c.smb_login())
            .map(|p| (p.user.clone(), p.domain.clone()))
            .unwrap_or_default();
        self.remote.creds.forget(&auth.host, auth.port);
        if is_login_refused(Scheme::Smb, err) {
            self.remote.login_prompt = Some(LoginPrompt {
                uri: landing,
                host: auth.host,
                port: auth.port,
                error: Some("Login failed. Check the user name, domain and password.".to_string()),
                user,
                domain: domain.unwrap_or_else(|| DEFAULT_DOMAIN.to_string()),
                navigate,
                manual: false,
            });
        } else {
            self.abandon(&landing);
        }
    }
}

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

/// `network.open` — show the Network folder in the focused pane.
pub fn open() -> CommandSpec {
    CommandSpec {
        id: "network.open",
        label: "Network",
        menu: &["File", "Network"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            if let Some(state) = s.layout.state_mut(pane)
                && !state.kind.is_files()
            {
                state.kind = fm_layout::PaneKind::Network;
            }
            s.open_network(pane)
        },
    }
}

/// `network.connect.dialog` — ask for a server by name (Connect to Server).
pub fn connect_dialog() -> CommandSpec {
    CommandSpec {
        id: "network.connect.dialog",
        label: "Connect to SMB Server…",
        menu: &["File", "Connect to SMB Server…"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            s.open_manual_login();
            Ok(())
        },
    }
}

/// `network.login` — answer the Authentication Required popup.
pub fn login() -> CommandSpec {
    CommandSpec {
        id: "network.login",
        label: "Connect",
        menu: &["File", "Network", "Connect"],
        shortcut: None,
        params: "user, password, domain?, remember?: \"forget\"|\"session\"; manual prompt: host, port?, share?, require_encryption?",
        enabled: |s| s.remote.login_prompt.is_some(),
        run: |s, params: &Params| {
            let map = object(params)?
                .ok_or_else(|| FmError::InvalidParams("user and password required".to_string()))?;
            let text = |key: &str| match map.get(key) {
                None | Some(Params::Null) => Ok(None),
                Some(v) => v
                    .as_str()
                    .map(|t| Some(t.to_string()))
                    .ok_or_else(|| FmError::InvalidParams(format!("{key} must be a string"))),
            };
            let user = text("user")?
                .ok_or_else(|| FmError::InvalidParams("user is required".to_string()))?;
            let password = text("password")?
                .ok_or_else(|| FmError::InvalidParams("password is required".to_string()))?;
            let remember = match text("remember")? {
                None => Remember::Forget,
                Some(r) => Remember::parse(&r).ok_or_else(|| {
                    FmError::InvalidParams("remember must be forget or session".to_string())
                })?,
            };
            let manual = match text("host")? {
                None => None,
                Some(host) => Some(Manual {
                    host,
                    port: match map.get("port") {
                        None | Some(Params::Null) => None,
                        Some(v) => Some(
                            v.as_u64()
                                .and_then(|p| u16::try_from(p).ok())
                                .filter(|p| *p != 0)
                                .ok_or_else(|| {
                                    FmError::InvalidParams("port must be 1-65535".to_string())
                                })?,
                        ),
                    },
                    share: text("share")?.unwrap_or_default(),
                    require_encryption: map
                        .get("require_encryption")
                        .and_then(Params::as_bool)
                        .unwrap_or(false),
                }),
            };
            s.submit_login(user, password, text("domain")?, remember, manual)
        },
    }
}

/// `network.sftp.open` — answer the SFTP popup by opening the SFTP pane.
pub fn sftp_open() -> CommandSpec {
    CommandSpec {
        id: "network.sftp.open",
        label: "Open SFTP Pane",
        menu: &["File", "Network", "Open SFTP Pane"],
        shortcut: None,
        params: "none",
        enabled: |s| s.remote.sftp_prompt.is_some(),
        run: |s, _p: &Params| s.accept_sftp_prompt(),
    }
}

/// `network.sftp.cancel` — dismiss the SFTP popup.
pub fn sftp_cancel() -> CommandSpec {
    CommandSpec {
        id: "network.sftp.cancel",
        label: "Cancel SFTP",
        menu: &["File", "Network", "Cancel SFTP"],
        shortcut: None,
        params: "none",
        enabled: |s| s.remote.sftp_prompt.is_some(),
        run: |s, _p: &Params| {
            s.cancel_sftp_prompt();
            Ok(())
        },
    }
}

/// `network.login.cancel` — dismiss the popup.
pub fn cancel() -> CommandSpec {
    CommandSpec {
        id: "network.login.cancel",
        label: "Cancel",
        menu: &["File", "Network", "Cancel"],
        shortcut: None,
        params: "none",
        enabled: |s| s.remote.login_prompt.is_some(),
        run: |s, _p: &Params| {
            s.cancel_login();
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        open(),
        connect_dialog(),
        login(),
        cancel(),
        sftp_open(),
        sftp_cancel(),
    ]
}
