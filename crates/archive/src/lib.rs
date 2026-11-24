//! Archives as file systems (change `add-archive-vfs`).
//!
//! Format readers build an [`Index`] from a random-access [`Source`]; [`ArchiveFs`]
//! serves that index through the `Vfs` trait. No egui, no direct filesystem I/O:
//! the container is reached only through a `Source`, which [`source::VfsSource`]
//! implements over any `Vfs`.

pub mod format;
pub mod fs;
pub mod index;
pub mod limits;
pub mod mount;
pub mod secret;
pub mod source;
pub mod time;
pub mod write;

pub use format::{FormatId, Info, Passwords, Progress, Reader, by_name, signature_fits, sniff};
pub use fs::ArchiveFs;
pub use index::{Index, Meta, Node, NodeId, NodeKind};
pub use mount::LazyArchive;
pub use secret::Secret;
pub use source::{ConcatSource, MemSource, SliceSource, Source, SourceReader, VfsSource};

use sha2::{Digest, Sha256};

/// The mount handle of a container: 16 hex digits of SHA-256 over the container's
/// canonical uri, plus the offset and format when they are not the defaults (design D1).
pub fn handle_for(container: &fm_vfs::VfsUri, offset: u64, format: Option<FormatId>) -> String {
    let mut h = Sha256::new();
    h.update(container.to_string().as_bytes());
    if offset != 0 || format.is_some() {
        h.update(b"\0");
        h.update(offset.to_le_bytes());
        h.update(format.map_or("", FormatId::id).as_bytes());
    }
    h.finalize()
        .iter()
        .take(fm_vfs::ARCHIVE_HANDLE_LEN / 2)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod fuzz_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_stable_hex_and_differ_by_location() {
        let a = fm_vfs::VfsUri::parse("/home/me/site.zip").expect("a");
        let b = fm_vfs::VfsUri::parse("sftp://me@host/site.zip").expect("b");
        let ha = handle_for(&a, 0, None);
        assert!(fm_vfs::is_archive_handle(&ha), "{ha}");
        assert_eq!(ha, handle_for(&a, 0, None));
        assert_ne!(ha, handle_for(&b, 0, None));
        assert_ne!(ha, handle_for(&a, 512, Some(FormatId::Tar)));
    }
}
