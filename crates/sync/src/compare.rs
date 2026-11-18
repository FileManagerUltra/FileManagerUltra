//! Deciding whether two files are the same: size and time first, BLAKE3 when asked.

use std::io::Read;
use std::time::{Duration, SystemTime};

use fm_ops::CancelFlag;
use fm_vfs::{FmError, Vfs, VfsUri};

use crate::profile::CompareMethod;
use crate::scan::Node;

pub type Hash = [u8; 32];

const BUF: usize = 256 * 1024;

pub fn times_equal(a: Option<SystemTime>, b: Option<SystemTime>, tolerance: Duration) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let d = match a.duration_since(b) {
                Ok(d) => d,
                Err(e) => e.duration(),
            };
            d <= tolerance
        }
        // Unknown times can't prove equality; content or size decides elsewhere.
        _ => false,
    }
}

/// Stream a file through BLAKE3. Cancellable between buffers; never holds the file in memory.
pub fn hash_file(vfs: &dyn Vfs, uri: &VfsUri, cancel: &CancelFlag) -> Result<Hash, FmError> {
    let mut r = vfs.open_read(uri)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; BUF];
    loop {
        if cancel.is_cancelled() {
            return Err(FmError::Cancelled);
        }
        let n = r.read(&mut buf).map_err(FmError::Io)?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or_default());
    }
    Ok(*h.finalize().as_bytes())
}

/// `hash(side)` is only called when the method needs content; `side` is `true` for left.
pub fn files_equal(
    method: CompareMethod,
    tolerance: Duration,
    left: &Node,
    right: &Node,
    hash: &mut dyn FnMut(bool) -> Result<Hash, FmError>,
) -> Result<bool, FmError> {
    if left.size != right.size {
        return Ok(false);
    }
    let same_time = times_equal(left.mtime, right.mtime, tolerance);
    match method {
        CompareMethod::SizeMtime => Ok(same_time),
        CompareMethod::Checksum if same_time => Ok(true),
        CompareMethod::Checksum | CompareMethod::ChecksumAlways => Ok(hash(true)? == hash(false)?),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use crate::scan::NodeKind;
    use fm_testkit::MemFs;
    use std::time::UNIX_EPOCH;

    fn node(size: u64, secs: u64) -> Node {
        Node {
            kind: NodeKind::File,
            size,
            mtime: Some(UNIX_EPOCH + Duration::from_secs(secs)),
            link: None,
        }
    }

    #[test]
    fn size_and_time_rules() {
        let none = &mut |_| -> Result<Hash, FmError> { Err(FmError::Cancelled) };
        let tol = Duration::from_secs(2);
        let mut eq = |m, a: &Node, b: &Node| files_equal(m, tol, a, b, none).expect("cmp");
        assert!(eq(CompareMethod::SizeMtime, &node(5, 100), &node(5, 101)));
        assert!(!eq(CompareMethod::SizeMtime, &node(5, 100), &node(5, 110)));
        assert!(!eq(CompareMethod::SizeMtime, &node(5, 100), &node(6, 100)));
        // Checksum with matching size and time needs no hashing (the closure would fail).
        assert!(eq(CompareMethod::Checksum, &node(5, 100), &node(5, 100)));
    }

    #[test]
    fn checksum_detects_same_size_different_content_even_with_equal_times() {
        let fs = MemFs::new();
        fs.put_str("/a", b"hello").expect("put");
        fs.put_str("/b", b"jello").expect("put");
        let (a, b) = (
            VfsUri::parse("/a").expect("uri"),
            VfsUri::parse("/b").expect("uri"),
        );
        let c = CancelFlag::new();
        let mut hash = |left: bool| hash_file(&fs, if left { &a } else { &b }, &c);
        let same_time = node(5, 100);
        assert!(
            !files_equal(
                CompareMethod::ChecksumAlways,
                Duration::ZERO,
                &same_time,
                &same_time,
                &mut hash
            )
            .expect("cmp")
        );
        // Different mtime but identical content is equal under Checksum.
        fs.put_str("/b", b"hello").expect("put");
        let mut hash = |left: bool| hash_file(&fs, if left { &a } else { &b }, &c);
        assert!(
            files_equal(
                CompareMethod::Checksum,
                Duration::ZERO,
                &node(5, 1),
                &node(5, 900),
                &mut hash
            )
            .expect("cmp")
        );
    }

    #[test]
    fn hashing_honours_cancel() {
        let fs = MemFs::new();
        fs.put_str("/a", b"hello").expect("put");
        let c = CancelFlag::new();
        c.cancel();
        let r = hash_file(&fs, &VfsUri::parse("/a").expect("uri"), &c);
        assert!(matches!(r, Err(FmError::Cancelled)));
    }
}
