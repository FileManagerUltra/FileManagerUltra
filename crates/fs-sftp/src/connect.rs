//! Connecting: TCP + SSH handshake, host-key check, authentication, SFTP subsystem.
//!
//! Every error message is built from fixed strings plus host/user — never from the
//! password, a passphrase, or a library error that might echo input.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fm_vfs::{Authority, FmError};
use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::{RawSftpSession, SftpSession};
use secrecy::{ExposeSecret, SecretString};

use crate::fs::SftpFs;
use crate::rt;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FIELD_BYTES: usize = 1024;

/// How to prove who we are. Secrets live in zeroizing types and are never printed.
pub enum AuthMethod {
    Password(SecretString),
    Key {
        path: PathBuf,
        passphrase: Option<SecretString>,
    },
}

impl std::fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password(_) => f.write_str("AuthMethod::Password(..)"),
            Self::Key { path, .. } => write!(f, "AuthMethod::Key({})", path.display()),
        }
    }
}

impl Clone for AuthMethod {
    fn clone(&self) -> Self {
        match self {
            Self::Password(p) => Self::Password(SecretString::from(p.expose_secret().to_string())),
            Self::Key { path, passphrase } => Self::Key {
                path: path.clone(),
                passphrase: passphrase
                    .as_ref()
                    .map(|p| SecretString::from(p.expose_secret().to_string())),
            },
        }
    }
}

/// Everything needed to open one connection.
#[derive(Debug, Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthMethod,
    /// Directory to open first; `None` = the server's default (home).
    pub start_dir: Option<String>,
}

impl ConnectParams {
    /// The registry/URI authority; the default port is left implicit.
    pub fn authority(&self) -> Authority {
        Authority {
            user: Some(self.user.clone()),
            host: self.host.clone(),
            port: if self.port == 22 {
                None
            } else {
                Some(self.port)
            },
        }
    }

    pub fn validate(&self) -> Result<(), FmError> {
        let bad = |what: &str| FmError::InvalidParams(format!("sftp {what} is invalid"));
        let field_ok = |s: &str| {
            !s.is_empty()
                && s.len() <= MAX_FIELD_BYTES
                && !s.chars().any(|c| c.is_control() || c == '/' || c == '@')
                && !s.contains(char::is_whitespace)
        };
        if !field_ok(&self.host) {
            return Err(bad("host"));
        }
        if self.user.is_empty()
            || self.user.len() > MAX_FIELD_BYTES
            || self.user.chars().any(|c| c.is_control() || c == '/')
        {
            return Err(bad("user name"));
        }
        if self.port == 0 {
            return Err(bad("port"));
        }
        if let Some(dir) = &self.start_dir
            && (dir.len() > fm_vfs::MAX_PATH_BYTES || dir.chars().any(|c| c.is_control()))
        {
            return Err(bad("start directory"));
        }
        Ok(())
    }
}

/// A server key presented during the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    /// `SHA256:<base64>` as printed by OpenSSH.
    pub fingerprint: String,
    /// OpenSSH public-key line (`ssh-ed25519 AAAA…`), for the known-hosts file.
    pub openssh: String,
}

/// Answer from the host-key policy. `Reject` carries an actionable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyVerdict {
    Trust,
    Reject(String),
}

/// Decides whether to trust a server key. Called from a blocking-friendly thread, so an
/// implementation may wait for the user.
pub trait HostKeyVerifier: Send + Sync {
    fn verify(&self, info: &HostKeyInfo) -> HostKeyVerdict;
}

/// Trusts nothing: the safe default for tests that must not connect.
pub struct RejectAll;

impl HostKeyVerifier for RejectAll {
    fn verify(&self, _: &HostKeyInfo) -> HostKeyVerdict {
        HostKeyVerdict::Reject("host key not trusted".into())
    }
}

pub(crate) struct Handler {
    host: String,
    port: u16,
    verifier: Arc<dyn HostKeyVerifier>,
    rejection: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            self.note("host certificates are not supported");
            return Ok(false);
        };
        let info = HostKeyInfo {
            host: self.host.clone(),
            port: self.port,
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            openssh: key.to_openssh().unwrap_or_default(),
        };
        let verifier = self.verifier.clone();
        let verdict = tokio::task::spawn_blocking(move || verifier.verify(&info))
            .await
            .unwrap_or_else(|_| HostKeyVerdict::Reject("host key check failed".into()));
        match verdict {
            HostKeyVerdict::Trust => Ok(true),
            HostKeyVerdict::Reject(why) => {
                self.note(&why);
                Ok(false)
            }
        }
    }
}

impl Handler {
    fn note(&self, why: &str) {
        if let Ok(mut slot) = self.rejection.lock() {
            *slot = Some(why.to_string());
        }
    }
}

/// Open a connection (blocking; run on the `sftp` lane).
/// `~` and `~/…` mean the home folder, as in a shell.
fn expand_home(path: &std::path::Path) -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => path.to_path_buf(),
    }
}

/// `~` / `~/x` against the remote home; any other path is used as given.
fn expand_remote_home(dir: &str, home: &str) -> String {
    match dir.strip_prefix('~') {
        Some("") => home.to_string(),
        Some(rest) if rest.starts_with('/') => format!("{}{rest}", home.trim_end_matches('/')),
        _ => dir.to_string(),
    }
}

