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
- Writing RAR or ISO, split/multi-volume archives, self-extracting executables,
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
| zip, zip64, AES, ZipCrypto | ✓ | ✓ | `zip` with `deflate` (zlib-rs via flate2), `deflate64`, `bzip2` (pure-Rust `libbz2-rs-sys`), `lzma`, `xz`, `zstd-rs` (ruzstd), `ppmd`, `aes-crypto`, `time` |
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

### D10. Passwords

Encrypted zip and 7z ask for the password when a member is first read (or, for 7z
with encrypted headers, when mounting). The password lives in memory with the mount
(zeroised on unmount) and is never stored, logged or put in `Display`. A wrong
password shows "Wrong password" and asks again; three failures stop.

### D11. Compress to…

`archive.compress { sources, target, format, level, password? }` creates a new archive
as a file job: zip (Deflate; AES-256 with a password), tar.gz, tar.xz, 7z (LZMA2;
AES-256 with header encryption when a password is given) and tar.zst. `ruzstd`'s
encoder is young; task 1.4 checks its output against the reference decoder and its
speed. If it is not good enough, tar.zst stays read-only in this change and the
Compress dialog does not offer it.

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

- ≤ 1 000 000 entries per archive, ≤ 4 096 bytes per name, ≤ 64 path depth; beyond
  that, mounting stops with an actionable error.
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
