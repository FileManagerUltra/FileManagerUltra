//! Remote commands: connect, reconnect, disconnect, host-key answers and profiles.

use fm_fs_sftp::{AuthMethod, ConnectParams, SecretString};
use fm_vfs::{Authority, FmError, Scheme};

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

const MAX_FIELD: usize = 4096;

fn text(
    map: Option<&serde_json::Map<String, Params>>,
    key: &str,
) -> Result<Option<String>, FmError> {
    let Some(value) = map.and_then(|m| m.get(key)) else {
        return Ok(None);
    };
    match value {
        Params::Null => Ok(None),
        Params::String(s) if s.len() <= MAX_FIELD => Ok(Some(s.clone())),
        _ => Err(FmError::InvalidParams(format!(
            "{key} must be a string of at most {MAX_FIELD} bytes"
        ))),
    }
}

fn port(map: Option<&serde_json::Map<String, Params>>) -> Result<Option<u16>, FmError> {
    match map.and_then(|m| m.get("port")) {
        None | Some(Params::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n != 0)
            .map(Some)
            .ok_or_else(|| FmError::InvalidParams("port must be 1-65535".to_string())),
    }
}

/// The remote authority the focused Files pane shows, if any.
fn focused_authority(s: &Session) -> Option<Authority> {
    let pane = s.layout.state(s.layout.focused())?;
    (pane.location.scheme() == Scheme::Sftp).then(|| pane.location.authority().clone())
}

fn parse_connect(s: &Session, params: &Params) -> Result<ConnectParams, FmError> {
    let map = object(params)?;
    let profile = match text(map, "profile")? {
        Some(name) => Some(
            s.prefs
                .profiles
                .iter()
                .find(|p| p.name == name)
                .cloned()
                .ok_or_else(|| FmError::NotFound(format!("profile {name}")))?,
        ),
        None => None,
    };
    let host = text(map, "host")?
        .or_else(|| profile.as_ref().map(|p| p.host.clone()))
        .ok_or_else(|| FmError::InvalidParams("host is required".to_string()))?;
    let user = text(map, "user")?
        .or_else(|| profile.as_ref().map(|p| p.user.clone()))
        .ok_or_else(|| FmError::InvalidParams("user is required".to_string()))?;
    let port = port(map)?
        .or_else(|| profile.as_ref().map(|p| p.port))
        .unwrap_or(22);
    let start_dir = text(map, "start_dir")?.or_else(|| profile.as_ref()?.start_dir.clone());
    let key_path = text(map, "key_path")?.or_else(|| profile.as_ref()?.key_path.clone());
    let auth = match (text(map, "password")?, key_path) {
        (Some(password), _) => AuthMethod::Password(SecretString::from(password)),
        (None, Some(path)) => AuthMethod::Key {
            path: path.into(),
            passphrase: text(map, "passphrase")?.map(SecretString::from),
        },
        (None, None) => {
            return Err(FmError::InvalidParams(
                "password or key_path is required".to_string(),
            ));
        }
    };
    let params = ConnectParams {
        host,
        port,
        user,
        auth,
        start_dir,
    };
    params.validate()?;
    Ok(params)
}

/// `remote.connect.dialog` — turn the focused pane into an SFTP pane (inline connect form).
pub fn connect_dialog() -> CommandSpec {
    CommandSpec {
        id: "remote.connect.dialog",
        label: "Connect to Server…",
        menu: &["File", "Connect to Server…"],
        shortcut: None,
        params: "none",
        enabled: |s| s.focused_kind().is_some_and(|k| k.is_files()),
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            s.dispatch(
                "pane.kind.set",
                &serde_json::json!({"kind": "sftp", "pane": pane}),
            )
        },
    }
}

/// `remote.connect` — connect to an SFTP server on the `sftp` lane.
pub fn connect() -> CommandSpec {
    CommandSpec {
        id: "remote.connect",
        label: "Connect",
        menu: &["File", "Connect to Server…", "Connect"],
        shortcut: None,
        params: "host, user, password | key_path (+ passphrase?), port?, start_dir?, profile?",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let connect = parse_connect(s, params)?;
            s.start_connect(connect, true).map(|_| ())
        },
    }
}

/// `remote.reconnect` — reconnect the focused pane's server with the in-memory login.
pub fn reconnect() -> CommandSpec {
    CommandSpec {
        id: "remote.reconnect",
        label: "Reconnect",
        menu: &["File", "Reconnect"],
        shortcut: None,
        params: "none",
        enabled: |s| {
            focused_authority(s)
                .and_then(|a| s.remote.connection(&a))
                .is_some_and(|c| {
                    matches!(c.state, crate::remote::ConnState::Failed(_)) && c.can_reconnect()
                })
        },
        run: |s, _p: &Params| {
            let authority = focused_authority(s)
                .ok_or_else(|| FmError::NotFound("remote connection".to_string()))?;
            s.reconnect(&authority).map(|_| ())
        },
    }
}

