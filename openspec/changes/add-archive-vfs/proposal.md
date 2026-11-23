## Why

Archives are everyday files: downloads arrive as zip, backups as tar.gz, tools as 7z,
installers as ISO, and CTF challenges as all of them, often nested. Today the app
treats them as opaque files, so looking inside, taking one file out or fixing one file
means leaving the file manager or extracting everything first. Browsing an archive
like a folder, in the tree and the Files pane, with the same copy, move, preview and
Find as everywhere else, removes that detour. Changes inside an archive are staged
and saved deliberately, so an archive is never rewritten by accident.

## What Changes

- **Archives open like folders.** Enter or double-click on a recognised archive browses
  into it. The breadcrumb reads `Backups › site.tar.gz › www`, Backspace and Back leave
  it, and the tree expands archives like folders. A preference keeps the old behaviour
  (Enter opens the archive with its default app); the context menu always offers both.
- **Archives look different.** Archive files, and the folders and files inside an
  archive, are drawn with a distinct theme colour and an archive badge in the tree,
  the listing and the breadcrumb, in dark and light. No new Places section.
- **Formats.** Read and write: zip (Deflate, Deflate64, Bzip2, LZMA, XZ, Zstandard,
  PPMd; zip64; AES and ZipCrypto passwords), tar and tar.gz / .tgz, .tar.bz2, .tar.xz,
  .tar.zst, and 7z (LZMA/LZMA2, AES). Read-only: ISO 9660 with Joliet and Rock Ridge,
  single-file .gz/.bz2/.xz/.zst (shown as a folder holding one file), and RAR through an
  installed 7-Zip or unrar. All decoders are pure Rust; RAR is the only format that
  uses an external program, and only when the user has one installed.
- **Split archives (multiple volumes).** Archives split into volumes open as one
  archive from any of their parts: numbered parts of any format (`backup.7z.001`,
  `site.zip.001`, `dump.tar.gz.001`, …), zip spanned sets (`site.z01`, `site.z02`, …,
  `site.zip`) and RAR volumes (`data.part1.rar`, … or `data.rar`, `data.r00`, …). The
  parts must sit together in one folder (on any backend); a missing part is named in
  the error. Numbered-part archives of writable formats stay editable: Save rewrites
  them as the same number of parts at the same part size. Compress to… can split its
  output into parts of a chosen size, and "Join parts" turns numbered parts back into
  one file.
- **Encrypted archives, reading and creating.** Encrypted zip (AES-128/192/256 and
  legacy ZipCrypto), 7z (AES-256, also with encrypted file names) and RAR (through the
  tool, also with encrypted file names) open after a password prompt. Saving changes
  keeps the archive encrypted with the same password. Compress to… encrypts zip
  (AES-256, or ZipCrypto only when asked for compatibility, marked as weak) and 7z
  (AES-256, optionally with encrypted file names); the password is typed twice. For an
  existing zip or 7z, "Set password…", "Change password…" and "Remove password" are
  staged changes applied on Save. Passwords are kept in memory only while needed.
- **Works on every backend and nests.** An archive on SMB or SFTP, or an archive inside
  an archive (a zip inside a tar.gz), opens the same way. Locations inside archives are
  ordinary `VfsUri`s, so history, tabs, workspaces, bookmarks and the address bar work.
- **Staged editing with Save and Discard.** Copying or moving files in, creating
  folders, renaming, deleting and saving an edited file inside a writable archive
  changes nothing on disk until Save. The pane shows that the archive has unsaved
  changes; Save rewrites the archive to a temporary file and replaces the original
  only if it did not change meanwhile; Discard drops the changes. Closing the pane or
  quitting with unsaved changes asks first, like PDF Fill & Sign.
- **Extract and compress.** Extract here, Extract to… (path picker) and Compress to…
  (zip, tar.gz, tar.xz, tar.zst, 7z; level; optional password for zip and 7z) as file
  jobs with progress, cancel and conflict handling. Copying out of an archive with
  F5 or drag and drop is extraction too.
