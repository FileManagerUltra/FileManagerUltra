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

/// Which shell terminal panes start (capability `terminal-pane`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalShell {
    /// PowerShell on Windows, the login shell elsewhere.
    #[default]
    Default,
    PowerShell,
    Cmd,
}

impl TerminalShell {
    pub fn to_term(self) -> fm_term::Shell {
        match self {
            TerminalShell::Default => fm_term::Shell::Default,
            TerminalShell::PowerShell => fm_term::Shell::PowerShell,
            TerminalShell::Cmd => fm_term::Shell::Cmd,
        }
    }
}

/// Row spacing of listings and trees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Normal,
    Comfortable,
}

/// How file sizes are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeUnits {
    /// KiB, MiB (1024).
    #[default]
    Binary,
    /// kB, MB (1000).
    Decimal,
    /// Exact byte counts.
    Bytes,
}

/// How modification times are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    /// 2026-10-28 13:40
    #[default]
    Iso,
    /// 2026-10-28 13:40:05
    IsoSeconds,
    /// 10/28/2026 1:40 PM
    Us,
    /// 28/10/2026 13:40
    Eu,
}

/// Last normal window geometry; only non-secret numeric values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Native logical pixels, independent of application zoom.
    pub size: [f32; 2],
    /// Outer position in physical desktop pixels (unavailable on native Wayland).
    pub position: Option<[f32; 2]>,
    pub scale_factor: f32,
}

impl WindowGeometry {
    pub fn is_valid(&self) -> bool {
        self.size
            .iter()
            .all(|v| v.is_finite() && (1.0..=32768.0).contains(v))
            && self
                .position
                .is_none_or(|pos| pos.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.0))
            && self.scale_factor.is_finite()
            && (0.25..=16.0).contains(&self.scale_factor)
    }
}

/// The complete preferences schema. **Closed**: only non-secret, UI-level settings
/// may ever live here — the closed-schema test asserts the exact key set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "default_true")]
    pub check_updates: bool,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub window_maximized: bool,
    #[serde(default)]
    pub window_geometry: Option<WindowGeometry>,
    #[serde(default)]
    pub last_dir: Option<String>,
    /// Copying a tree points absolute symlinks that lie inside it at the copy, not the
    /// original.
    #[serde(default = "default_true")]
    pub retarget_links: bool,
    /// A folder in the preview pane counts (and thumbnails) every file below it.
    #[serde(default)]
    pub recursive_preview: bool,
    #[serde(default = "default_log_poll_ms")]
    pub log_poll_ms: u64,
    /// Zero removes the initial-window limit, not the preview safety caps.
    #[serde(default = "default_log_tail_mib")]
    pub log_tail_mib: u64,
    #[serde(default)]
    pub preview_filters: crate::preview_types::PreviewFilters,
    /// Show git status in listings and trees (capability `git-status`).
    #[serde(default = "default_true")]
    pub git_enabled: bool,
    /// Status letter in a gutter column.
    #[serde(default = "default_true")]
    pub git_gutter: bool,
    /// Tint file names by git status.
    #[serde(default = "default_true")]
    pub git_tint: bool,
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
    /// User bookmarks shown under Places (capability `places`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<Bookmark>,
    /// Shell for new terminal panes.
    #[serde(default)]
    pub terminal_shell: TerminalShell,
    /// Retained visual rows for newly opened terminal sessions.
    #[serde(default = "default_terminal_scrollback")]
    pub terminal_scrollback_rows: u64,
    /// Accent colour as `#rrggbb`; `None` keeps the theme's own.
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub density: Density,
    /// Interface text size in points.
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    /// Terminal text size in points.
    #[serde(default = "default_font_size")]
    pub terminal_font_size: f32,
    #[serde(default)]
    pub size_units: SizeUnits,
    #[serde(default)]
    pub date_format: DateFormat,
    /// Start with dot-files visible.
    #[serde(default)]
    pub show_hidden: bool,
    /// A single click opens a file or folder.
    #[serde(default)]
    pub single_click_open: bool,
    /// Ask before a permanent delete.
    #[serde(default = "default_true")]
    pub confirm_delete: bool,
    /// Ask before running a program.
    #[serde(default = "default_true")]
    pub confirm_run: bool,
    #[serde(default = "default_thumb_memory")]
    pub thumb_memory_mib: u64,
    #[serde(default = "default_thumb_disk")]
    pub thumb_disk_mib: u64,
    /// Parent folder of the thumbnail cache; `None` uses the platform cache directory.
    #[serde(default)]
    pub thumb_cache_dir: Option<String>,
    /// Longest a preview may take to render.
    #[serde(default = "default_preview_secs")]
    pub preview_time_secs: u64,
}

pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 9.0..=24.0;
pub const THUMB_MEMORY_RANGE: std::ops::RangeInclusive<u64> = 32..=2048;
pub const THUMB_DISK_RANGE: std::ops::RangeInclusive<u64> = 32..=8192;
pub const PREVIEW_TIME_RANGE: std::ops::RangeInclusive<u64> = 2..=120;

fn default_font_size() -> f32 {
    14.0
}
fn default_thumb_memory() -> u64 {
    192
}
fn default_thumb_disk() -> u64 {
    256
}
fn default_preview_secs() -> u64 {
    15
}

/// An absolute, sane folder path for the thumbnail cache.
pub fn valid_cache_parent(dir: &Path) -> bool {
    let text = dir.to_string_lossy();
    dir.is_absolute() && text.len() <= 4096 && !text.chars().any(char::is_control)
}

/// Parse `#rrggbb` into its channels.
pub fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Most bookmarks kept; longest bookmark name / uri.
pub const MAX_BOOKMARKS: usize = 200;
pub const MAX_BOOKMARK_NAME: usize = 100;
const MAX_BOOKMARK_URI: usize = 4096;

/// A folder pinned under Places.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub uri: String,
}

impl Bookmark {
    /// A non-empty, single-line name and a parseable uri, both within their caps.
    pub fn is_valid(&self) -> bool {
        let name = self.name.trim();
        !name.is_empty()
            && self.name.chars().count() <= MAX_BOOKMARK_NAME
            && !self.name.chars().any(char::is_control)
            && self.uri.len() <= MAX_BOOKMARK_URI
            && fm_vfs::VfsUri::parse(&self.uri).is_ok()
    }
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
pub const LOG_POLL_RANGE: std::ops::RangeInclusive<u64> = 100..=60_000;
pub const LOG_TAIL_RANGE: std::ops::RangeInclusive<u64> = 1..=64;

fn default_log_poll_ms() -> u64 {
    500
}

fn default_log_tail_mib() -> u64 {
    8
}

fn default_remote_poll() -> u64 {
    5
}
fn default_remote_timeout() -> u64 {
    fm_fs_sftp::DEFAULT_OP_TIMEOUT_SECS
}
fn default_remote_keepalive() -> u64 {
    fm_fs_sftp::DEFAULT_KEEPALIVE_SECS
}

fn default_terminal_scrollback() -> u64 {
    10_000
}

impl Prefs {
    pub fn log_poll(&self) -> std::time::Duration {
        std::time::Duration::from_millis(
            self.log_poll_ms
                .clamp(*LOG_POLL_RANGE.start(), *LOG_POLL_RANGE.end()),
        )
    }

