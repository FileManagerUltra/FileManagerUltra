## Context

`add-archive-vfs` introduces `fm-archive`: a `Format` trait (index, open member,
optional writer), `ArchiveFs: Vfs` over an index and a random-access `Source`, the
`archive://<handle>/…` scheme with a mount table, the archive look, read-only formats
and extraction rules. File-system images fit the same model: they are read-only
containers with an index (directory tree) and members (file extents). This change adds
image formats, partitions and content detection on top of it, so nothing in panes,
the tree, jobs or previews needs to know about images.

Constraints as for `add-archive-vfs`: pure Rust, no `unsafe`, no FFI, never block the
UI, hostile input, same behaviour on every backend.

User decisions (2026-11-23): SquashFS, ext2/3/4, FAT, NTFS, JFFS2 and CramFS; content
detection through an explicit "Open as…" command; images look and open like archives.

## Goals / Non-Goals

**Goals:**
- Read the common CTF and firmware file systems without root, on Linux, Windows and
  macOS, from local, SMB, SFTP and nested sources.
- Show partition tables as folders and find file systems at offsets inside blobs.
- Show the metadata that matters for analysis (owner, mode, times, links, devices,
  xattrs) without ever opening special files.

**Non-Goals:**
- Writing images (FAT and SquashFS writing is a later change).
- Virtual-disk containers (VHD, VMDK, qcow2, E01), LVM, RAID, encryption, other file
  systems, carving and deleted-file recovery.

## Decisions

### D1. Formats and crates

| Format | Crate | Notes |
|---|---|---|
| SquashFS 4.0 | `backhand` (features `gzip`, `lz4`, `lzo` only) | xz/lzma and zstd through a custom decompressor using `lzma-rust2` and `ruzstd` (spike 1.2); no `liblzma`/`zstd` C builds |
| ext2/3/4 | `ext4-view` | read-only by design; extents, htree, inline data; journal not replayed |
| FAT12/16/32 + VFAT names | `fatfs` | used read-only here |
| NTFS | `ntfs` | MFT, resident and non-resident data, ADS shown as `name:stream` entries (capped) |
| JFFS2 | own parser | node scan (both endiannesses), zlib, rtime, lzo (via `backhand`'s lzo dep), latest version per inode wins |
| CramFS | own parser | both endiannesses, zlib; the `cramfs` crate is evaluated in 1.3 but very young |
| MBR / GPT | `mbrman`, `gpt` | partitions become folders; protective MBR detected |

Every crate is pinned exactly and checked with `cargo deny` and `cargo tree -e
features` for native builds. If `backhand`'s custom-decompressor hook cannot carry
xz/zstd, the fallback is to read SquashFS with an own parser (the format is
documented; read-only is a few hundred lines); task 1.2 decides.

### D2. Slices: partitions and offsets

A `SliceSource { inner, offset, len }` wraps any `Source`. Partitions and items found
by "Open as…" are mounted from a slice, so every format reader sees an image starting
at 0. `ArchiveRef` gains `offset: u64` and `format: Option<FormatId>` (serde default),
and the handle hashes container + offset + format, so such locations survive history,
tabs and restarts.

### D2a. Split images

Raw images split into numbered parts (`disk.img.001`, `.002`, …, as written by 7-Zip,
FTK Imager's raw split or `split -d`) open as one image through the `ConcatSource` and
name-based set resolution from `add-archive-vfs` (D16 there), from any part, with the
same missing-part error. Split images are read-only like all images here.

### D3. Partitions as a folder

A file with a valid MBR or GPT (and no file system at offset 0 that claims the whole
file) opens as a folder of partitions named `<n> · <type> · <size>` (and the GPT
label when set). Entering a partition probes it for a known file system; unknown ones
show as a single raw file `partition.img` that can be copied out or opened with
"Open as…". Extended/logical MBR partitions are followed with a depth and count cap.

### D4. Recognition

File-name patterns (defaults: `*.squashfs`, `*.sqsh`, `*.sfs`, `*.img`, `*.dd`,
`*.raw`, `*.ext2`, `*.ext3`, `*.ext4`, `*.fat`, `*.vfat`, `*.ntfs`, `*.jffs2`,
`*.cramfs`) are added to Preferences › Archives. Because `.img`, `.bin` and `.raw` are
ambiguous, Enter on them probes the signature at offset 0 (and the partition table)
first and falls back to the normal open action when nothing is found.

### D5. "Open as…" scanner

`archive.scan { uri }` runs a job on the archive lane that reads the file once in
1 MiB windows (overlapping by the longest signature) and matches a fixed table of
signatures, each with a validator that parses enough of the header to report a size
and reject false positives:

- archives: zip local header / end of central directory, gzip, xz, bzip2, zstd, 7z,
  tar (`ustar` at 257 of a 512-byte block), ISO (`CD001` at 32 769);
- file systems: SquashFS (`hsqs`/`sqsh`), ext (`0xEF53` at 1 080 with sane block
  size), FAT boot sector, NTFS (`NTFS    ` at 3), JFFS2 node magic `0x1985` (first
  valid node run), CramFS (`0x28CD3D45`);
- partition tables: MBR (`0x55AA` with sane entries), GPT (`EFI PART` at 512);
- firmware headers: U-Boot uImage (`0x27051956`) and TRX, reported with the offset of
  their payload.

The scan stops at 64 GiB or 10 000 hits, reports progress, can be cancelled, and
suppresses hits inside an already found item's extent unless "show nested hits" is
ticked. The dialog lists offset (hex and decimal), format, size and a short detail
(label, block size, compression); Open mounts that item as a slice. "Open as" with a
format and offset (also typed) skips the scan.

### D6. Metadata

`Attrs` (from `stat_full`) already carries mode, uid/gid, owner/group, times, xattrs
and DOS flags. Image readers fill those; NTFS file attributes map to `dos_flags`.
Device numbers have no field yet: a new `Attrs::rdev: Option<(u32, u32)>` (serde
default) is added and shown in Properties. Xattrs stay capped (count and bytes). Devices, FIFOs and sockets are listed as
`EntryKind::Other` and never opened or extracted (as in `add-archive-vfs` D12).

### D7. Bounds

On top of the archive limits: ext directory blocks and extent trees depth-capped;
NTFS MFT record count capped at 10 million and attribute lists depth-capped; JFFS2
node count capped at 5 million and per-inode fragment count capped; FAT cluster
chains checked for loops; SquashFS fragment and inode tables bounded by the image
size. Every reader runs inside `catch_unwind` (from `add-archive-vfs`).

## Risks / Trade-offs

- [Own JFFS2/CramFS parsers are new code on hostile input] → fuzz corpora from real
  firmware dumps (generated with `mkfs.jffs2`/`mkcramfs` in a test-fixture script,
  committed as small fixtures) plus bit-flip fuzzing in CI.
- [`ntfs` crate last released 2023] → read-only use, pinned, fuzzed; acceptable.
- [`.img`/`.bin` probing on Enter may surprise] → only when a signature matches at 0;
  otherwise the old behaviour; the context menu always has "Open with default app".
- [Scanning a large remote file reads it all] → progress, cancel, and a size question
  above 4 GiB for remote files.

## Migration Plan

Additive. New prefs default on; `ArchiveRef` fields have serde defaults.

## Open Questions

- Should the scanner also offer "Extract all found items" (binwalk `-e` style)? Not in
  this change unless asked.
