//! Bounds on hostile archives (add-archive-vfs design D13).

/// Entries per archive; mounting a bigger one fails with an actionable error.
pub const MAX_ENTRIES: usize = 1_000_000;
/// Bytes in one entry's full path inside the archive.
pub const MAX_NAME_BYTES: usize = 4096;
/// Folder levels inside one archive.
pub const MAX_DEPTH: usize = 64;
/// Archives inside archives (the ninth level is refused).
pub const MAX_NESTING: usize = 8;
/// Members up to this size are decompressed in one go; bigger ones stream.
pub const INLINE_MEMBER: u64 = 1 << 20;
/// Block size of the container read cache.
pub const BLOCK: u64 = 1 << 20;
/// Blocks kept per mounted container (32 MiB).
pub const CACHED_BLOCKS: usize = 32;
/// A symlink target longer than this is cut (it is shown, never followed).
pub const MAX_LINK_TARGET: usize = 4096;
