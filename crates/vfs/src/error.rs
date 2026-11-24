//! `FmError` — the workspace error type.
//!
//! `Display` **redacts credentials**: any string carried by an error (backend messages,
//! invalid URIs, command ids) goes through [`crate::redact::redact`] so a password can
//! never reach a toast, a log or a screenshot (capability `security`).

use std::fmt;

use crate::redact::redact;

/// Every failure the product can report. Never panic — return this instead.
#[derive(Debug)]
pub enum FmError {
    /// Path does not exist (carries the path, redacted for display).
    NotFound(String),
    /// Operating system refused access.
    PermissionDenied(String),
    /// Destination already exists.
    AlreadyExists(String),
    /// A file was where a directory was expected (or vice versa).
    NotADirectory(String),
    /// Refusing to remove a directory that still has children.
    NotEmpty(String),
    /// The user or the system cancelled the operation.
    Cancelled,
    /// The backend or feature cannot do this (e.g. a stub crate).
    Unsupported(&'static str),
    /// A backend reported a failure; `message` is redacted on display.
    Backend {
        scheme: &'static str,
        message: String,
    },
    /// A URI could not be parsed (input redacted on display).
    InvalidUri(String),
    /// Underlying I/O error.
    Io(std::io::Error),
    /// `Session::dispatch` was given an id that is not registered.
    UnknownCommand(String),
    /// Command parameters were malformed or exceeded the caps.
    InvalidParams(String),
    /// Two commands share an id (registry construction error).
    DuplicateCommand(String),
    /// An encrypted archive needs a password before this can be read (carries the
    /// archive's name, redacted for display). Never carries a password.
    PasswordRequired(String),
    /// The password given for an encrypted archive is wrong (carries the archive's name).
    WrongPassword(String),
    /// The destination has no free space left (carries the path being written).
    NoSpace(String),
}

impl FmError {
    /// Backend error with credential-shaped text removed at construction *and* display.
    pub fn backend(scheme: &'static str, message: impl Into<String>) -> Self {
        Self::Backend {
            scheme,
            message: redact(&message.into()),
        }
    }

    /// URI parse failure; the raw input is redacted for display.
    pub fn invalid_uri(input: impl Into<String>) -> Self {
        Self::InvalidUri(redact(&input.into()))
    }

    /// `true` when reading needs a (different) archive password: ask the user.
    pub fn wants_password(&self) -> bool {
        matches!(self, Self::PasswordRequired(_) | Self::WrongPassword(_))
    }

    /// An I/O error while writing `path`; a full disk or quota becomes [`Self::NoSpace`].
    pub fn writing(err: std::io::Error, path: &str) -> Self {
        if is_out_of_space(&err) {
            Self::NoSpace(path.to_string())
        } else {
            Self::Io(err)
        }
    }

    /// `true` when the destination ran out of space: the rest of a job cannot succeed.
    pub fn is_no_space(&self) -> bool {
        match self {
            Self::NoSpace(_) => true,
            Self::Io(err) => is_out_of_space(err),
            _ => false,
        }
    }

    /// `true` when the error is a cancellation (used to end jobs quietly).
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl fmt::Display for FmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "not found: {}", redact(path)),
            Self::PermissionDenied(path) => write!(f, "permission denied: {}", redact(path)),
            Self::AlreadyExists(path) => write!(f, "already exists: {}", redact(path)),
            Self::NotADirectory(path) => write!(f, "not a directory: {}", redact(path)),
            Self::NotEmpty(path) => write!(f, "directory not empty: {}", redact(path)),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Unsupported(what) => write!(f, "unsupported: {what}"),
            Self::Backend { scheme, message } => write!(f, "backend {scheme}: {}", redact(message)),
            Self::InvalidUri(input) => write!(f, "invalid uri: {}", redact(input)),
            Self::Io(err) => write!(f, "i/o error: {err}"),
            Self::UnknownCommand(id) => write!(f, "unknown command: {}", redact(id)),
            Self::InvalidParams(detail) => write!(f, "invalid parameters: {}", redact(detail)),
            Self::DuplicateCommand(id) => write!(f, "duplicate command id: {}", redact(id)),
            Self::PasswordRequired(name) => write!(f, "{} needs a password", redact(name)),
            Self::WrongPassword(name) => write!(f, "wrong password for {}", redact(name)),
            Self::NoSpace(path) => write!(
                f,
                "not enough free space for {}: free some space or choose another folder",
                redact(path)
            ),
        }
    }
}

/// Disk full or quota exceeded (`ENOSPC`/`EDQUOT` on Unix, `ERROR_DISK_FULL` /
/// `ERROR_HANDLE_DISK_FULL` on Windows).
fn is_out_of_space(err: &std::io::Error) -> bool {
    use std::io::ErrorKind as K;
    if matches!(err.kind(), K::StorageFull | K::QuotaExceeded) {
        return true;
    }
    match err.raw_os_error() {
        Some(code) if cfg!(windows) => code == 112 || code == 39,
        Some(code) => code == 28 || code == 122,
        None => false,
    }
}

impl std::error::Error for FmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for FmError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_redacts_password_in_uri_shaped_input() {
        let err = FmError::invalid_uri("sftp://alice:hunter2@example.com/x");
        let shown = err.to_string();
        assert!(!shown.contains("hunter2"), "leaked password: {shown}");
        assert!(shown.contains("alice"), "user should stay visible: {shown}");
    }

    #[test]
    fn display_redacts_backend_message() {
        let err = FmError::backend("smb", "auth failed for bob:s3cret@nas");
        let shown = err.to_string();
        assert!(!shown.contains("s3cret"), "leaked password: {shown}");
    }

    #[test]
    fn io_error_keeps_source() {
        use std::error::Error as _;
        let err = FmError::from(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(err.to_string().contains("i/o error"));
        assert!(err.source().is_some());
    }

    #[test]
    fn a_full_disk_is_an_actionable_no_space_error() {
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let err = FmError::writing(full, "/data/out/big.iso");
        assert!(matches!(err, FmError::NoSpace(_)));
        assert!(err.is_no_space());
        assert!(err.to_string().contains("free some space"), "{err}");
        let other = FmError::writing(std::io::Error::other("boom"), "/x");
        assert!(matches!(other, FmError::Io(_)) && !other.is_no_space());
        let raw = FmError::Io(std::io::Error::from(std::io::ErrorKind::QuotaExceeded));
        assert!(raw.is_no_space());
    }

    #[test]
    fn cancelled_predicate() {
        assert!(FmError::Cancelled.is_cancelled());
        assert!(!FmError::Unsupported("x").is_cancelled());
    }
}
