## MODIFIED Requirements

### Requirement: The current and parent entries are always first
A pane's listing SHALL start with a `.` row for the current folder, followed by a `..` row when the pane is not at a root location. The `.` row SHALL be focusable but SHALL NOT become part of the selection, SHALL NOT be dragged, and SHALL NOT navigate when activated — activating it leaves the pane where it is.

#### Scenario: The current-folder row comes first
- **WHEN** the pane is showing any directory
- **THEN** the first row is `.` and, when the pane is not at a root location, the second row is `..`

#### Scenario: The current folder can be previewed
- **WHEN** the cursor is on the `.` row of a Files pane that a Preview pane follows
- **THEN** the Preview pane shows the pane's folder itself (directory summary or thumbnail grid), not a child entry

#### Scenario: The special rows stay out of the selection
- **WHEN** the user Shift-clicks from `.` or `..` to an entry, or presses Ctrl+A
- **THEN** only entry names are selected and the special rows are not part of the selection

#### Scenario: Activating the current-folder row is inert
- **WHEN** the user double-clicks or presses Enter on the `.` row
- **THEN** the pane does not navigate and the cursor stays on `.`

### Requirement: Keyboard navigation moves the focus row
Arrow keys and Home/End SHALL move the focused row within the listing, including the `.` and `..` rows; with Shift held the movement extends the selection over entry rows; plain movement clears a multi-selection to the focused row. Home SHALL land on `.`, and Up from `..` SHALL reach `.`.

#### Scenario: Arrow down with a multi-selection
- **WHEN** three rows are selected and the user presses Down without a modifier
- **THEN** the selection collapses to the new focused row

#### Scenario: Home reaches the current-folder row
- **WHEN** the user presses Home in the listing
- **THEN** the cursor is on `.`

#### Scenario: Up walks through the special rows
- **WHEN** the cursor is on the first entry and the user presses Up twice
- **THEN** the first press moves to `..` and the second to `.`

## ADDED Requirements

### Requirement: Opening a directory starts on its current-folder row
Navigating a pane to a directory — by selecting it in the tree, `nav.open`, or back/forward — SHALL put the cursor on the new location's `.` row with an empty selection, so the folder just opened is immediately the preview target. A refresh of the same location SHALL keep the existing cursor instead.

#### Scenario: Tree selection starts on the current folder
- **WHEN** the user selects a folder in the tree and it becomes the pane's location
- **THEN** the listing cursor is on `.` and nothing is selected

#### Scenario: Entering an empty directory
- **WHEN** a directory with no entries is opened
- **THEN** the listing shows `.` (and `..` when not at a root) with the cursor on `.`

#### Scenario: Refresh keeps the cursor
- **WHEN** the directory is re-read by `nav.refresh` after the user moved to another row
- **THEN** the cursor stays on that row