/// `remote.disconnect` — close the focused pane's connection.
pub fn disconnect() -> CommandSpec {
    CommandSpec {
        id: "remote.disconnect",
        label: "Disconnect",
        menu: &["File", "Disconnect"],
        shortcut: None,
        params: "none",
        enabled: |s| focused_authority(s).is_some_and(|a| s.remote.connection(&a).is_some()),
        run: |s, _p: &Params| {
            let authority = focused_authority(s)
                .ok_or_else(|| FmError::NotFound("remote connection".to_string()))?;
            s.disconnect(&authority)
        },
    }
}

/// `remote.hostkey.answer` — accept or refuse the unknown host key being shown.
pub fn hostkey_answer() -> CommandSpec {
    CommandSpec {
        id: "remote.hostkey.answer",
        label: "Answer Host Key Prompt",
        menu: &["File", "Connect to Server…", "Host key"],
        shortcut: None,
        params: "accept: bool",
        enabled: |s| s.remote.pending_host_key().is_some(),
        run: |s, params: &Params| {
            let accept = object(params)?
                .and_then(|m| m.get("accept"))
                .and_then(|v| v.as_bool())
                .ok_or_else(|| {
                    FmError::InvalidParams("accept must be true or false".to_string())
                })?;
            s.answer_host_key(accept)
        },
    }
}

/// `remote.hostkey.forget` — stop trusting a host; the next connect asks again.
pub fn hostkey_forget() -> CommandSpec {
    CommandSpec {
        id: "remote.hostkey.forget",
        label: "Forget Host Key",
        menu: &["File", "Connect to Server…", "Forget host key"],
        shortcut: None,
        params: "host: \"host:port\"",
        enabled: |s| !s.remote.known_hosts().is_empty(),
        run: |s, params: &Params| {
            let host = object(params)?
                .and_then(|m| m.get("host"))
                .and_then(|v| v.as_str())
                .filter(|h| !h.is_empty() && h.len() <= 300)
                .ok_or_else(|| FmError::InvalidParams("host: \"host:port\" required".to_string()))?
                .to_string();
            s.forget_host_key(&host)
        },
    }
}

/// `remote.profile.save` — add or replace a saved connection (never a secret).
pub fn profile_save() -> CommandSpec {
    CommandSpec {
        id: "remote.profile.save",
        label: "Save Connection Profile",
        menu: &["File", "Connect to Server…", "Save profile"],
        shortcut: None,
        params: "name, host, user, port?, key_path?, start_dir?",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let map = object(params)?;
            let need = |key: &str| {
                text(map, key)?.ok_or_else(|| FmError::InvalidParams(format!("{key} is required")))
            };
            let profile = crate::prefs::Profile {
                name: need("name")?,
                host: need("host")?,
                user: need("user")?,
                port: port(map)?.unwrap_or(22),
                key_path: text(map, "key_path")?,
                start_dir: text(map, "start_dir")?,
            };
            profile.validate()?;
            let existing = s.prefs.profiles.iter().position(|p| p.name == profile.name);
            match existing {
                Some(ix) => s.prefs.profiles[ix] = profile,
                None if s.prefs.profiles.len() >= crate::prefs::MAX_PROFILES => {
                    return Err(FmError::InvalidParams(format!(
                        "at most {} profiles",
                        crate::prefs::MAX_PROFILES
                    )));
                }
                None => s.prefs.profiles.push(profile),
            }
            Ok(())
        },
    }
}

/// `remote.profile.delete` — remove a saved connection by name.
pub fn profile_delete() -> CommandSpec {
    CommandSpec {
        id: "remote.profile.delete",
        label: "Delete Connection Profile",
        menu: &["File", "Connect to Server…", "Delete profile"],
        shortcut: None,
        params: "name: string",
        enabled: |s| !s.prefs.profiles.is_empty(),
        run: |s, params: &Params| {
            let name = text(object(params)?, "name")?
                .ok_or_else(|| FmError::InvalidParams("name is required".to_string()))?;
            let before = s.prefs.profiles.len();
            s.prefs.profiles.retain(|p| p.name != name);
            if s.prefs.profiles.len() == before {
                return Err(FmError::NotFound(format!("profile {name}")));
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        connect_dialog(),
        connect(),
        reconnect(),
        disconnect(),
        hostkey_answer(),
        hostkey_forget(),
        profile_save(),
        profile_delete(),
    ]
}
