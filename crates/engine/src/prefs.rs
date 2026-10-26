//! Preferences persistence (`prefs.toml`) — non-secret settings only.
//!
//! The schema is **closed** (capability `security`): passwords, passphrases, private
//! keys and session tokens must never gain a field here; state files are written
//! owner-only (`0600` file / `0700` dir on Unix) via a temp file + atomic rename.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use fm_vfs::FmError;
use serde::{Deserialize, Serialize};

/// Which theme the app should use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the OS setting.
    #[default]
    System,
    Dark,
    Light,
}

/// The complete preferences schema. **Closed**: only non-secret, UI-level settings
/// may ever live here — the closed-schema test asserts the exact key set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub window_maximized: bool,
    #[serde(default)]
    pub last_dir: Option<String>,
    /// Copying a tree points absolute symlinks that lie inside it at the copy, not the
    /// original.
    #[serde(default = "default_true")]
    pub retarget_links: bool,
    /// A folder in the preview pane counts (and thumbnails) every file below it.
    #[serde(default)]
    pub recursive_preview: bool,
    /// Saved SFTP connections. Never holds a password or passphrase.
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Seconds between background refreshes of remote panes (also how fast a dead link shows).
    #[serde(default = "default_remote_poll", alias = "sftp_poll_secs")]
    pub remote_poll_secs: u64,
    /// Seconds a remote call may stall before the link counts as lost.
    #[serde(default = "default_remote_timeout", alias = "sftp_timeout_secs")]
    pub remote_timeout_secs: u64,
    /// Seconds between keepalive probes on an idle link (applies to new connections).
    #[serde(default = "default_remote_keepalive", alias = "sftp_keepalive_secs")]
    pub remote_keepalive_secs: u64,
    /// Refuse SMB connections that are not encrypted (applies to new connections).
    #[serde(default)]
    pub smb_require_encryption: bool,
    /// UI zoom factor (Ctrl+Shift+Plus / Minus), restored on start.
    #[serde(default = "default_zoom")]
    pub zoom: f32,
}

/// Allowed UI zoom range and the step of one zoom command.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;
pub const ZOOM_STEP: f32 = 0.1;

fn default_zoom() -> f32 {
    1.0
}
/// Allowed ranges (seconds) for the SFTP timing preferences.
pub const REMOTE_POLL_RANGE: std::ops::RangeInclusive<u64> = 1..=60;
pub const REMOTE_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 2..=120;
pub const REMOTE_KEEPALIVE_RANGE: std::ops::RangeInclusive<u64> = 1..=60;

fn default_remote_poll() -> u64 {
    5
}
fn default_remote_timeout() -> u64 {
    fm_fs_sftp::DEFAULT_OP_TIMEOUT_SECS
}
fn default_remote_keepalive() -> u64 {
    fm_fs_sftp::DEFAULT_KEEPALIVE_SECS
}

impl Prefs {
    /// Push the SFTP timeouts to the backend (call after load and after every change).
    pub fn apply_sftp(&self) {
        fm_fs_sftp::set_timeouts(
            self.remote_timeout_secs
                .clamp(*REMOTE_TIMEOUT_RANGE.start(), *REMOTE_TIMEOUT_RANGE.end()),
            self.remote_keepalive_secs.clamp(
                *REMOTE_KEEPALIVE_RANGE.start(),
                *REMOTE_KEEPALIVE_RANGE.end(),
            ),
        );
    }

    /// The remote-pane refresh interval, clamped to the allowed range.
    pub fn remote_poll(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.remote_poll_secs
                .clamp(*REMOTE_POLL_RANGE.start(), *REMOTE_POLL_RANGE.end()),
        )
    }
}

/// Most saved connection profiles.
pub const MAX_PROFILES: usize = 50;
const MAX_PROFILE_FIELD: usize = 1024;

/// A saved SFTP connection: where to connect, never how to authenticate secretly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Absent in old files, which are all SFTP.
    #[serde(default, skip_serializing_if = "Protocol::is_default")]
    pub protocol: Protocol,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    /// Path of a private key file (the key itself is never stored).
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub start_dir: Option<String>,
    /// SMB only: authentication domain / workgroup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// SMB only: share to open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<String>,
}

/// Which backend a saved profile connects with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Sftp,
    Smb,
}

impl Protocol {
    fn is_default(&self) -> bool {
        *self == Self::Sftp
    }
}

