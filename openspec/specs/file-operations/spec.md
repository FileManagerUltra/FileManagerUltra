# file-operations Specification

## Purpose
TBD - created by archiving change dnd-and-file-ops. Update Purpose after archive.
## Requirements
### Requirement: Copy pipeline
`ops` SHALL copy files and directory trees between any two `Vfs` locations by streaming `open_read`/`open_write` with a bounded buffer, preserving directory structure and reporting progress; it SHALL never hold a whole file in memory.

#### Scenario: Copy a tree
- **WHEN** a directory with nested files is copied to an empty destination
- **THEN** the destination contains an identical tree and the report lists every item as ok

#### Scenario: Copy across backends
- **WHEN** the source is the in-memory testkit backend and the destination is `fs-local` on a tempdir
- **THEN** content and sizes match byte for byte

#### Scenario: Source vanishes mid-copy
- **WHEN** a source file disappears after planning
- **THEN** that item is reported failed with `NotFound`, other items continue, and no partial destination file remains

### Requirement: Move pipeline
Move SHALL use `Vfs::rename` when source and destination share an authority and device; otherwise it SHALL copy, verify the destination (kind and size), and only then delete the source.

#### Scenario: Same-device move is a rename
- **WHEN** a file is moved within one local directory tree
- **THEN** no bytes are copied and the source no longer exists

#### Scenario: Failed cross-device move keeps the source
- **WHEN** the destination write fails midway during a cross-backend move
- **THEN** the source is untouched and the item is reported failed

### Requirement: Mkdir and rename
`ops` SHALL create directories and rename entries in place, refusing names that are empty, contain a separator or NUL, are `.`/`..`, or exceed `MAX_COMPONENT_BYTES`.

#### Scenario: Invalid name rejected
- **WHEN** rename receives the name `a/b` or `..`
- **THEN** it returns `Err(InvalidParams)` and nothing changes

#### Scenario: Existing target
- **WHEN** mkdir or rename targets an existing name
- **THEN** it returns `AlreadyExists` and nothing is overwritten

### Requirement: Planning pass and caps
Every operation SHALL plan first: walk the sources (bounded by `MAX_DIR_ENTRIES` per directory, a depth cap and a total item cap), compute item/byte totals, and run guards before any write.

#### Scenario: Totals known before the first byte
- **WHEN** a copy job starts
- **THEN** its first progress event carries total items and bytes

#### Scenario: Over-cap selection
- **WHEN** the plan would exceed the item cap
- **THEN** the job fails with an actionable error before writing anything

### Requirement: Containment guards
`ops` SHALL refuse to copy or move a directory into itself or any descendant, SHALL treat copying an item onto its own uri as a silent no-op, and SHALL never follow symlinks: a source symlink is recreated as a link (`Vfs::create_symlink`) and its target is never read. A backend that cannot create links reports the item as failed (`Unsupported`).

#### Scenario: Into itself
- **WHEN** `/a` is dropped onto `/a/b`
- **THEN** the operation is rejected during planning and the tree is unchanged

#### Scenario: Symlink escape
- **WHEN** a source tree contains a symlink pointing outside it
- **THEN** the link is recreated as a link with the same target text and its target is not read or copied

### Requirement: Link re-targeting preference
When the `retarget_links` preference is on (default), copying a tree SHALL point **absolute** symlinks whose target lies inside the copied tree at the same place under the copy. Relative links, links to other places, dangling links and targets that would escape through `..` SHALL be copied verbatim. The rewrite is a lexical path-prefix swap (never resolves or follows), only between local volumes, and never yields a target containing `..`. With the preference off every link is copied verbatim. Same-device moves are a rename and keep links untouched.

#### Scenario: Inside link follows the copy
- **WHEN** `/src/p` containing `bin/x -> /src/p/lib/a` is copied to `/dst` with the preference on
- **THEN** `/dst/p/bin/x` points at `/dst/p/lib/a`

#### Scenario: Preference off
- **WHEN** the same copy runs with the preference off
- **THEN** `/dst/p/bin/x` still points at `/src/p/lib/a`

### Requirement: Editable location bar
Clicking a pane's path or running `nav.goto` (Ctrl+L) SHALL turn the path into a text editor with directory completion and a session history. Completion MUST be non-blocking (cached or queued reads only), Tab completes, Up/Down select, Enter navigates via `nav.open`, Esc or focus loss cancels. A failing path SHALL leave the editor open with an error notice.

#### Scenario: Complete and go
- **WHEN** the user presses Ctrl+L, types a prefix, presses Tab and Enter
- **THEN** the pane opens the completed directory and it is added to the history

### Requirement: Operations work across backends
Copy and move SHALL work for local ↔ SFTP and SFTP ↔ SFTP by streaming `open_read`/`open_write` with the usual progress, cancel and conflict handling. A move within one authority SHALL use `Vfs::rename`; a move across backends SHALL copy and then remove the source only after the copy fully succeeded. A failed or cancelled transfer SHALL NOT leave a partial destination file behind.

#### Scenario: Cancel mid-upload
- **WHEN** a local-to-SFTP copy of a large file is cancelled
- **THEN** the partial remote file is removed and the source is untouched

#### Scenario: Cross-backend move failure
- **WHEN** the copy half of a move fails
- **THEN** the source is not removed and the failure is reported

#### Scenario: Same-authority move
- **WHEN** files are moved between two directories on one server
- **THEN** a rename is used and no bytes are transferred

