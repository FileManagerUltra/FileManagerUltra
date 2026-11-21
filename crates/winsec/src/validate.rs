//! Safe-code validation and conversion in front of the Win32 calls.

use std::path::Path;

use crate::SecError;

/// Largest descriptor accepted or returned (matches `fm_vfs::MAX_ACL_BYTES`).
pub const MAX_SD_BYTES: usize = 64 * 1024;

/// Windows' extended path limit in UTF-16 units.
const MAX_PATH_UNITS: usize = 32_767;

/// `SECURITY_DESCRIPTOR_RELATIVE` header: revision, sbz1, control (u16), four u32 offsets.
const HEADER: usize = 20;
const REVISION: u8 = 1;
const SE_SELF_RELATIVE: u16 = 0x8000;

/// Cheap structural checks so Win32 never sees an obviously hostile blob: size bounds,
/// revision, the self-relative flag, and every offset pointing inside the blob.
pub fn validate_descriptor(sd: &[u8]) -> Result<(), SecError> {
    if sd.len() < HEADER {
        return Err(SecError::Invalid("too short"));
    }
    if sd.len() > MAX_SD_BYTES {
        return Err(SecError::Invalid("too large"));
    }
    if sd.first().copied() != Some(REVISION) {
        return Err(SecError::Invalid("unknown revision"));
    }
    let word = |at: usize| sd.get(at..at + 2).and_then(|b| b.try_into().ok());
    let dword = |at: usize| sd.get(at..at + 4).and_then(|b| b.try_into().ok());
    let control = u16::from_le_bytes(word(2).ok_or(SecError::Invalid("too short"))?);
    if control & SE_SELF_RELATIVE == 0 {
        return Err(SecError::Invalid("not self-relative"));
    }
    for i in 0..4 {
        let at = 4 + i * 4;
        let off = u32::from_le_bytes(dword(at).ok_or(SecError::Invalid("too short"))?) as usize;
        if off != 0 && (off < HEADER || off >= sd.len()) {
            return Err(SecError::Invalid("offset outside the descriptor"));
        }
    }
    Ok(())
}

/// NUL-terminated UTF-16 form of `path`, rejecting embedded NULs and over-long paths.
pub fn wide_path(path: &Path) -> Result<Vec<u16>, SecError> {
    #[cfg(windows)]
    let mut units: Vec<u16> = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    };
    #[cfg(not(windows))]
    let mut units: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    if units.is_empty() || units.len() > MAX_PATH_UNITS || units.contains(&0) {
        return Err(SecError::Invalid("bad path"));
    }
    units.push(0);
    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(control: u16, offsets: [u32; 4], len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len.max(HEADER)];
        b[0] = REVISION;
        b[2..4].copy_from_slice(&control.to_le_bytes());
        for (i, o) in offsets.iter().enumerate() {
            b[4 + i * 4..8 + i * 4].copy_from_slice(&o.to_le_bytes());
        }
        b
    }

    #[test]
    fn accepts_a_well_formed_header() {
        assert!(validate_descriptor(&header(SE_SELF_RELATIVE, [20, 0, 0, 0], 64)).is_ok());
    }

    #[test]
    fn rejects_short_long_and_malformed_blobs() {
        assert!(validate_descriptor(&[]).is_err());
        assert!(validate_descriptor(&[1; 19]).is_err());
        assert!(validate_descriptor(&vec![1; MAX_SD_BYTES + 1]).is_err());
        let mut bad_rev = header(SE_SELF_RELATIVE, [0; 4], 32);
        bad_rev[0] = 9;
        assert!(validate_descriptor(&bad_rev).is_err());
        assert!(validate_descriptor(&header(0, [0; 4], 32)).is_err());
    }

    #[test]
    fn rejects_offsets_outside_the_blob() {
        assert!(validate_descriptor(&header(SE_SELF_RELATIVE, [5, 0, 0, 0], 64)).is_err());
        assert!(validate_descriptor(&header(SE_SELF_RELATIVE, [0, 64, 0, 0], 64)).is_err());
        assert!(validate_descriptor(&header(SE_SELF_RELATIVE, [0, 0, u32::MAX, 0], 64)).is_err());
    }

    #[test]
    fn paths_are_bounded_and_nul_free() {
        assert!(wide_path(Path::new("")).is_err());
        assert!(wide_path(Path::new("a\0b")).is_err());
        assert!(wide_path(Path::new(&"x".repeat(MAX_PATH_UNITS + 1))).is_err());
        let w = wide_path(Path::new("C:/a")).unwrap();
        assert_eq!(w.last(), Some(&0));
    }
}
