# local-backend Specification

## Purpose
TBD - created by archiving change multi-pane-ui. Update Purpose after archive.
## Requirements
### Requirement: The local backend implements the full Vfs trait
`fs-local` SHALL implement `read_dir`, `stat`, `open_read`, `open_write`, `create_dir`, `remove`, `rename`, `same_device` and `space` for `file://` URIs, mapping operating-system failures to the matching `FmError` variants.

#### Scenario: Round-trip on a tempdir
- **WHEN** a test creates a temp tree (nested dirs, files of several sizes) and drives every trait method through `LocalFs`
- **THEN** the resulting tree matches what a direct `std::fs` inspection would show, and every call returns `Ok`

#### Scenario: Errors are typed, not strings
- **WHEN** `stat`/`read_dir`/`remove`/`rename`/`open_write` target a missing path, a permission-denied path, a file where a directory is required, or a non-empty directory removed without recursion
- **THEN** each returns the corresponding `FmError` variant (`NotFound`, `PermissionDenied`, `NotADirectory`, `NotEmpty`, `AlreadyExists`)

#### Scenario: A file vanishing mid-operation
- **WHEN** an entry disappears between `read_dir` and `stat` (simulated by removing it in the test)
- **THEN** the second call returns `Err(NotFound)` and no panic occurs

#### Scenario: Volumes are compared correctly
- **WHEN** `same_device` is asked about two paths on the same filesystem and two paths on different filesystems (when the environment offers one)
- **THEN** it answers true and false respectively, and never panics when the answer is undecidable (returns false)

### Requirement: Reads are bounded and never execute on the UI thread
Directory reads SHALL be capped at a configured maximum entry count, and the UI SHALL reach the backend only through the engine's job lanes.

#### Scenario: A directory over the cap
- **WHEN** a directory contains more than the configured maximum entries
- **THEN** `read_dir` returns an actionable `FmError` naming the cap rather than allocating an unbounded vector

#### Scenario: No direct I/O from UI code
- **WHEN** `fm-ui` and `fm-app` sources are inspected (review + layering: only `fm-engine` above L0 performs filesystem work)
- **THEN** no widget or app module calls `std::fs` or a `Vfs` method directly — reads arrive as job events applied in `Session::poll`


### Requirement: Windows drives
On Windows the local root `file:///` SHALL be a virtual **This PC** folder listing every existing drive root (A:–Z:) as a folder; it is the default tree root, it appears in Places, `This PC` and a bare `D:` are accepted in the location bar, and Up from a drive root returns to it. It has no OS path, so writes to it fail with an error. Uris map to OS paths only through `VfsUri::local_path` (`/C:/x` → `C:\x`, `/C:` → `C:\`, `/server/share` → `\\server\share`), and paths are shown, copied and passed to child processes in that native form.

#### Scenario: Switch drives
- **WHEN** a Windows user opens This PC in the tree or Places
- **THEN** the available drives are listed and clicking `D:` shows `D:\`

#### Scenario: Drive labels
- **WHEN** a drive has a volume label
- **THEN** it is shown as `Label (C:)` in the tree and listing (sanitized, at most 64 characters) while its name, selection key and path stay `C:`
