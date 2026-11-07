# drag-and-drop Specification

## Purpose
TBD - created by archiving change dnd-and-file-ops. Update Purpose after archive.
## Requirements
### Requirement: Drag payload
Dragging from a listing row or tree node SHALL carry a single `DragPayload { sources, origin }`; if the dragged row is in the selection the payload is the whole selection, otherwise only that row.

#### Scenario: Drag a selection
- **WHEN** three selected rows are dragged by one of them
- **THEN** the payload contains all three uris

#### Scenario: Drag an unselected row
- **WHEN** an unselected row is dragged
- **THEN** the payload contains only that row

### Requirement: Drop targets
Drop destinations SHALL be: tree directory node → that directory; tree file node, listing file row, listing background and `..` row → the pane's relevant directory per architecture §8; listing directory row → that directory; pane chrome/borders → rejected.

#### Scenario: Drop on directory row
- **WHEN** files are released over a directory row
- **THEN** the destination is that directory

#### Scenario: Drop on chrome
- **WHEN** released over a pane border
- **THEN** nothing happens and the cursor was `NoDrop`

### Requirement: Modifier resolution
The action SHALL be resolved by the engine: none → move if same device else copy; Ctrl → copy; Shift → move; Ctrl+Shift → rejected; Escape, drop onto self or onto an ancestor of a source → cancel.

#### Scenario: Cross-device default
- **WHEN** a drop without modifiers crosses devices
- **THEN** the action is copy

#### Scenario: Shift forces move
- **WHEN** a same-device drop holds Shift
- **THEN** the action is move

### Requirement: Drop is a command
Releasing a payload SHALL dispatch `dnd.drop` with sources, destination and action; the UI never calls `ops`. Malformed params SHALL return `Err` without panicking.

#### Scenario: Fuzzed drop params
- **WHEN** `dnd.drop` receives empty, oversized or wrong-typed params
- **THEN** it returns `Err` and nothing is mutated

### Requirement: Drag feedback
The hovered valid target SHALL get an accent outline and a badge with the resolved action and count (`Copy 12 items`); invalid targets SHALL show `NoDrop`; all colours come from tokens in both palettes.

#### Scenario: Badge text
- **WHEN** three items hover a directory with Shift held
- **THEN** the badge reads `Move 3 items`

### Requirement: External drops
Files dropped from the OS SHALL be imported by copy into the hovered pane's directory via `dnd.drop` with an external origin; payload paths are untrusted and validated as `VfsUri`s.

#### Scenario: OS drop
- **WHEN** two files are dropped from the desktop onto a pane
- **THEN** a copy job into that pane's directory starts

#### Scenario: Hostile payload path
- **WHEN** a dropped path fails `VfsUri` validation
- **THEN** that path is rejected with an error and the rest still import

### Requirement: Dragging local files out to other applications
When an in-app drag of local files leaves the window with the primary button held, the application SHALL continue it as a native OS drag carrying those files, on Linux (X11 and native Wayland) and on Windows, so other applications can receive them. Remote items cannot be dragged out and SHALL produce a notice instead. The payload SHALL contain only absolute local paths, capped in count, and SHALL NOT stage, copy or delete any file. A failure SHALL surface as a notice and never as a crash. On platforms without a drag source (macOS) the application SHALL report that dragging out is not supported.

#### Scenario: Drop files into another Windows application
- **WHEN** the user drags selected local files out of a pane on Windows and releases over a window that accepts file drops
- **THEN** that application receives the files, the files stay where they are, and the status bar reports "Dropped files into another application"

#### Scenario: Cancel the OS drag
- **WHEN** the user presses Escape during the OS drag, or releases over a window that takes no files
- **THEN** nothing is dropped, no failure notice is shown, and the next click in the window works normally

#### Scenario: Remote items are not dragged out
- **WHEN** the dragged items include a remote (SFTP/SMB) item and the pointer leaves the window
- **THEN** no OS drag starts and a notice says only local files can be dragged out

#### Scenario: Unsupported platform
- **WHEN** a drag leaves the window on a platform without a drag source
- **THEN** the application shows a "not supported" notice and does not crash