    pub fn log_limits(&self) -> fm_preview::log::LogLimits {
        let limits = fm_preview::log::LogLimits::default();
        let initial_tail = if self.log_tail_mib == 0 {
            limits.max_bytes
        } else {
            self.log_tail_mib
                .clamp(*LOG_TAIL_RANGE.start(), *LOG_TAIL_RANGE.end())
                << 20
        };
        fm_preview::log::LogLimits {
            initial_tail,
            ..limits
        }
    }

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
            check_updates: true,
            theme: ThemePreference::default(),
            window_maximized: false,
            window_geometry: None,
            last_dir: None,
            retarget_links: true,
            recursive_preview: false,
            log_poll_ms: default_log_poll_ms(),
            log_tail_mib: default_log_tail_mib(),
            preview_filters: crate::preview_types::PreviewFilters::default(),
            git_enabled: true,
            git_gutter: true,
            git_tint: true,
            profiles: Vec::new(),
            remote_poll_secs: default_remote_poll(),
            remote_timeout_secs: default_remote_timeout(),
            remote_keepalive_secs: default_remote_keepalive(),
            smb_require_encryption: false,
            zoom: default_zoom(),
            bookmarks: Vec::new(),
            terminal_shell: TerminalShell::default(),
            terminal_scrollback_rows: default_terminal_scrollback(),
            accent: None,
            density: Density::default(),
            font_size: default_font_size(),
            terminal_font_size: default_font_size(),
            size_units: SizeUnits::default(),
            date_format: DateFormat::default(),
            show_hidden: false,
            single_click_open: false,
            confirm_delete: true,
            confirm_run: true,
            thumb_memory_mib: default_thumb_memory(),
            thumb_disk_mib: default_thumb_disk(),
            thumb_cache_dir: None,
            preview_time_secs: default_preview_secs(),
        }
    }
}

impl Prefs {
    pub fn terminal_scrollback_rows(&self) -> usize {
        self.terminal_scrollback_rows.clamp(5_000, 1_000_000) as usize
    }
    fn clamp_f32(value: f32, default: f32) -> f32 {
        if value.is_finite() {
            value.clamp(*FONT_SIZE_RANGE.start(), *FONT_SIZE_RANGE.end())
        } else {
            default
        }
    }

    /// Interface font size, safe against hand-edited values.
    pub fn ui_font_size(&self) -> f32 {
        Self::clamp_f32(self.font_size, default_font_size())
    }

    pub fn term_font_size(&self) -> f32 {
        Self::clamp_f32(self.terminal_font_size, default_font_size())
    }

    /// The accent override, if it is a valid colour.
    pub fn accent_rgb(&self) -> Option<[u8; 3]> {
        self.accent.as_deref().and_then(parse_hex_color)
    }

    /// Where thumbnails live: a `thumbs` folder inside the chosen directory (so clearing
    /// or pruning never touches the user's own files), else the platform cache.
    pub fn thumb_cache_path(&self) -> Option<PathBuf> {
        match self.thumb_cache_dir.as_deref().map(Path::new) {
            Some(dir) if valid_cache_parent(dir) => Some(dir.join("thumbs")),
            _ => default_cache_dir().map(|d| d.join("thumbs")),
        }
    }

