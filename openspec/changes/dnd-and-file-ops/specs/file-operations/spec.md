## ADDED Requirements

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
`ops` SHALL refuse to copy or move a directory into itself or any descendant, SHALL treat copying an item onto its own uri as a silent no-op, and SHALL never follow symlinks: because the `Vfs` trait cannot create links in v1, a symlink in the source is reported as a skipped item (`Unsupported`) and its target is never read.

#### Scenario: Into itself
- **WHEN** `/a` is dropped onto `/a/b`
- **THEN** the operation is rejected during planning and the tree is unchanged

#### Scenario: Symlink escape
- **WHEN** a source tree contains a symlink pointing outside it
- **THEN** the link is reported as a failed/skipped item and its target is not read or copied
