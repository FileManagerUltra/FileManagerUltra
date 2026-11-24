## Context

Every location in the app is a `VfsUri` served by a `Vfs` backend registered by
`(scheme, authority)` in `engine::backends::VfsRegistry`. Panes, the tree, file jobs,
Find, previews and sync only talk to the `Vfs` trait (`read_dir`, `stat`, `open_read`,
`open_read_at`, `open_write`, `create_dir`, `remove`, `rename`, …), and work runs on
per-scheme job lanes. An archive backend that implements `Vfs` therefore gets most of
the UI for free; what is new is that its data comes from *another* location (a file on
any backend, possibly inside another archive) and that writes must be staged.

Constraints (AGENTS.md): pure Rust, no `unsafe`, no FFI; the UI thread never blocks;
nothing below L4 names egui; every action is a command; hostile input everywhere;
secrets never on disk or in `Display`; same behaviour on local, SMB and SFTP.

User decisions (2026-11-23): Enter opens archives like folders; no Places section, but
archives and their contents get a distinct colour; staged editing with Save/Discard;
RAR read through an installed 7-Zip or unrar; file-system images and Docker are
separate changes (`add-disk-images`, `add-docker`).

## Goals / Non-Goals

**Goals:**
- Browse zip, tar (plain and gz/bz2/xz/zst), 7z, ISO, single-file compressed files and
  RAR as folders, on every backend, nested to a bounded depth.
- Edit writable archives (zip, tar family, 7z) with staged changes and an explicit,
  atomic, conflict-checked Save.
- Extract and compress as ordinary file jobs with progress, cancel and conflicts.
- Stay fast on big archives: listing a 100 000-entry zip opens in about a second
  locally; opening a big compressed tar shows progress and never blocks a frame.

**Non-Goals:**
- File-system images and partitions (`add-disk-images`), Docker (`add-docker`).
- Writing RAR or ISO, editing zip spanned sets and RAR volumes, self-extracting executables,
  solid-block-level editing of 7z (a 7z save always recompresses).
- Mounting archives into the OS (FUSE, Windows ProjFS).

## Decisions

### D1. Locations: `archive://<handle>/<inner path>`

A new `Scheme::Archive`. The authority is a *handle*: the first 16 hex digits of a
SHA-256 over the canonical container URI string (for example the handle of
`sftp://me@host/backups/site.tar.gz`). The engine keeps a **mount table**
`handle → container VfsUri` and registers one archive backend per mounted container
under `(Scheme::Archive, handle)`, the way SFTP connections are registered per host.

- The inner path uses the normal `VfsUri` path rules (normalised, `..` clamped at the
  archive root), so containment comes from the existing parser.
- Nesting is natural: the container of a nested archive is itself an `archive://` URI.
  Depth is capped at 8.
- Persistence: wherever a location is stored (history, tabs, workspaces, bookmarks),
  an `archive://` URI is stored together with its container URI (a small
  `ArchiveRef { handle, container }` beside the URI). On restore the engine re-mounts
  from the container; if the container is gone the pane shows the usual
  "location unavailable" state.
