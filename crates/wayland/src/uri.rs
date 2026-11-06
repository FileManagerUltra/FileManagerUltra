//! Parsing a dropped `text/uri-list` (RFC 2483) into local paths. The list comes from another
//! application, so it is capped and every entry is checked.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

/// Largest list read from another application.
pub(crate) const MAX_BYTES: usize = 256 * 1024;
const MAX_PATHS: usize = 10_000;

fn hex(b: u8) -> Option<u8> {
    char::from(b)
        .to_digit(16)
        .and_then(|d| u8::try_from(d).ok())
}

fn decode(raw: &str) -> Option<Vec<u8>> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    Some(out)
}

/// The absolute local paths named by `text`; other schemes, remote hosts and bad entries are
/// skipped.
pub(crate) fn parse(text: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(text);
    let mut paths = Vec::new();
    for line in text.lines().take(MAX_PATHS) {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("file://") else {
            continue;
        };
        let Some(slash) = rest.find('/') else {
            continue;
        };
        let (host, path) = rest.split_at(slash);
        if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
            continue;
        }
        let Some(bytes) = decode(path) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        paths.push(PathBuf::from(OsString::from_vec(bytes)));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_local_files_and_skips_the_rest() {
        let list = b"# comment\r\nfile:///tmp/a%20b.png\r\nfile://localhost/x/y\r\nfile://other/z\r\nhttp://e.com/a\r\nfile:///bad%zz\r\nfile:///nul%00\r\n";
        let got = parse(list);
        assert_eq!(
            got,
            vec![PathBuf::from("/tmp/a b.png"), PathBuf::from("/x/y")]
        );
    }

    #[test]
    fn empty_and_garbage_give_nothing() {
        assert!(parse(b"").is_empty());
        assert!(parse(&[0xff, 0xfe, b'\n']).is_empty());
    }
}
