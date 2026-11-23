## Why

File-system images are everyday material in CTFs, firmware analysis and forensics:
a router firmware `.bin` with a SquashFS inside, an `ext4.img` or `disk.dd` with
partitions, a FAT SD-card dump, an NTFS image, an old JFFS2 or CramFS flash dump.
Looking inside today needs root (`mount -o loop`), Linux-only tools or a chain of
`binwalk`, `unsquashfs` and `7z`. Browsing them read-only like folders, on any OS and
without privileges, with the same copy, preview and Find as everywhere else, makes
the file manager useful for that work. Builds on `add-archive-vfs`.

## What Changes

- **Images open like archives.** SquashFS (v4, gzip/xz/lzma/lzo/lz4/zstd), ext2/3/4,
  FAT12/16/32 (with long names), NTFS, JFFS2 and CramFS images open with Enter like an
  archive, use the archive look and work on every backend and nested (an image inside
  a tar.gz, a SquashFS inside a firmware image).
- **Partitions.** A raw disk image with an MBR or GPT partition table opens as a folder
  of partitions (`1 · Linux · 512 MB`, `2 · EFI System · 100 MB`); each partition opens
  with the file system found in it.
- **Open as…** For any file, "Open as archive or image…" scans it for known
  signatures (archives, file systems, partition tables, compressed streams, firmware
  headers) at any offset, like binwalk, and lists what it finds with offset, format and
  size; picking one opens it as a folder. A format can also be forced.
- **Read-only.** Every image format is read-only in this change; extraction and copying
  out work as for archives. Writing to FAT and SquashFS images is a later change.
- **Metadata is visible.** Owner, group, mode, timestamps, symlink targets, device
  numbers and extended attributes (where the format has them) show in Properties and
  the listing columns, because they matter for CTF and firmware work; devices and
  FIFOs show as entries but are never opened.
- **Out of scope.** Writing images, VHD/VHDX/VMDK/qcow2/E01 containers, LVM, RAID,
  encrypted volumes (LUKS, BitLocker), UBIFS/YAFFS2, APFS/HFS+, Btrfs/XFS/ZFS,
  deleted-file recovery and carving.
- Everything is a command (`archive.open_as`, `archive.scan`) with bad-parameter tests.

## Capabilities

### New Capabilities
- `disk-images`: SquashFS, ext2/3/4, FAT, NTFS, JFFS2 and CramFS images and MBR/GPT partitions browsed read-only as folders, with their metadata, on every backend.
- `content-detection`: "Open as archive or image…": signature scan of a file at any offset, the result list, opening a found item at its offset, and forcing a format.

### Modified Capabilities
- `security`: file-system images and our own parsers are untrusted, fuzzed and bounded; scans are capped; special files inside images are never opened.

## Impact

- `fm-archive` (from `add-archive-vfs`): image readers behind the same `Format` trait,
  a slice `Source` (offset + length) for partitions and "Open as…", the signature
  scanner, own read-only JFFS2 and CramFS parsers.
- `fm-vfs`: `ArchiveRef` gains `offset` and `format` so locations found by "Open as…"
  and partitions survive restarts.
- `fm-engine`: `archive.open_as`, `archive.scan` commands; scan job on the archive lane.
- `fm-ui`: "Open as…" dialog (scan progress and results), partition listing, extra
  metadata columns.
- Dependencies (pure Rust, permissive): `backhand` (default features off: `gzip`,
  `lz4`, `lzo`; xz/lzma and zstd through our own pure-Rust decompressors instead of its
  C-backed features), `ext4-view`, `fatfs`, `ntfs`, `gpt`, `mbrman`.
