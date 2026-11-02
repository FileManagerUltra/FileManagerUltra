//! Opening a session: TCP connect, negotiate, authenticate, tree-connect one share.

use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fm_vfs::{Authority, FmError, MAX_DIR_ENTRIES};
use secrecy::{ExposeSecret, SecretString};
use smb::connection::EncryptionMode;
use smb::{Client, ClientConfig, ConnectionConfig, UncPath};

use crate::errors::map_err;
use crate::fs::{Inner, SmbFs};

pub const DEFAULT_OP_TIMEOUT_SECS: u64 = 30;

/// Everything needed to open one connection. The password never leaves a
/// [`SecretString`]; `Debug` shows nothing secret.
#[derive(Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: Option<u16>,
    /// The share opened first (and shown as the start folder); others connect on demand.
    /// Empty: sign in only, and start at the server root, which lists the server's shares.
    pub share: String,
    pub user: String,
    /// Windows domain / workgroup; folded into the account name (`DOMAIN\user`).
    pub domain: Option<String>,
    pub password: SecretString,
    pub timeout: Duration,
    /// Fail the connection unless the server encrypts the session. Sessions are always
    /// signed; unsigned (guest/anonymous) access is never allowed.
    pub require_encryption: bool,
    /// Address the server as `smb://host/` (no user name), so URIs from the Network
    /// folder and typed addresses resolve to this connection.
    pub anonymous_uri: bool,
}

impl std::fmt::Debug for ConnectParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("share", &self.share)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
}

/// Accept a bare host or a Windows UNC server name; shares belong in the share field.
pub fn normalize_host(input: &str) -> Result<String, FmError> {
    let input = input.trim();
    let host = match input.strip_prefix(r"\\") {
        Some(host) => host.strip_suffix('\\').unwrap_or(host),
        None => input,
    };
    if !valid_host(host) {
        return Err(FmError::backend(
            "smb",
            r"Enter a host name, IP address or \\server; enter the share separately in Share.",
        ));
    }
    Ok(host.to_string())
}

impl ConnectParams {
    /// Reject malformed host, share and user before any network traffic.
    pub fn validate(&self) -> Result<(), FmError> {
        if !valid_host(&self.host) {
            return Err(FmError::backend("smb", "invalid host name"));
        }
        if !self.share.is_empty() && !crate::fs::valid_share(&self.share) {
            return Err(FmError::backend("smb", "invalid share name"));
        }
        if self.user.is_empty() || self.user.chars().any(|c| c.is_control()) {
            return Err(FmError::backend("smb", "invalid user name"));
        }
        Ok(())
    }
}

/// Open a connection and return a ready [`SmbFs`]. Blocking; run it on the `smb` lane.
pub fn connect(params: &ConnectParams) -> Result<SmbFs, FmError> {
    params.validate()?;
    let account = match params.domain.as_deref().filter(|d| !d.is_empty()) {
        Some(d) => format!("{d}\\{}", params.user),
        None => params.user.clone(),
    };

    let mut connection = ConnectionConfig {
        timeout: Some(params.timeout),
        allow_unsigned_guest_access: false,
        encryption_mode: if params.require_encryption {
            EncryptionMode::Required
        } else {
            EncryptionMode::Allowed
        },
        ..ConnectionConfig::default()
    };
    connection.port = params.port;
    let client = Client::new(ClientConfig {
        connection,
        ..ClientConfig::default()
    });

    let server = match params.port {
        Some(p) => format!("{}:{p}", params.host),
        None => params.host.clone(),
    };
    let password = params.password.expose_secret().to_string();
    let mut ipc = false;
    if params.share.is_empty() {
        // Authenticate against the pipes share; this is also what lists the shares.
        client
            .ipc_connect(&server, &account, password)
            .map_err(|e| map_err(&e, &server))?;
        ipc = true;
    } else {
        let first = UncPath::from_str(&format!("\\\\{server}\\{}", params.share))
            .map_err(|e| map_err(&e, &params.share))?;
        client
            .share_connect(&first, &account, password)
            .map_err(|e| map_err(&e, &params.share))?;
    }

    Ok(SmbFs {
        inner: Arc::new(Inner {
            client,
            server,
            account,
            password: params.password.clone(),
            shares: Mutex::new(if params.share.is_empty() {
                Vec::new()
            } else {
                vec![params.share.clone()]
            }),
            ipc: AtomicBool::new(ipc),
            authority: Authority {
                user: (!params.anonymous_uri).then(|| params.user.clone()),
                host: params.host.clone(),
                port: params.port,
            },
            max_entries: MAX_DIR_ENTRIES,
            dead: AtomicBool::new(false),
            encrypted: params.require_encryption,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bare_and_unc_hosts_without_changing_server_identity() {
        for (input, expected) in [
            ("nas", "nas"),
            ("192.168.0.50", "192.168.0.50"),
            (r"\\192.168.0.50", "192.168.0.50"),
            (r" \\NAS\ ", "NAS"),
            ("[::1]", "[::1]"),
        ] {
            assert_eq!(normalize_host(input).expect("host"), expected);
        }
    }

    #[test]
    fn rejects_paths_credentials_and_oversized_hosts_without_echoing_input() {
        for input in [
            "",
            r"\\",
            r"\nas",
            r"\\nas\share",
            r"\\nas\share\folder",
            "smb://nas",
            "alice:secret@nas",
            "nas\nhost",
            &"x".repeat(256),
        ] {
            let err = normalize_host(input).expect_err("invalid host");
            let message = err.to_string();
            assert!(message.contains("enter the share separately"));
            assert!(!message.contains("secret"));
        }
    }
}
