## Why

Dragging files out of a pane into a browser, mail or chat window works on Linux (X11 and native Wayland) but not on Windows: the drag simply ends at the window edge. Windows is a primary release target, so users there cannot drop files into upload fields or other applications.

## What Changes

- When an in-app file drag leaves the window on Windows, the same local files continue as a native OS drag (OLE drag source carrying `CF_HDROP`), so Explorer, browsers, mail and chat clients can receive them.
- New L0 crate `fm-winout` holds the Windows drag source. It is Windows-only code and the second approved `unsafe` exception (after `fm-wayland`); it sets its own lints and confines `unsafe` to one module.
- `fm-app/src/dragout.rs` starts the Windows drag the same way it starts the Linux ones; payload rules (local files only, capped count, no remote URIs) are unchanged and shared.
- Copy and Move are both offered; the outcome is reported ("Dropped files into another application") like on Linux.
- Pressing Escape or releasing over a non-target cancels cleanly.
- macOS stays unsupported and keeps the existing "not supported" notice.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `drag-and-drop`: adds the requirement that dragging local files out of the window starts an OS drag on Windows (and records the existing Linux behaviour).
- `security`: the "No unsafe code" requirement gains the `fm-winout` exception with its confinement rules.

## Impact

- New crate `crates/winout` (`fm-winout`, L0, registered in `xtask/src/layers.rs`); depends on `windows` (already in the lockfile via the GUI stack) with a minimal feature set, Windows targets only.
- `apps/fm-app/src/dragout.rs`, `apps/fm-app/Cargo.toml`, root `Cargo.toml`.
- `docs/architecture.md`, `docs/roadmap.md`, `AGENTS.md` (unsafe exception list), `CHANGELOG.md`.
- Cannot be exercised in Linux CI beyond pure logic (path/`DROPFILES` encoding); the drag itself needs a manual Windows smoke check.
