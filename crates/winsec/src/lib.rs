//! Windows owner, group and DACL of local files, as an opaque security descriptor.
//!
//! [`read`] returns the self-relative security descriptor of a path (plus the resolved
//! owner and group names for display); [`write`] applies one to another path. The blob is
//! never interpreted beyond validation: it round-trips between Windows locations only and
//! is never translated to POSIX users or modes. Every `unsafe` Win32 call lives in `sd.rs`;
//! input validation and conversion are safe code in `validate.rs`. Applying a descriptor
//! needs the same privileges Windows would ask for (changing an owner to another account
//! needs `SeRestorePrivilege`/`SeTakeOwnershipPrivilege`); a refusal is reported as
//! [`SecError::PermissionDenied`], never escalated. On other platforms every call is
//! `Unsupported`.

use std::fmt;
use std::path::Path;

mod validate;

#[cfg(windows)]
mod sd;

pub use validate::{MAX_SD_BYTES, validate_descriptor, wide_path};

/// A security descriptor read from a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Security {
    /// Self-relative descriptor (owner, group, DACL, control flags), at most [`MAX_SD_BYTES`].
    pub descriptor: Vec<u8>,
    /// `DOMAIN\name` of the owner, when it resolves.
    pub owner: Option<String>,
    /// `DOMAIN\name` of the primary group, when it resolves.
    pub group: Option<String>,
}

/// Why reading or writing a descriptor failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecError {
    /// Not Windows, or the filesystem has no ACLs (FAT, some network shares).
    Unsupported,
    /// Windows refused: missing privilege, not the owner, read-only volume.
    PermissionDenied,
    NotFound,
    /// The descriptor is over [`MAX_SD_BYTES`] or malformed.
    Invalid(&'static str),
    /// Any other Win32 error code.
    Os(u32),
}

impl fmt::Display for SecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "security descriptors are not supported here"),
            Self::PermissionDenied => write!(f, "permission denied"),
            Self::NotFound => write!(f, "not found"),
            Self::Invalid(why) => write!(f, "invalid security descriptor: {why}"),
            Self::Os(code) => write!(f, "Windows error {code}"),
        }
    }
}

impl std::error::Error for SecError {}

/// Read the owner, group and DACL of `path` (the target of a final symlink is read).
pub fn read(path: &Path) -> Result<Security, SecError> {
    let wide = wide_path(path)?;
    read_impl(&wide)
}

/// Which parts of a descriptor to apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    /// Owner and primary group.
    pub owner: bool,
    /// The DACL (and its inheritance-protection flag).
    pub dacl: bool,
}

impl Parts {
    pub const ALL: Parts = Parts {
        owner: true,
        dacl: true,
    };
}

/// Apply the selected parts of `descriptor` to `path`.
pub fn write(path: &Path, descriptor: &[u8], parts: Parts) -> Result<(), SecError> {
    validate_descriptor(descriptor)?;
    let wide = wide_path(path)?;
    if !parts.owner && !parts.dacl {
        return Ok(());
    }
    write_impl(&wide, descriptor, parts)
}

#[cfg(windows)]
fn read_impl(wide: &[u16]) -> Result<Security, SecError> {
    sd::read(wide)
}

#[cfg(windows)]
fn write_impl(wide: &[u16], descriptor: &[u8], parts: Parts) -> Result<(), SecError> {
    sd::write(wide, descriptor, parts)
}

#[cfg(not(windows))]
fn read_impl(_: &[u16]) -> Result<Security, SecError> {
    Err(SecError::Unsupported)
}

#[cfg(not(windows))]
fn write_impl(_: &[u16], _: &[u8], _: Parts) -> Result<(), SecError> {
    Err(SecError::Unsupported)
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn every_call_is_unsupported_off_windows() {
        let p = Path::new("/tmp/x");
        assert_eq!(read(p), Err(SecError::Unsupported));
    }

    #[test]
    fn a_bad_descriptor_is_rejected_before_any_os_call() {
        let p = Path::new("/tmp/x");
        assert!(matches!(
            write(p, &[], Parts::ALL),
            Err(SecError::Invalid(_))
        ));
        assert!(matches!(
            write(p, &vec![1u8; MAX_SD_BYTES + 1], Parts::ALL),
            Err(SecError::Invalid(_))
        ));
    }
}

/// Windows-only: a real round trip on a temp file (skipped silently on filesystems
/// without ACLs).
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn descriptor_round_trips_between_files() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let sec = match read(&a) {
            Ok(s) => s,
            Err(SecError::Unsupported) => return,
            Err(e) => panic!("read failed: {e}"),
        };
        assert!(sec.descriptor.len() <= MAX_SD_BYTES);
        assert!(sec.owner.is_some());
        write(&b, &sec.descriptor, Parts::ALL).unwrap();
        let again = read(&b).unwrap();
        assert_eq!(again.owner, sec.owner);
        assert_eq!(again.group, sec.group);
    }

    #[test]
    fn a_missing_file_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("nope")), Err(SecError::NotFound));
    }
}