fn default_port() -> u16 {
    22
}

impl Profile {
    /// Reject empty, oversized or control-character fields.
    pub fn validate(&self) -> Result<(), FmError> {
        let bad = |what: &str| FmError::InvalidParams(format!("profile {what} is invalid"));
        let ok = |s: &str| {
            !s.is_empty() && s.len() <= MAX_PROFILE_FIELD && !s.chars().any(|c| c.is_control())
        };
        let ok_opt = |s: &Option<String>| s.as_deref().is_none_or(ok);
        if !ok(&self.name) {
            return Err(bad("name"));
        }
        if !ok(&self.host) || self.host.contains(char::is_whitespace) {
            return Err(bad("host"));
        }
        if !ok(&self.user) {
            return Err(bad("user"));
        }
        if self.port == 0 {
            return Err(bad("port"));
        }
        if !ok_opt(&self.key_path) {
            return Err(bad("key path"));
        }
        if !ok_opt(&self.start_dir) {
            return Err(bad("start directory"));
        }
        if !ok_opt(&self.domain) {
            return Err(bad("domain"));
        }
        if !ok_opt(&self.share)
            || self
                .share
                .as_deref()
                .is_some_and(|s| s.contains(['/', '\\']))
        {
            return Err(bad("share"));
        }
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: ThemePreference::default(),
            window_maximized: false,
            last_dir: None,
            retarget_links: true,
            recursive_preview: false,
            profiles: Vec::new(),
            remote_poll_secs: default_remote_poll(),
            remote_timeout_secs: default_remote_timeout(),
            remote_keepalive_secs: default_remote_keepalive(),
            smb_require_encryption: false,
            zoom: default_zoom(),
        }
    }
}

impl Prefs {
    /// The zoom to apply, safe against hand-edited values (NaN, out of range).
    pub fn zoom_factor(&self) -> f32 {
        if self.zoom.is_finite() {
            self.zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end())
        } else {
            1.0
        }
    }
}

/// File name inside the config directory.
pub const PREFS_FILE: &str = "prefs.toml";

/// Hard cap on the prefs file; larger files are ignored (defaults win).
pub const MAX_PREFS_BYTES: u64 = 10 * 1024 * 1024;

/// Platform config directory: `~/.config/<slug>/` (Linux),
/// `~/Library/Application Support/<slug>/` (macOS), `%APPDATA%\<slug>\` (Windows).
pub fn default_config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", fm_brand::APP_SLUG)
        .map(|dirs| dirs.config_dir().to_path_buf())
}

/// Platform cache directory for regenerable data (thumbnails).
pub fn default_cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", fm_brand::APP_SLUG)
        .map(|dirs| dirs.cache_dir().to_path_buf())
}

/// Load preferences from `path`. Missing, unreadable, corrupt or oversized files
/// yield `Prefs::default()` — load never fails and never panics.
pub fn load(path: &Path) -> Prefs {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return Prefs::default(),
    };
    if size > MAX_PREFS_BYTES {
        return Prefs::default();
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Prefs::default(),
    };
    toml::from_str(&text).unwrap_or_default()
}

/// Save preferences into `dir`/`prefs.toml` (creating the directory, owner-only).
pub fn save(dir: &Path, prefs: &Prefs) -> Result<(), FmError> {
    let text = toml::to_string(prefs).map_err(|err| FmError::Io(std::io::Error::other(err)))?;
    write_atomic(dir, PREFS_FILE, text.as_bytes())
}

/// Write `dir`/`file` owner-only through a temp file and an atomic rename.
pub(crate) fn write_atomic(dir: &Path, file: &str, bytes: &[u8]) -> Result<(), FmError> {
    create_config_dir(dir)?;
    let final_path = dir.join(file);
    let tmp_path = dir.join(format!("{file}.tmp"));
    write_owner_only(&tmp_path, bytes)?;
    std::fs::rename(&tmp_path, &final_path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp_path);
        FmError::Io(err)
    })
}

fn create_config_dir(dir: &Path) -> Result<(), FmError> {
    std::fs::create_dir_all(dir).map_err(FmError::Io)?;
    set_owner_only_dir(dir)
}

#[cfg(unix)]
fn set_owner_only_dir(dir: &Path) -> Result<(), FmError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(FmError::Io)
}

