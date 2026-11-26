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

/// Default cap on temporary copies of archives (remote RAR sets): 4 GiB.
pub const DEFAULT_SPOOL_CAP: u64 = 4 << 30;
/// The range Preferences accepts for the spool cap, in GiB.
pub const SPOOL_CAP_GIB: std::ops::RangeInclusive<u64> = 1..=1024;

static SPOOL_CAP: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_SPOOL_CAP);

/// Bytes a temporary copy of an archive may take (Preferences › Files › Archives).
pub fn spool_cap() -> u64 {
    SPOOL_CAP.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set the spool cap in GiB, clamped to [`SPOOL_CAP_GIB`].
pub fn set_spool_cap_gib(gib: u64) {
    let gib = gib.clamp(*SPOOL_CAP_GIB.start(), *SPOOL_CAP_GIB.end());
    SPOOL_CAP.store(gib << 30, std::sync::atomic::Ordering::Relaxed);
}