pub fn connect(
    params: &ConnectParams,
    verifier: Arc<dyn HostKeyVerifier>,
) -> Result<SftpFs, FmError> {
    params.validate()?;
    // Read the key before touching the network so a bad path fails fast.
    let key = match &params.auth {
        AuthMethod::Key { path, passphrase } => {
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ppk"))
            {
                return Err(FmError::backend(
                    "sftp",
                    "PuTTY .ppk keys are not supported; in PuTTYgen use Conversions > Export OpenSSH key",
                ));
            }
            let path = expand_home(path);
            let loaded = russh::keys::load_secret_key(
                &path,
                passphrase.as_ref().map(|p| p.expose_secret()),
            )
            .map_err(|_| {
                FmError::backend(
                    "sftp",
                    "cannot use the private key (unreadable, unsupported, or wrong passphrase)",
                )
            })?;
            Some(Arc::new(loaded))
        }
        AuthMethod::Password(_) => None,
    };
    let fut = connect_async(params, verifier, key);
    match rt::block_on(async { tokio::time::timeout(CONNECT_TIMEOUT * 2, fut).await })? {
        Ok(result) => result,
        Err(_) => Err(FmError::backend(
            "sftp",
            format!(
                "Connection to {}:{} timed out. Check the server address, network or VPN, and firewall.",
                params.host, params.port
            ),
        )),
    }
}

async fn connect_async(
    params: &ConnectParams,
    verifier: Arc<dyn HostKeyVerifier>,
    key: Option<Arc<russh::keys::PrivateKey>>,
) -> Result<SftpFs, FmError> {
    let host = params.host.as_str();
    let fail = |what: &str| FmError::backend("sftp", format!("{what} ({}@{host})", params.user));
    let rejection = Arc::new(Mutex::new(None));
    let handler = Handler {
        host: params.host.clone(),
        port: params.port,
        verifier,
        rejection: rejection.clone(),
    };
    let config = client::Config {
        keepalive_interval: Some(rt::keepalive_interval()),
        keepalive_max: 3,
        // Request/reply traffic: Nagle + delayed ACK would add ~40 ms per round trip.
        nodelay: true,
        ..Default::default()
    };
    let addr = (params.host.clone(), params.port);
    let connecting = client::connect(Arc::new(config), addr, handler);
    let mut session = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => {
            let why = rejection.lock().ok().and_then(|mut r| r.take());
            return Err(match why {
                Some(why) => FmError::backend("sftp", why),
                None => fail(
                    "Cannot reach the SFTP server. Check the server address, network or VPN, and firewall",
                ),
            });
        }
        Err(_) => {
            return Err(fail(
                "Connection timed out. Check the server address, network or VPN, and firewall",
            ));
        }
    };

    let user = params.user.clone();
    let authed = match (&params.auth, key) {
        (AuthMethod::Password(pw), _) => session
            .authenticate_password(user, pw.expose_secret())
            .await
            .map(|r| r.success()),
        (AuthMethod::Key { .. }, Some(key)) => {
            let hash = session
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten();
            session
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(key, hash))
                .await
                .map(|r| r.success())
        }
        (AuthMethod::Key { .. }, None) => Ok(false),
    };
    match authed {
        Ok(true) => {}
        Ok(false) => {
            return Err(FmError::PermissionDenied(format!(
                "authentication failed for {}@{host}",
                params.user
            )));
        }
        Err(_) => return Err(fail("authentication could not complete")),
    }

    let cfg = russh_sftp::client::Config::default();
    let sftp = SftpSession::new_with_config(open_sftp(&session, &fail).await?, cfg.clone())
        .await
        .map_err(|_| fail("sftp handshake failed"))?;
    let dir = RawSftpSession::new_with_config(open_sftp(&session, &fail).await?, cfg);
    dir.init()
        .await
        .map_err(|_| fail("sftp handshake failed"))?;

    // The server's default folder is the home folder; `~` and `~/x` are resolved against it
    // because the SFTP protocol knows no tilde.
    let default = sftp
        .canonicalize(".")
        .await
        .map_err(|_| fail("cannot resolve the start directory"))?;
    let start = match params.start_dir.as_deref().map(str::trim) {
        None | Some("") => default.clone(),
        Some(dir) => expand_remote_home(dir, &default),
    };
    let home = sftp.canonicalize(&start).await.unwrap_or(default);

    Ok(SftpFs::from_parts(
        session,
        sftp,
        dir,
        params.authority(),
        home,
    ))
}

async fn open_sftp(
    session: &client::Handle<Handler>,
    fail: &impl Fn(&str) -> FmError,
) -> Result<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static, FmError> {
    let channel = session
        .channel_open_session()
        .await
        .map_err(|_| fail("cannot open a channel"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|_| fail("the server refused the sftp subsystem"))?;
    Ok(channel.into_stream())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_tilde_resolves_against_the_remote_home() {
        assert_eq!(expand_remote_home("~", "/home/a"), "/home/a");
        assert_eq!(expand_remote_home("~/src", "/home/a"), "/home/a/src");
        assert_eq!(expand_remote_home("~/src", "/"), "/src");
        assert_eq!(expand_remote_home("/srv", "/home/a"), "/srv");
        assert_eq!(expand_remote_home("~bob", "/home/a"), "~bob");
        assert_eq!(expand_remote_home("rel", "/home/a"), "rel");
    }

    #[test]
    fn tilde_means_home_and_other_paths_are_untouched() {
        let abs = std::path::Path::new("/etc/key");
        assert_eq!(expand_home(abs), abs);
        let expanded = expand_home(std::path::Path::new("~/.ssh/id_rsa"));
        assert!(expanded.ends_with(".ssh/id_rsa"));
        if std::env::var_os("HOME").is_some() {
            assert!(!expanded.starts_with("~"));
        }
    }
}