#[cfg(not(unix))]
fn set_owner_only_dir(_dir: &Path) -> Result<(), FmError> {
    // Windows: ACLs are inherited from the per-user profile; the scaffold relies on that.
    Ok(())
}

fn write_owner_only(path: &Path, bytes: &[u8]) -> Result<(), FmError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path).map_err(FmError::Io)?;
    file.write_all(bytes).map_err(FmError::Io)?;
    file.sync_all().map_err(FmError::Io)?;
    // Re-assert in case the file already existed with wider permissions.
    set_owner_only_file(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> Result<(), FmError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(FmError::Io)
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> Result<(), FmError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        tempfile::tempdir().map(|d| d.keep()).unwrap_or_else(|_| {
            // Extremely unlikely; fall back to a unique path under the OS temp dir.
            std::env::temp_dir().join(format!("fm-prefs-test-{}", std::process::id()))
        })
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = temp_dir();
        let prefs = load(&dir.join(PREFS_FILE));
        assert_eq!(prefs, Prefs::default());
    }

    #[test]
    fn round_trip_theme() {
        let dir = temp_dir();
        let prefs = Prefs {
            theme: ThemePreference::Dark,
            window_maximized: true,
            last_dir: Some("/home/alice".to_string()),
            retarget_links: false,
            recursive_preview: true,
            profiles: vec![
                Profile {
                    name: "nas".to_string(),
                    host: "nas.local".to_string(),
                    port: 2222,
                    user: "alice".to_string(),
                    key_path: Some("/home/alice/.ssh/id_ed25519".to_string()),
                    start_dir: None,
                    protocol: Protocol::Sftp,
                    domain: None,
                    share: None,
                },
                Profile {
                    protocol: Protocol::Smb,
                    name: "pc".to_string(),
                    host: "pc.local".to_string(),
                    port: 445,
                    user: "bob".to_string(),
                    key_path: None,
                    start_dir: None,
                    domain: Some("WORKGROUP".to_string()),
                    share: Some("Public".to_string()),
                },
            ],
            ..Prefs::default()
        };
        save(&dir, &prefs).expect("save");
        assert_eq!(load(&dir.join(PREFS_FILE)), prefs);
    }

    #[test]
    fn invalid_toml_yields_defaults_and_keeps_file() {
        let dir = temp_dir();
        let path = dir.join(PREFS_FILE);
        std::fs::write(&path, "not = = valid toml [[[").expect("write");
        assert_eq!(load(&path), Prefs::default());
        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            after, "not = = valid toml [[[",
            "load must not rewrite the file"
        );
    }

    #[test]
    fn oversized_file_yields_defaults() {
        let dir = temp_dir();
        let path = dir.join(PREFS_FILE);
        let big = "x".repeat((MAX_PREFS_BYTES + 1) as usize);
        std::fs::write(&path, big).expect("write");
        assert_eq!(load(&path), Prefs::default());
    }

    #[test]
    fn many_unknown_fields_yield_defaults() {
        let dir = temp_dir();
        let path = dir.join(PREFS_FILE);
        let mut doc = String::new();
        for i in 0..10_000 {
            doc.push_str(&format!("junk{i} = {i}\n"));
        }
        std::fs::write(&path, doc).expect("write");
        // Unknown fields are ignored; known fields fall back to defaults.
        assert_eq!(load(&path), Prefs::default());
    }

    #[test]
    fn closed_schema_has_exactly_the_allowed_keys() {
        let prefs = Prefs {
            theme: ThemePreference::Dark,
            window_maximized: false,
            last_dir: Some("/tmp".to_string()),
            retarget_links: true,
            recursive_preview: false,
            profiles: Vec::new(),
            ..Prefs::default()
        };
        let text = toml::to_string(&prefs).expect("serialize");
        let parsed: toml::Table = toml::from_str(&text).expect("parse");
        let mut keys: Vec<String> = parsed.keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "last_dir",
                "profiles",
                "recursive_preview",
                "remote_keepalive_secs",
                "remote_poll_secs",
                "remote_timeout_secs",
                "retarget_links",
                "smb_require_encryption",
                "theme",
                "window_maximized",
                "zoom"
            ]
        );
        // No secret-ish key can appear.
        for key in &keys {
            let lower = key.to_ascii_lowercase();
            for banned in ["pass", "secret", "token", "key", "credential"] {
                assert!(
                    !lower.contains(banned),
                    "schema grew a suspicious key: {key}"
                );
            }
        }
    }

    #[test]
    fn profile_fields_are_closed_and_validated() {
        let profile = Profile {
            name: "nas".to_string(),
            host: "nas.local".to_string(),
            port: 22,
            user: "alice".to_string(),
            key_path: None,
            start_dir: Some("/srv".to_string()),
            protocol: Protocol::Sftp,
            domain: None,
            share: None,
        };
        assert!(profile.validate().is_ok());
        let text = toml::to_string(&Prefs {
            profiles: vec![profile.clone()],
            ..Prefs::default()
        })
        .expect("serialize");
        let parsed: toml::Table = toml::from_str(&text).expect("parse");
        let row = parsed["profiles"][0].as_table().expect("table");
        let mut keys: Vec<&str> = row.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["host", "name", "port", "start_dir", "user"]);
        for bad in [
            Profile {
                host: "a b".into(),
                ..profile.clone()
            },
            Profile {
                name: String::new(),
                ..profile.clone()
            },
            Profile {
                user: "x".repeat(2000),
                ..profile.clone()
            },
            Profile {
                port: 0,
                ..profile.clone()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn old_files_load_with_legacy_timing_keys_and_sftp_profiles() {
        let dir = temp_dir();
        let path = dir.join(PREFS_FILE);
        std::fs::write(
            &path,
            "sftp_poll_secs = 7\nsftp_timeout_secs = 9\nsftp_keepalive_secs = 11\n\
             [[profiles]]\nname = \"nas\"\nhost = \"nas.local\"\nuser = \"alice\"\n",
        )
        .expect("write");
        let prefs = load(&path);
        assert_eq!(prefs.remote_poll_secs, 7);
        assert_eq!(prefs.remote_timeout_secs, 9);
        assert_eq!(prefs.remote_keepalive_secs, 11);
        assert_eq!(prefs.profiles.len(), 1);
        assert_eq!(prefs.profiles[0].protocol, Protocol::Sftp);
        assert_eq!(prefs.profiles[0].port, 22);
        assert!(prefs.profiles[0].share.is_none());
    }

    #[test]
    fn smb_profiles_round_trip_without_secrets() {
        let profile = Profile {
            protocol: Protocol::Smb,
            name: "pc".into(),
            host: "pc.local".into(),
            port: 445,
            user: "bob".into(),
            key_path: None,
            start_dir: None,
            domain: Some("WORKGROUP".into()),
            share: Some("Public".into()),
        };
        assert!(profile.validate().is_ok());
        let text = toml::to_string(&Prefs {
            profiles: vec![profile.clone()],
            ..Prefs::default()
        })
        .expect("serialize");
        assert!(!text.to_ascii_lowercase().contains("pass"));
        let back: Prefs = toml::from_str(&text).expect("parse");
        assert_eq!(back.profiles, vec![profile.clone()]);
        let bad = Profile {
            share: Some("a/b".into()),
            ..profile
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn failed_temp_write_preserves_existing_prefs() {
        let dir = temp_dir();
        let prefs = Prefs {
            theme: ThemePreference::Light,
            ..Prefs::default()
        };
        save(&dir, &prefs).expect("save");
        let path = dir.join(PREFS_FILE);
        let before = std::fs::read_to_string(&path).expect("read");
        // Make the temp path unusable: a directory in the way.
        std::fs::create_dir(dir.join(format!("{PREFS_FILE}.tmp"))).expect("mkdir trap");
        let other = Prefs {
            theme: ThemePreference::Dark,
            ..Prefs::default()
        };
        assert!(save(&dir, &other).is_err());
        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(before, after, "existing prefs must survive a failed save");
    }

    #[cfg(unix)]
    #[test]
    fn unix_permissions_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        save(&dir, &Prefs::default()).expect("save");
        let dir_mode = std::fs::metadata(&dir).expect("meta").permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(dir.join(PREFS_FILE))
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "config dir must be owner-only");
        assert_eq!(file_mode, 0o600, "prefs file must be owner-only");
    }

    #[test]
    fn default_config_dir_is_absolute_and_slug_scoped() {
        let dir = default_config_dir().expect("config dir on this platform");
        assert!(dir.is_absolute(), "{dir:?} should be absolute");
        assert!(
            dir.to_string_lossy().contains(fm_brand::APP_SLUG),
            "{dir:?} should be scoped to the brand slug"
        );
    }
}
