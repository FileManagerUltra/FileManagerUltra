## MODIFIED Requirements

### Requirement: No unsafe code, no unchecked trust in dependencies
Our crates SHALL contain no `unsafe` (workspace-wide forbid), and every dependency added later that performs FFI or network I/O MUST be justified in `docs/architecture.md` and isolated in its L0 backend crate. The only exceptions are the user-approved platform crates `fm-wayland`, `fm-winout` and `fm-macsec`: each sets its own lints (`unsafe_code = "deny"`), allows `unsafe` in exactly one named module, documents the invariants of every `unsafe` call, and keeps all other code in the crate safe. `fm-macsec` SHALL contain no `unsafe` at all when every Foundation/AppKit call it needs is safe in `objc2`; its exception exists only for calls `objc2` marks `unsafe`. The Windows drag source SHALL offer only the dragged local paths as a file-drop list, never read or modify the files, and SHALL validate the path list (absolute, no NUL, count-capped) before any OS call.

#### Scenario: Unsafe is structurally impossible
- **WHEN** any workspace crate other than `fm-wayland`, `fm-winout` and `fm-macsec` enables `unsafe`
- **THEN** the build fails (`unsafe_code = "forbid"` in workspace lints)

#### Scenario: Dependency additions are auditable
- **WHEN** a change adds a dependency that does network or FFI work
- **THEN** the change updates the dependency table in `docs/architecture.md` (review checklist item; enforced by change review, not tooling)

#### Scenario: Hostile drag-out paths
- **WHEN** a drag-out payload contains an empty list, relative paths, paths with NUL, or more than the cap
- **THEN** the drag is refused with an error before any OS call and the application does not crash

## ADDED Requirements

### Requirement: Folder grants are stored owner-only and loaded as untrusted data
The grants file SHALL be written atomically with owner-only permissions inside the
app's state folder, SHALL contain only paths and bookmark data (no credentials), and
SHALL be capped at 256 grants of at most 64 KiB bookmark data each. On load, a missing,
truncated, oversized or malformed file or entry SHALL be skipped with a logged notice,
never crash, and never widen access beyond what resolved bookmarks grant.

#### Scenario: Corrupt grants file
- **WHEN** the grants file contains invalid TOML, a 10 MiB bookmark, a relative path or 10 000 entries
- **THEN** the app starts, loads at most the valid entries within the caps and reports the rest as skipped

#### Scenario: Permissions
- **WHEN** a grant is stored
- **THEN** the file has mode `0600` and its folder `0700`

### Requirement: The App Store edition never asks for broader access than the user grants
The App Store edition SHALL NOT request Full Disk Access, SHALL NOT use temporary
sandbox exceptions or Apple Events, SHALL NOT spawn helper processes, and SHALL reach
local files only through grants, dropped items and its own container.

#### Scenario: No child processes
- **WHEN** the App Store edition's command fuzz and integration tests run with process spawning instrumented
- **THEN** no command path spawns a process
