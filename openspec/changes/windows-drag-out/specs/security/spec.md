## MODIFIED Requirements

### Requirement: No unsafe code, no unchecked trust in dependencies
Our crates SHALL contain no `unsafe` (workspace-wide forbid), and every dependency added later that performs FFI or network I/O MUST be justified in `docs/architecture.md` and isolated in its L0 backend crate. The only exceptions are the user-approved platform drag-and-drop crates `fm-wayland` and `fm-winout`: each sets its own lints (`unsafe_code = "deny"`), allows `unsafe` in exactly one named module, documents the invariants of every `unsafe` call, and keeps all other code in the crate safe. The Windows drag source SHALL offer only the dragged local paths as a file-drop list, never read or modify the files, and SHALL validate the path list (absolute, no NUL, count-capped) before any OS call.

#### Scenario: Unsafe is structurally impossible
- **WHEN** any workspace crate other than `fm-wayland` and `fm-winout` enables `unsafe`
- **THEN** the build fails (`unsafe_code = "forbid"` in workspace lints)

#### Scenario: Dependency additions are auditable
- **WHEN** a change adds a dependency that does network or FFI work
- **THEN** the change updates the dependency table in `docs/architecture.md` (review checklist item; enforced by change review, not tooling)

#### Scenario: Hostile drag-out paths
- **WHEN** a drag-out payload contains an empty list, relative paths, paths with NUL, or more than the cap
- **THEN** the drag is refused with an error before any OS call and the application does not crash
