## ADDED Requirements

### Requirement: Sync view layout
The sync view SHALL show a top bar (left folder, mode selector, right folder, Compare and Synchronize buttons), a comparison grid, a summary bar and a profile list. Folders SHALL be chosen through the existing path/place pickers.

#### Scenario: Open from panes
- **WHEN** `sync.open` runs with two panes open
- **THEN** the view opens with the panes' folders prefilled as left and right

### Requirement: Comparison grid
The grid SHALL be virtualized and list, per item: left name/size/mtime, an action marker (copy right, copy left, delete left, delete right, skip, conflict, equal), and right name/size/mtime. Rows SHALL be filterable by action type and sortable by path, size or time. Conflict rows SHALL be visually distinct.

#### Scenario: 100 000 rows
- **WHEN** a plan holds 100 000 items
- **THEN** only visible rows are laid out and a frame does not wait on I/O

### Requirement: Per-row action override
The user SHALL be able to change an item's action (including resolving a conflict by choosing a direction, or excluding the item) before running, through `sync.set_action`. The summary SHALL update immediately.

#### Scenario: Resolve conflict
- **WHEN** the user sets a conflict row to copy left to right
- **THEN** the conflict count drops by one and that file is copied on run

### Requirement: Summary and confirmation
The summary bar SHALL show counts and bytes per action, deletes (flagging permanent ones), conflicts and attribute warnings. Synchronize SHALL show the deletion confirmation required by the safeguards before running.

#### Scenario: Permanent deletes
- **WHEN** the plan contains permanent deletes
- **THEN** the confirmation names their number and the target

### Requirement: Progress and results
Running SHALL show progress in the jobs panel and per-item results (done, failed, warning with unpreserved attributes) in the grid; failures SHALL be actionable messages.

#### Scenario: Failure
- **WHEN** one file fails to copy
- **THEN** its row shows the error, the rest continue and the run ends as partially failed

### Requirement: Theming and thin UI
The view SHALL use only theme tokens (dark and light) and SHALL dispatch commands by id; it SHALL NOT call backends or `fm-sync` directly.

#### Scenario: Light theme snapshot
- **WHEN** the sync view is rendered with `Tokens::light()` and `Tokens::dark()`
- **THEN** both snapshots show readable text and distinct action markers
