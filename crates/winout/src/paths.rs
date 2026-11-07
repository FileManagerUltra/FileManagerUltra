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

/// Size of the fixed `DROPFILES` header.
const HEADER: u32 = 20;
/// Largest `CF_HDROP` block built (bytes).
const MAX_BLOB: usize = 16 * 1024 * 1024;

/// The `CF_HDROP` payload: a `DROPFILES` header (offset, point, non-client flag, wide flag)
/// followed by the NUL-terminated paths and a final extra NUL.
pub fn dropfiles(wide: &[Vec<u16>]) -> Result<Vec<u8>, DragError> {
    let units: usize = wide.iter().map(Vec::len).sum::<usize>() + 1;
    if units * 2 + HEADER as usize > MAX_BLOB {
        return Err(DragError::BadPayload("the selection is too large".into()));
    }
    let mut out = Vec::with_capacity(HEADER as usize + units * 2);
    out.extend_from_slice(&HEADER.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    for unit in wide.iter().flatten().chain(std::iter::once(&0)) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(out)
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
    fn dropfiles_has_header_paths_and_double_nul() {
        let blob = dropfiles(&[
            [w("C:\\a"), vec![0]].concat(),
            [w("D:\\b"), vec![0]].concat(),
        ])
        .expect("blob");
        assert_eq!(blob[..4], 20u32.to_le_bytes());
        assert_eq!(blob[16..20], 1u32.to_le_bytes());
        let units: Vec<u16> = blob[20..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        assert_eq!(
            units,
            [w("C:\\a"), vec![0], w("D:\\b"), vec![0, 0]].concat()
        );
    }

    #[test]
    fn dropfiles_refuses_oversized_selections() {
        let huge = vec![vec![1u16; MAX_UNITS]; 300];
        assert!(dropfiles(&huge).is_err());
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