- Rejected: putting the container URI inside the archive URI (`zip:file:///a.zip!/x`,
  like Java's jar URLs). It needs escaping of nested URIs, breaks the per-component
  length caps, and makes `parent()`, `join()` and `display` special cases everywhere.

### D2. Display form

`path-display` gains a rule: the display form of an archive location is the display
form of its container followed by the inner path with the platform separator, e.g.
`/home/me/Backups/site.tar.gz/www/index.html` or
`C:\Users\me\site.zip\www\index.html`. The breadcrumb draws the archive segment and
everything after it with the archive tint. Typed back, the parser walks the path from
the left and, when a component is a file whose name matches an archive pattern,
mounts it and continues inside (bounded by the nesting cap).

### D3. Crate and layering

`crates/archive` (`fm-archive`, L1). It depends on `fm-vfs` and the format crates, and
reads the container through a `Source` trait implemented in the engine over
`Vfs::open_read_at` (random access) plus the container's size and modification time.
It has no egui and does no direct filesystem I/O. Format code lives behind one
internal trait:

```rust
trait Format {
    fn index(src: &dyn Source, opts: &Limits, pw: Option<&Secret>) -> Result<Index, FmError>;
    fn open_member(&self, src: &dyn Source, idx: &Index, id: EntryId) -> Result<Box<dyn Read + Send>, FmError>;
    fn writer(&self) -> Option<&dyn FormatWriter>; // None = read-only
}
```

`ArchiveFs` (in `fm-archive`) implements `Vfs` from an `Index`, the `Source` and an
`Overlay` (D6). The engine owns the mount table and the backend registration.

### D4. Formats and crates

| Format | Read | Write | Crate (default features off) |
|---|---|---|---|
| zip, zip64, AES, ZipCrypto | ✓ | ✓ | `zip` with `deflate` (zlib-rs via flate2), `deflate64`, `bzip2` (pure-Rust `libbz2-rs-sys`), `lzma`, `xz`, `ppmd`, `aes-crypto`, `time`; no Zstandard members (its `zstd-rs` feature still builds `zstd-sys` with a pre-release C-to-Rust port, checked 2026-11-23), which show "compression method not supported: Zstandard" |
| tar (ustar, GNU, PAX) | ✓ | ✓ | `tar` |
| .gz / .bz2 / .xz / .zst (tar or single file) | ✓ | ✓ (zst: see D11) | `flate2`, `bzip2`, `lzma-rust2`, `ruzstd` |
| 7z (LZMA, LZMA2, BZip2, Deflate, PPMd, AES-256) | ✓ | ✓ | `sevenz-rust2` |
| ISO 9660 + Joliet + Rock Ridge | ✓ | — | `hadris-iso` (read features only) |
| RAR 4/5 | ✓ via tool | — | none (D9) |

Every crate is checked with `cargo deny` and `cargo tree -e features` for C build
scripts (`cc`, `*-sys` with native code) in task 1; any crate that pulls native code
with the chosen features is replaced or reported to the user before use.

Recognition is by file name first (case-insensitive patterns, editable under
Preferences › Archives) and confirmed by magic bytes on mount; a mismatch shows an
actionable error rather than garbage.

### D5. Random access, indexes and caching

- **Seekable formats** (zip, 7z, ISO, plain tar) read the container with
  `open_read_at` through a block cache (1 MiB blocks, 64 MiB LRU shared by all mounts),
  so remote containers do not need a full download to list or to read one member.
- **Compressed tar** has no index. Mounting makes one sequential pass that builds the
  index (name, size, mode, mtime, link target, offset in the decompressed stream) and
  shows progress for large files. Reading a member decompresses from the start and
  skips to its offset. After the second member read from the same mount, or when a
  whole-tree operation starts (copy out, Find in contents, Save), the decompressed tar
  is spooled once to an owner-only temp file (same private folder as opened remote
  files) when it fits under the spool cap (default 4 GiB and 80 % of free space), and
  from then on it is a plain tar.
- **7z solid blocks** are decoded per block; one decoded block (≤ 256 MiB) is cached
  per mount so siblings in the same block do not decode again.
- Indexes stay in memory while mounted. At most 16 containers stay mounted; idle mounts
  without unsaved changes unmount after 10 minutes, the least recently used first.

### D6. Staged editing: the overlay

An `Overlay` per mount records changes against the base index: added or replaced
files (content spooled to the owner-only temp folder), created folders, renames
(including folder renames, applied to the whole subtree), and deletions. `read_dir`,
`stat` and `open_read` see base plus overlay. `open_write`, `create_dir`, `rename` and
`remove` on the archive backend change only the overlay, so the existing file jobs
(copy, move, delete, rename, new folder, F4 edit-and-save) work unchanged.

- Delete inside an archive cannot use the trash; it is staged instead, and Discard is
  its undo. The delete confirmation says so ("removed from the archive when you save").
- Read-only formats (ISO, RAR, single-file compressed) reject writes with
  `FmError::Unsupported("read-only archive")`, and the UI greys out write commands there.
- Overlay size is capped by the spool cap; exceeding it fails the operation with an
  actionable error.

### D7. Save

`archive.save` runs as a file job on the container's bulk lane:

1. Re-stat the container; if size or modification time differ from the mounted
   version, stop with "The archive changed on disk since it was opened" and offer
   Save as copy or Discard.
2. Write a new archive to a temporary name next to the container (`.name.fmtmp-XXXX`)
   via the container's `Vfs`: unchanged zip members are copied raw (no recompression),
   tar members are copied as stored, 7z is recompressed (same method and block size),
   overlay entries are compressed with the archive's dominant method. Encrypted archives
   keep their encryption and password.
3. Verify by re-indexing the temporary file (entry count and sizes).
4. Rename over the container (`Vfs::rename` with replace), then re-mount from the new
   file and clear the overlay. On failure the original is untouched and the temporary
   file is removed.

Progress, cancel and errors use the existing job panel. "Save as copy" writes to a
picked location instead (path picker) and leaves the original mounted.

### D8. Unsaved changes

A mount with a non-empty overlay is *dirty*. The Files pane shows a slim bar above the
listing ("site.zip has 3 unsaved changes · Save · Discard"), the tab and the pane
header show a dot, and a dirty mount never unmounts. Closing a pane or tab whose
location is inside a dirty archive, closing the last view of it, and quitting all ask
(Save, Don't save, Cancel), through the same prompt machinery as PDF Fill & Sign and
workspaces. Navigating away inside the same tab does not ask; the bar comes back when
the user returns.

### D9. RAR through an installed tool

When a RAR is opened, the engine looks for a tool, in this order: `7zz`, `7z`, `7za`
on `PATH`, then (Windows) `%ProgramFiles%\7-Zip\7z.exe`, then `unrar` on `PATH`. The
tool path is shown in Preferences › Archives and can be set explicitly. Without one,
the pane shows "RAR archives need 7-Zip or unrar" with the install hint per OS.

- Listing: `7z l -slt -ba -p- -- <file>` or `unrar lt -p- -- <file>`, parsed with caps.
- Reading a member: `7z e -so -p<pw> -- <file> <member>` (stdout streamed) or
  `unrar p -inul -p<pw> -- <file> <member>`.
- Always an argument array, never a shell; the archive path and member name are single
  arguments after `--`; the child gets no inherited handles beyond the pipes, a
  timeout and a size cap on stdout. Passwords are passed with `-p` because neither
  tool reads them from a pipe; this is noted in the security spec (visible to the same
  user's process list for the life of the child).
- Remote RAR files are first copied to the private temp folder (≤ 4 GiB).

### D10. Encrypted archives: reading

| Format | Encryption read | Names hidden too |
|---|---|---|
| zip | WinZip AES-128/192/256 (AE-1, AE-2), traditional ZipCrypto | no (zip never hides names) |
| 7z | AES-256 + SHA-256 key derivation | yes, when headers are encrypted |
| RAR 4/5 | through the tool (`-p`) | yes, when headers are encrypted (`-hp`) |

- Archives whose names are hidden (7z encrypted headers, RAR `-hp`) ask for the
  password when they are entered; others ask when an encrypted member is first read
  (preview, copy out, Find in contents). The listing marks encrypted entries with a
  lock badge, and the archive preview says which method is used.
- One password per mount is tried first for every member; a zip whose members use
  different passwords asks again for a member the known password does not open, and
  keeps up to 8 passwords for that mount.
- Wrong passwords are detected by the format's check value (zip AES verifier, ZipCrypto
  check byte plus CRC on read, 7z/RAR header or CRC failure) and show "Wrong
  password"; three failures in a row stop that read. ZipCrypto's one-byte check can
  pass a wrong password, so a CRC failure after a ZipCrypto read is reported as
  "Wrong password or damaged file".
- Passwords are `Secret` values (zeroised on drop), held by the mount, cleared on
  unmount, quit and "Forget password" (`archive.password.forget`), and never stored,
  logged, put in notices, `Display`, workspaces or the diagnostic log.
- Secrets are not handed to other threads beyond the mount's own workers; the RAR tool
  gets the password as its `-p` argument for the life of that child (D9).

### D11a. Adding a selection

All add paths take the pane's whole selection (`focused_selection`, or explicit
`sources` parsed by `parse_sources`, the same as F5 and the other item commands: Shift ranges, Ctrl picks, Select all, the quick-filter result) and run as **one**
job with one progress bar, one conflict policy and one summary, never one job per item.

- **Layout inside the archive**: each selected item is stored under its own name at
  the archive root (or the target folder inside it), folders recursively, so selecting
  `docs/` and `notes.txt` in `/home/me/project` gives `docs/…` and `notes.txt`, not
  `home/me/project/…`. Empty folders are stored. A selection from a Find Files result
  (items from several folders) keeps paths relative to the search root.
- **Links**: tar and zip store symlinks as links; 7z stores them as links when
  `sevenz-rust2` supports it (spike 1.3), otherwise they are skipped and listed in the
  summary. Links are never followed while adding, so a link to `/` cannot pull in the
  whole disk.
- **Default name** for a new archive: the single item's name, or the pane folder's name
  for several items; the dialog shows how many files and folders and the total size.
- **New archive**: `archive.compress` (dialog) or `archive.compress_quick { sources,
  format: "zip" | "7z" }` ("Add to *name*.zip", no dialog, normal level, no password).
- **Existing archive from outside**: `archive.add { sources, archive, dir?, ...}` —
  chosen with "Add to archive…" (path picker limited to writable archives) or by
  dropping the selection on an archive file in a listing or the tree. It mounts the
  archive (asking for its password if encrypted; new members are encrypted with it),
  stages the selection with the usual conflict prompt for names that already exist,
  and saves in the same job (D7). If that archive already has staged changes from a
  pane, the selection is staged on top and not saved, and the unsaved bar shows it.
- **Into an archive a pane is showing**: F5/F6, paste or drag a selection onto that
  pane; this is the staged path (D6) with the normal multi-item copy pipeline.
- Read-only archives (ISO, RAR, spanned zip, single-file compressed) are not offered
  as targets and refuse drops with the reason.

### D11. Compress to… and encrypting

`archive.compress { sources, target, format, level, password?, encryption?,
encrypt_names?, split? }` creates a new archive as a file job: zip (Deflate), tar.gz,
tar.xz, 7z (LZMA2) and tar.zst. `ruzstd`'s encoder is young; task 1.4 checks its
output against the reference decoder and its speed. If it is not good enough, tar.zst
stays read-only in this change and the Compress dialog does not offer it.

Encryption when a password is given:

- zip: WinZip AES-256 (AE-2) by default; `encryption: "zipcrypto"` only on request,
  with the dialog marking it "weak, for old tools only".
- 7z: AES-256; `encrypt_names: true` (default on) also encrypts the headers.
- tar family: no encryption exists in the format; the password field is disabled with
  the tooltip "tar has no encryption; choose 7z or zip".

The dialog asks for the password twice and shows whether they match; it is never
pre-filled or remembered.

Encrypting, re-encrypting or decrypting an existing zip or 7z is a staged change
(`archive.password.set { uri, password?, encryption?, encrypt_names? }`, no password =
remove) applied on Save: every member is re-encrypted (raw copy is not possible for
members whose encryption changes, so Save decompresses and recompresses them).
The unsaved bar says "password will change on save".

### D16. Split archives (multiple volumes)

Three kinds of volume sets are recognised from the name of any part:

| Kind | Names | Structure | In this change |
|---|---|---|---|
| Numbered parts | `name.ext.001`, `.002`, … (any archive format; 7-Zip's and HJSplit's convention) | the archive's bytes cut at fixed sizes | read and write |
| Zip spanned set | `name.z01`, `name.z02`, …, `name.zip` (last) | zip with per-disk offsets in the central directory | read-only |
| RAR volumes | `name.part1.rar`, `name.part2.rar`, … or `name.rar`, `name.r00`, `name.r01`, … | RAR's own volume format | read-only, through the tool |

- **Finding the set.** Entering any part resolves the set in that folder (same backend)
  by name pattern only, never by content: parts are listed with one `read_dir`, sorted
  by number, checked for gaps, and capped at 10 000 parts. A missing part fails the
  mount with "Part 4 of backup.7z is missing (backup.7z.004)". The mount's container is
  the first part; `ArchiveRef` records the kind so locations restore after a restart.
- **Numbered parts** are read through a `ConcatSource` over the parts (offset → part,
  random access through each part's `open_read_at`), so the format reader sees one
  file. This also covers split disk images (`add-disk-images`).
- **Zip spanned sets** need the central directory's (disk number, offset) pairs mapped to
  a global offset. Spike 1.5 checks whether the `zip` crate reads them through a
  `ConcatSource`; if not, our index reader translates disk-relative offsets itself
  before handing members to `zip`'s decompressors, and if that is not feasible, spanned
  zips open through the 7-Zip tool like RAR (read-only), with the same "needs 7-Zip"
  state.
- **RAR volumes**: the tool is given the first volume and finds the rest itself; we
  still resolve the set first so a missing part is reported before the tool runs.
  Remote RAR sets are copied to the private temp folder as a whole set (≤ 4 GiB total).
- **Listing.** Every part is tinted as an archive in the folder listing; parts after
  the first also carry a "part n of N" hint. Entering any part shows the same mount.
- **Changed-on-disk check** covers every part (size and mtime).
- **Saving numbered parts** writes the new archive as temporary parts of the original
  part size next to the originals, verifies the joined result by re-indexing, then
  swaps: each original part is renamed to a backup name, the new parts are renamed into
  place, and the backups are removed. If any rename fails, the swap is rolled back from
  the backups; a crash in the middle leaves the backups beside the parts, which the next
  mount detects and offers to restore. The number of parts may change; surplus old parts
  are removed only after the swap succeeded.
- **Compress to… split**: `split: { size }` (presets 100 MB, 700 MB, 4 GB FAT32 limit,
  custom ≥ 64 KiB) writes numbered parts `name.ext.001…` for every format.
- **Join parts** (`archive.join { uri, target? }`) concatenates numbered parts into one
  file next to them (`backup.7z`), as a file job; it does not apply to zip spanned sets
  or RAR volumes, which can be converted by extracting and compressing.

### D12. Extraction

Extract here / Extract to… / copy out are the existing copy pipeline reading from the
archive backend, so conflicts, progress and cancel are unchanged. Extraction never
writes outside the destination: names were normalised at index time, symlinks
pointing outside the archive root are extracted as broken links only when
`ops` allows (existing "Symlinks cannot be used to escape a destination" rule),
hard links outside the archive are skipped, device files and FIFOs are skipped,
setuid/setgid bits are cleared, and Windows-reserved names are renamed per the
existing rules.

### D13. Hostile-archive limits

- ≤ 1 000 000 entries per archive, ≤ 4 096 bytes per name, ≤ 64 path depth, ≤ 10 000
  parts per split archive; beyond that, mounting stops with an actionable error.
- Names: backslashes, absolute paths, drive letters and `..` are normalised at index
  time; control characters are shown escaped; duplicate paths keep the last entry
  and the archive summary says how many were shadowed.
- Bombs: a member whose decompressed output exceeds its declared size stops with an
  error; extraction of more than 1 GiB at a ratio above 1000:1 asks first; reads stop
  at the spool cap.
- Every decoder runs inside `catch_unwind`; a panic becomes an `FmError` on that mount.
- Nothing inside an archive is executed: `file.open` in `run` mode is refused for
  archive locations, and opening a member with its default app extracts it to the
  private temp folder as a non-executable file, like remote files.

### D14. Look

Theme tokens `archive` and `archive_muted` (both palettes, contrast-checked against
`bg` and the selection colour). The tree, listing and breadcrumb use them for archive
files and for every entry inside an archive; archive files get a small badge on their
icon. Colour is never the only signal: the badge and the breadcrumb's archive icon
carry it too (accessibility).

### D15. Lanes

A new `JobLane::Archive` (2 workers) serves listings and member reads for all
archive mounts, plus `BulkArchive` for saves and extraction, so a slow 7z decode does
not hold up local listings. The container's own I/O still happens through its backend,
which is safe because those backends allow use from other threads (SMB's blocking
session already allows bounded use outside its lane, see AGENTS.md §7; archive reads
are bounded the same way).

## Risks / Trade-offs

- [Compressed tar random access is slow on huge files] → index once, spool after the
  second read, and show progress; documented in the summary card.
- [`sevenz-rust2` and `hadris-iso` are younger than `zip`/`tar`] → fuzz them through
  our limits in tests, wrap in `catch_unwind`, pin exact versions.
- [Saving a large tar.xz or 7z takes long] → it is a cancellable job with progress, and
  the original stays intact until the final rename.
- [Remote save needs a remote temp file and rename] → uses the backend's normal write
  and rename; SMB and SFTP both support replace-rename, covered by tests on the
  in-memory `Vfs` and the Docker SMB tests.
- [The RAR helper exposes the password to the process list] → only with an encrypted
  RAR, only for the child's lifetime, stated in the spec and the prompt.
- [Binary size] → measured in task 1 like the PDF change did; expected a few MB.

## Migration Plan

New scheme and settings only; existing workspaces load unchanged. The Enter behaviour
change is visible: the changelog explains it and the preference restores the old one.

## Open Questions

- Should Enter on an archive in the *tree* only expand it (as now for folders), or also
  navigate the listing? Default: same as folders.
- Do we want "Open with 7-Zip" as a fallback for other formats 7-Zip reads (CAB, WIM,
  DMG, ARJ, LZH)? Not in this change; easy to add once the RAR path exists.

## Implementation notes

- 2026-11-23: zip is built without its `zstd-rs` feature (it still compiles `zstd-sys`
  with a pre-release C-to-Rust port); Zstandard zip members report "compression method
  not supported".
- Mounting is lazy: `archive.enter` registers a `LazyArchive` backend for the handle and
  the first directory read (on the archive lane) opens the archive. Handles and their
  archive files live in a process-wide table (`archive_cmds::REFS`) that `display_path`
  and re-registration use; persisting it across restarts is still open (task 4.1).
- The archive lanes have one thread each, like every other lane (D15 said two).
- The container cache is 32 MiB per mount rather than one shared 64 MiB (D5).
- Extract/compress (part 2): extraction is `fm_ops::copy` from the archive root into a
  new folder (`archive_ops::extract`), so conflicts, progress, cancel and undo (the whole
  new folder) are the copy pipeline's; devices/FIFOs are skipped and counted, no modes are
  carried (so no setuid/setgid), links stay links. Hostile-fixture tests (6.2) and the
  bomb prompt (6.3) landed 2026-11-24: extraction plans the copy first and compares the
  declared total with the archive file's size; above 1 GiB at > 1000:1 the job parks on a
  yes/no question (`job.answer`, same parking as conflicts). Left-out entries are listed
  per item (`OpReport::left_out`), nested ones too. Windows-unsafe names are renamed for
  local Windows destinations in the shared copy planner, and a full destination stops any
  copy job (`FmError::NoSpace`). Compression writes to a temp file (next to a local
  target, else in a private 0700 temp folder), then renames, uploads or cuts it into
  `.001…` parts; names are numbered `name (2).zip` when taken. ZipCrypto writing is
  refused (the zip crate keeps it crate-private) and tar.zst is read-only (ruzstd has
  only its fastest level). 7z stores no symlinks; they are left out and counted. The
  entries are in the item context menu only (no File menu yet). Split output does not
  re-open as one archive until split reading (3.9) lands; `archive.join` and `archive.add`
  are still open.
- Tree: an archive file's row *is* its archive root (`archive_cmds::tree_root`, a pure
  handle computation that only remembers the handle), so expand, select, open and the
  location highlight reuse the folder paths; `submit_read` registers unmounted archive
  locations, and `reveal_in_trees` (in `poll`) expands the tree down into an archive
  through `PaneState::reveal_location_via(tree_parent)`, since fm-layout cannot see
  archive containers. Archive rows are tinted, drag their file, and are no drop target.
  By default only archives opened this session (`Mounts::opened`, filled by
  `reveal_in_trees`, capped at 256) and those around the pane's location are tree rows;
  they stay after leaving until `archive.unmount` (tree row "Close Archive") closes them,
  which unmounts them and archives inside them, folds them and moves panes out; `prefs.archives_in_tree` (Preferences, off by default) shows every
  archive file and keeps left archives expanded (user request 2026-11-23).
- The `archive` token is derived from the accent (user requests 2026-11-23): dark themes
  take `accent_text` 35% toward white; light themes take the accent's hue at full
  saturation in the readable shade furthest from the text (darkening drifted to the
  near-black text: ΔE 1.75:1 contrast for #5cb2ff). Candidates must be 4.5:1 on rows,
  CIE76 ΔE ≥ 30 from the text and ≥ 12 (then 8) from `accent_text`; grey accents take the
  readable candidate furthest from the text. `archive_muted` from D14 is not used.
- Split archives (2026-11-24): numbered parts and zip spanned sets mount through
  `LazyArchive::join_parts`; the mount's container is canonicalised by name (first part,
  or the `.zip`), so entering any part reuses one mount and display shows the first part.
  A spanned zip is detected by its end record's disk number, not by sibling names, so a
  plain zip costs no folder listing. Spanned zips (D16 spike) are read by rewriting the
  central directory in memory (≤ 256 MiB). The listing shows "part n" (not "of N") in the
  Type column. Closing an archive now also drops its cached listings.
- Restoring after a restart (2026-11-24): the handle table is saved as `archives.json`
  rather than an `ArchiveRef` beside every saved URI; one table covers workspaces,
  history, tabs and bookmarks. Loading checks each handle against its container's hash
  and that the container is normalised (the derived `Deserialize` bypasses `parse`).
  Idle archives are released (memory only; passwords stay) after 10 minutes, and at most
  16 stay opened.
