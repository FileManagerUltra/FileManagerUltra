## ADDED Requirements

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
