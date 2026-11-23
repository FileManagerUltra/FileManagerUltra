## 1. Dependencies and spikes

- [ ] 1.1 Add `ext4-view`, `fatfs`, `ntfs`, `gpt`, `mbrman` and `backhand` (default features off: `gzip`, `lz4`, `lzo`) as exact-pinned deps; `cargo tree -e features` shows no native builds; `cargo deny check` green; binary size before/after in the devlog
- [ ] 1.2 Spike: `backhand` custom decompressor carrying xz/lzma (`lzma-rust2`) and zstd (`ruzstd`); if not possible, decide on an own read-only SquashFS 4.0 reader and note it in `design.md`
- [ ] 1.3 Spike: evaluate the `jffs2` and `cramfs` crates against D1 (dependencies, quality); record the decision (own parsers by default)
- [ ] 1.4 Fixture script (test-only, run manually, outputs committed): small images made with `mksquashfs` (each compressor), `mkfs.ext2/3/4`, `mkfs.fat` (12/16/32), `mkntfs`, `mkfs.jffs2` (both endiannesses), `mkcramfs`, MBR and GPT disk images, a firmware blob with uImage + SquashFS at an offset

## 2. Readers (`fm-archive`)

- [ ] 2.1 `SliceSource` and `ArchiveRef { offset, format }` (serde default) with handle over container + offset + format; round-trip tests
- [ ] 2.2 SquashFS reader (all compressors, xattrs, devices, hard links)
- [ ] 2.3 ext2/3/4 reader via `ext4-view` (owner, mode, times, xattrs, symlinks, devices)
- [ ] 2.4 FAT reader via `fatfs` (read-only, VFAT names, cluster-loop detection)
- [ ] 2.5 NTFS reader via `ntfs` (resident/non-resident data, ADS as capped `name:stream` entries, MFT caps)
- [ ] 2.6 JFFS2 parser (node scan, both endiannesses, zlib/rtime/lzo, latest version wins, caps)
- [ ] 2.7 CramFS parser (both endiannesses, zlib)
- [ ] 2.8 Partition tables: MBR with extended partitions, GPT with labels; partitions as folder entries; raw `partition.img` fallback
- [ ] 2.9 Split raw images (`*.img.001`, `*.dd.001`, `*.raw.001`) through `ConcatSource`; fixture and missing-part test
- [ ] 2.10 Metadata into `Entry`/`Attrs` (mode, uid/gid, times, capped xattrs, NTFS attributes as `dos_flags`; new `Attrs::rdev` for device numbers)
- [ ] 2.11 Fuzz tests for every reader and the scanner (truncated, bit-flipped, looping structures); bounded memory

## 3. Content detection

- [ ] 3.1 Signature table with header validators (archives, compressed streams, file systems, partition tables, uImage/TRX) and size estimates; false-positive tests
- [ ] 3.2 Windowed scanner (1 MiB windows with overlap, 64 GiB / 10 000-hit caps, nested-hit suppression, progress, cancel)
- [ ] 3.3 `archive.scan` and `archive.open_as` commands (forced format + offset, hex or decimal); `commands.rs`; bad-param tests
- [ ] 3.4 Probe on Enter for `*.img`, `*.raw`, `*.bin`, `*.dd`; new default patterns in Preferences › Archives

## 4. UI

- [ ] 4.1 "Open as archive or image…" dialog: scan progress, result table (offset hex/dec, format, size, detail), Show nested hits, Open, forced format + offset
- [ ] 4.2 Partition folder rows; owner/group/mode columns; Properties shows device numbers, xattrs and NTFS attributes
- [ ] 4.3 Format-specific error states ("SquashFS 3 is not supported", corrupt MFT, …)
- [ ] 4.4 egui_kittest snapshots (dark and light): Open as… results, partition folder, SquashFS listing with modes; interaction test for opening a hit

## 5. Docs, checks and wrap-up

- [ ] 5.1 `docs/architecture.md` (images, slices, scanner), `docs/roadmap.md`, `THIRD-PARTY-NOTICES.md`
- [ ] 5.2 `CHANGELOG.md` entries under Unreleased
- [ ] 5.3 `cargo test`, clippy `-D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --check`, `cargo deny check`
- [ ] 5.4 Manual smoke on Linux and Windows with real firmware and a CTF disk image, from local and SFTP
- [ ] 5.5 `log/devlog.md` entry