    pub fn preview_budget(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.preview_time_secs
                .clamp(*PREVIEW_TIME_RANGE.start(), *PREVIEW_TIME_RANGE.end()),
        )
    }

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
    let mut prefs: Prefs = toml::from_str(&text).unwrap_or_default();
    if let Err(error) = prefs.preview_filters.compile() {
        eprintln!("Invalid preview filename filters; restoring defaults: {error}");
        prefs.preview_filters = crate::preview_types::PreviewFilters::default();
    }
    // A hand-edited file may hold junk bookmarks: drop them rather than the whole file.
    prefs.bookmarks.retain(Bookmark::is_valid);
    prefs.bookmarks.truncate(MAX_BOOKMARKS);
    prefs
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
            window_geometry: Some(WindowGeometry {
                size: [1200.0, 800.0],
                position: Some([-900.0, 75.0]),
                scale_factor: 2.0,
            }),
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
            window_geometry: Some(WindowGeometry {
                size: [1200.0, 800.0],
                position: Some([100.0, 75.0]),
                scale_factor: 1.0,
            }),
            last_dir: Some("/tmp".to_string()),
            retarget_links: true,
            recursive_preview: false,
            profiles: Vec::new(),
            bookmarks: vec![Bookmark {
                name: "Photos".into(),
                uri: "/home/u/Photos".into(),
            }],
            ..Prefs::default()
        };
        let text = toml::to_string(&prefs).expect("serialize");
        let parsed: toml::Table = toml::from_str(&text).expect("parse");
        let mut keys: Vec<String> = parsed.keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "bookmarks",
                "check_updates",
                "confirm_delete",
                "confirm_run",
                "date_format",
                "density",
                "font_size",
                "git_enabled",
                "git_gutter",
                "git_tint",
                "last_dir",
                "log_poll_ms",
                "log_tail_mib",
                "preview_filters",
                "preview_time_secs",
                "profiles",
                "recursive_preview",
                "remote_keepalive_secs",
                "remote_poll_secs",
                "remote_timeout_secs",
                "retarget_links",
                "show_hidden",
                "single_click_open",
                "size_units",
                "smb_require_encryption",
                "terminal_font_size",
                "terminal_scrollback_rows",
                "terminal_shell",
                "theme",
                "thumb_disk_mib",
                "thumb_memory_mib",
                "window_geometry",
                "window_maximized",
                "zoom"
            ]
        );
        let geometry = parsed
            .get("window_geometry")
            .and_then(toml::Value::as_table)
            .expect("geometry table");
        let mut geometry_keys: Vec<_> = geometry.keys().map(String::as_str).collect();
        geometry_keys.sort();
        assert_eq!(geometry_keys, ["position", "scale_factor", "size"]);
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
    fn junk_bookmarks_are_dropped_on_load_and_the_rest_kept() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join(PREFS_FILE);
        let long = "n".repeat(MAX_BOOKMARK_NAME + 1);
        let mut text = String::from("theme = \"dark\"\n");
        for (name, uri) in [
            ("Photos", "/home/u/Photos"),
            ("", "/x"),
            ("bad\\u0007", "/x"),
            (long.as_str(), "/x"),
            ("NoUri", "not a uri at all"),
        ] {
            text.push_str(&format!(
                "[[bookmarks]]\nname = \"{name}\"\nuri = \"{uri}\"\n"
            ));
        }
        for i in 0..MAX_BOOKMARKS + 5 {
            text.push_str(&format!(
                "[[bookmarks]]\nname = \"b{i}\"\nuri = \"/b/{i}\"\n"
            ));
        }
        std::fs::write(&path, text).expect("write");
        let prefs = load(&path);
        assert_eq!(prefs.theme, ThemePreference::Dark);
        assert_eq!(prefs.bookmarks.len(), MAX_BOOKMARKS);
        assert_eq!(prefs.bookmarks[0].name, "Photos");
        assert!(prefs.bookmarks.iter().all(Bookmark::is_valid));
    }

    #[test]
    fn window_geometry_rejects_nonfinite_and_absurd_values() {
        let valid = WindowGeometry {
            size: [1200.0, 800.0],
            position: Some([-900.0, 75.0]),
            scale_factor: 2.0,
        };
        assert!(valid.is_valid());
        for value in [f32::NAN, f32::INFINITY, -1.0, 0.0, 32769.0] {
            assert!(
                !WindowGeometry {
                    size: [value, 800.0],
                    ..valid
                }
                .is_valid()
            );
        }
        assert!(
            !WindowGeometry {
                position: Some([f32::INFINITY, 0.0]),
                ..valid
            }
            .is_valid()
        );
        assert!(
            !WindowGeometry {
                scale_factor: 0.0,
                ..valid
            }
            .is_valid()
        );
        assert!(
            WindowGeometry {
                position: None,
                ..valid
            }
            .is_valid()
        );
    }

    #[test]
    fn update_checks_default_on_and_opt_out_round_trips() {
        let old: Prefs = toml::from_str("").expect("old preferences");
        assert!(old.check_updates);
        let prefs = Prefs {
            check_updates: false,
            ..Prefs::default()
        };
        let encoded = toml::to_string(&prefs).expect("encode");
        let decoded: Prefs = toml::from_str(&encoded).expect("decode");
        assert!(!decoded.check_updates);
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
