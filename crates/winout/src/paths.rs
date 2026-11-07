//! Validation and UTF-16 conversion of the dragged paths. Safe code, tested on every OS.

use std::path::{Path, PathBuf};

use fm_dragout::{DragError, MAX_PATHS};

/// Longest single path accepted (UTF-16 units), the extended-length limit of Windows.
const MAX_UNITS: usize = 32_767;

const VERBATIM: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
const VERBATIM_UNC: [u16; 8] = [
    b'\\' as u16,
    b'\\' as u16,
    b'?' as u16,
    b'\\' as u16,
    b'U' as u16,
    b'N' as u16,
    b'C' as u16,
    b'\\' as u16,
];

#[cfg(windows)]
fn to_wide(path: &Path) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    Some(path.as_os_str().encode_wide().collect())
}

#[cfg(not(windows))]
fn to_wide(path: &Path) -> Option<Vec<u16>> {
    path.to_str().map(|s| s.encode_utf16().collect())
}

/// The shell does not parse `\\?\` prefixes: `\\?\C:\a` becomes `C:\a`, `\\?\UNC\h\s` `\\h\s`.
fn strip_verbatim(mut wide: Vec<u16>) -> Vec<u16> {
    if wide.starts_with(&VERBATIM_UNC) {
        let mut out = vec![b'\\' as u16, b'\\' as u16];
        out.extend_from_slice(&wide[VERBATIM_UNC.len()..]);
        return out;
    }
    if wide.starts_with(&VERBATIM) {
        wide.drain(..VERBATIM.len());
    }
    wide
}

/// NUL-terminated UTF-16 for every path. Empty, oversized, relative and NUL-containing
/// input is refused before any OS call.
pub fn wide_paths(paths: &[PathBuf]) -> Result<Vec<Vec<u16>>, DragError> {
    if paths.is_empty() {
        return Err(DragError::BadPayload("nothing to drag".into()));
    }
    if paths.len() > MAX_PATHS {
        return Err(DragError::BadPayload(format!(
            "at most {MAX_PATHS} items can be dragged out at once"
        )));
    }
    paths
        .iter()
        .map(|p| {
            if !p.is_absolute() {
                return Err(DragError::BadPayload(format!(
                    "{} is not an absolute path",
                    p.display()
                )));
            }
            let wide = to_wide(p).ok_or_else(|| {
                DragError::BadPayload(format!("{} is not valid text", p.display()))
            })?;
            let mut wide = strip_verbatim(wide);
            if wide.contains(&0) || wide.len() > MAX_UNITS {
                return Err(DragError::BadPayload(format!(
                    "{} cannot be dragged out",
                    p.display()
                )));
            }
            wide.push(0);
            Ok(wide)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn refuses_empty_and_oversized_lists() {
        assert!(wide_paths(&[]).is_err());
        let many = vec![PathBuf::from("/a"); MAX_PATHS + 1];
        assert!(wide_paths(&many).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_relative_nul_and_overlong_paths() {
        assert!(wide_paths(&[PathBuf::from("a/b")]).is_err());
        assert!(wide_paths(&[PathBuf::from("/a\0b")]).is_err());
        let long = PathBuf::from(format!("/{}", "a".repeat(MAX_UNITS)));
        assert!(wide_paths(&[long]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn terminates_every_path_with_nul() {
        let out = wide_paths(&[PathBuf::from("/ü b")]).expect("paths");
        assert_eq!(out, vec![[w("/ü b"), vec![0]].concat()]);
    }

    #[test]
    fn verbatim_prefixes_are_removed() {
        assert_eq!(strip_verbatim(w(r"\\?\C:\a")), w(r"C:\a"));
        assert_eq!(
            strip_verbatim(w(r"\\?\UNC\host\share\a")),
            w(r"\\host\share\a")
        );
        assert_eq!(strip_verbatim(w(r"C:\a")), w(r"C:\a"));
    }
}
