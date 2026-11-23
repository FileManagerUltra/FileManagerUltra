## ADDED Requirements

### Requirement: File-system images open like archives
SquashFS 4.0 (gzip, xz, lzma, lzo, lz4 and zstd compression), ext2/3/4, FAT12/16/32 with long names, NTFS, JFFS2 and CramFS images SHALL open with Enter like archives, read-only, with the archive look, on every backend and nested inside archives or other images. Write commands SHALL be disabled inside them with a tooltip saying images are read-only. No root or administrator rights SHALL be needed.

#### Scenario: SquashFS from a firmware dump
- **WHEN** the user presses Enter on `rootfs.squashfs`
- **THEN** the pane lists the image's root (`bin`, `etc`, `lib`, …) and files can be previewed and copied out

#### Scenario: ext4 image on Windows
- **WHEN** a user on Windows enters `disk.ext4`
- **THEN** the pane lists its root folders, including `lost+found`

### Requirement: Ambiguous image names are probed
For ambiguous names (`*.img`, `*.raw`, `*.bin`, `*.dd`), Enter SHALL check for a known file-system signature or partition table at offset 0 and SHALL browse inside only when one is found; otherwise it SHALL use the normal open action.

#### Scenario: A .bin without a file system
- **WHEN** the user presses Enter on `firmware.bin` that has no known signature at offset 0
- **THEN** it opens with its default action and the pane does not change

### Requirement: Partition tables open as folders
A disk image with an MBR (including extended partitions) or GPT partition table SHALL open as a folder of partitions named with number, type, size and label. Entering a partition SHALL open the file system found in it; a partition without a known file system SHALL show as one raw file that can be copied out or opened with "Open as…".

#### Scenario: SD-card dump
- **WHEN** the user enters `sdcard.img` with a FAT boot partition and an ext4 root partition
- **THEN** the pane lists `1 · FAT32 · 256 MB` and `2 · Linux · 7.2 GB`, and entering `2` shows the ext4 root

### Requirement: Image metadata is shown
Owner and group (numeric when names are unknown), mode bits, timestamps, symlink targets, device numbers and extended attributes SHALL be shown in Properties for entries of formats that store them, and owner, group and mode SHALL be available as listing columns. Device files, FIFOs and sockets SHALL be listed and never opened or extracted.

#### Scenario: setuid binary in SquashFS
- **WHEN** the user opens Properties on `bin/busybox` inside a SquashFS image
- **THEN** it shows mode `-rwsr-xr-x`, owner `0`, group `0` and the image's timestamp

### Requirement: Disk image commands
Entering images and partitions SHALL use `archive.enter`; extraction and copy out SHALL use the archive extraction rules. Image-specific failures (unsupported SquashFS version, ext feature not supported, corrupt MFT) SHALL show an actionable message naming the format and the problem.

#### Scenario: Unsupported SquashFS 3
- **WHEN** the user enters a SquashFS 3.x image
- **THEN** the pane says "SquashFS 3 is not supported (only 4.0)" and the file can still be copied
