//! The `text/uri-list` payload (RFC 2483): one `file:///…` URI per line, CRLF separated.

use std::path::{Path, PathBuf};

use crate::DragError;

/// Most files in one drag.
pub const MAX_PATHS: usize = 1000;
/// X requests are limited in size and INCR transfers are not implemented, so the list is capped.
const MAX_BYTES: usize = 64 * 1024;

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    path.to_str().unwrap_or_default().as_bytes()
}

fn encode(path: &Path, out: &mut String) {
    out.push_str("file://");
    for &b in path_bytes(path) {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.push_str("\r\n");
}

/// The uri-list for absolute `paths`. Relative paths, NUL bytes and oversized lists are refused.
pub fn uri_list(paths: &[PathBuf]) -> Result<String, DragError> {
    if paths.is_empty() {
        return Err(DragError::BadPayload("nothing to drag".into()));
    }
    if paths.len() > MAX_PATHS {
        return Err(DragError::BadPayload(format!(
            "at most {MAX_PATHS} items can be dragged out at once"
        )));
    }
    let mut out = String::new();
    for p in paths {
        if !p.is_absolute() || path_bytes(p).contains(&0) {
            return Err(DragError::BadPayload(format!(
                "{} is not an absolute path",
                p.display()
            )));
        }
        encode(p, &mut out);
        if out.len() > MAX_BYTES {
            return Err(DragError::BadPayload("the selection is too large".into()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn encodes_reserved_bytes_and_unicode() {
        let list = uri_list(&[PathBuf::from("/a b/ü#?%.png")]).expect("list");
        assert_eq!(list, "file:///a%20b/%C3%BC%23%3F%25.png\r\n");
    }

    #[cfg(unix)]
    #[test]
    fn joins_with_crlf() {
        let list = uri_list(&[PathBuf::from("/a"), PathBuf::from("/b")]).expect("list");
        assert_eq!(list, "file:///a\r\nfile:///b\r\n");
    }

    #[test]
    fn rejects_hostile_payloads() {
        assert!(uri_list(&[]).is_err());
        assert!(uri_list(&[PathBuf::from("relative")]).is_err());
        assert!(uri_list(&[PathBuf::from("/a\0b")]).is_err());
        let many: Vec<PathBuf> = (0..=MAX_PATHS)
            .map(|i| PathBuf::from(format!("/{i}")))
            .collect();
        assert!(uri_list(&many).is_err());
        let long = vec![PathBuf::from(format!("/{}", "x".repeat(4000))); 20];
        assert!(uri_list(&long).is_err());
    }
}