- **Adding a selection at once.** Everything selected in a pane (Shift-click ranges,
  Ctrl-click picks, Select all, mixed files and folders) goes into an archive in one
  job, with folders added recursively and paths kept relative to the pane's folder:
  into a new archive (Compress to…, or the one-click "Add to *name*.zip" / "Add to
  *name*.7z"), into an existing archive ("Add to archive…", or dropping the selection
  onto an archive file in a listing), or into the archive another pane is showing
  (F5 or drag and drop). Adding to an existing archive from outside saves it in the
  same job; adding inside a pane that is already showing the archive is staged like
  any other edit.
- **Preview.** The Preview pane shows an archive's format, entry count, packed and
  unpacked size, encryption and its top-level entries; files inside an archive preview
  like any other file.
- **Hostile archives are bounded.** Paths are normalised (no `..`, absolute paths,
  drive letters or device names escape), symlinks and hard links never resolve outside
  the archive, entry counts and name lengths are capped, decompression bombs are
  detected, and nothing inside an archive is ever executed.
- **Out of scope.** File-system images (SquashFS, ext, FAT, NTFS, JFFS2, CramFS),
  partition tables and "Open as…" content detection (change `add-disk-images`); Docker
  (change `add-docker`); writing RAR; creating ISO images; editing zip spanned sets
  and RAR volumes (both read-only); self-extracting executables; other formats through
  7-Zip; encryption that wraps a whole file (GPG, age, OpenSSL `enc`), which is not an
  archive format; encrypted disk volumes (LUKS, BitLocker, VeraCrypt).
- Everything is a command (`archive.*`) with bad-parameter tests.

## Capabilities

### New Capabilities
- `archive-browsing`: recognising archives, browsing them as folders on every backend and nested, the archive look, formats and their read/write support, split archives, encrypted archives and passwords, RAR through an installed tool, the archive preview and the bounds on hostile archives.
- `archive-editing`: staged changes inside writable archives (including split archives and setting, changing or removing a password), Save, Discard, conflict detection on save, and the unsaved-changes prompts.
- `archive-extract-compress`: Extract here, Extract to…, Compress to… (with encryption and splitting into parts), adding a multi-selection to a new or existing archive in one job, Join parts, and copying out of archives, as file jobs.

### Modified Capabilities
- `path-display`: locations inside archives have a display form (the archive's display form, then the inner path) that can be typed back.
- `security`: archives are untrusted input (path normalisation, link containment, bomb limits, no execution); archive passwords never persist; the RAR helper runs a fixed program with an argument array.

## Impact

- New crate `crates/archive` (`fm-archive`, L1): format readers and writers over a
  random-access source, the staged overlay and the save writer. Depends on `fm-vfs`;
  no egui, no direct filesystem I/O except through a `Vfs`.
- `fm-vfs`: new `Scheme::Archive`; `VfsUri` parsing and display for archive locations.
- `fm-engine`: archive mount table and backend registration, a new `archive` job lane,
  `archive_cmds.rs`, unsaved-archive tracking in quit and pane-close prompts,
  workspace and history persistence of archive locations.
- `fm-ui`: archive tint and badge in the tree, listing and breadcrumb; Save/Discard
  bar in the Files pane; password prompt; Compress dialog; archive preview card.
- `fm-preview`: archive summary model.
- Dependencies (all pure Rust, licences on the allow-list, checked by `cargo deny`):
  `zip` (already in the tree; default features off, with `zstd-rs` instead of the C
  `zstd`), `tar`, `flate2` (already present), `bzip2` 0.6 (pure-Rust backend),
  `lzma-rust2`, `ruzstd`, `sevenz-rust2`, `hadris-iso`.
- Theme: one new semantic token pair for the archive tint (dark and light).
- Docs: `docs/architecture.md` new section, `THIRD-PARTY-NOTICES.md`, `CHANGELOG.md`.
