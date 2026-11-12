# panes Specification

## Purpose
TBD - created by archiving change multi-pane-ui. Update Purpose after archive.
## Requirements
### Requirement: Folder context menus target the pointed folder
Right-clicking blank Files-listing space or the current-folder (`.`) row SHALL show
a folder context menu with New Folder, New File, Paste, Refresh, Copy Path, Copy Name,
Add Bookmark and Properties. Right-clicking an ordinary folder in the tree SHALL show
the same menu, targeting that folder without navigating into it. Labels, shortcuts
and availability SHALL come from registered commands; virtual locations SHALL not
offer enabled file-creation or paste actions. Saved-server entries retain their
Connect/Edit/Delete menu.

#### Scenario: Background menu in an unfocused pane
- **WHEN** a user right-clicks unused listing space in a Files pane
- **THEN** that pane becomes focused, its file selection is cleared, and folder actions target its current location
- **AND** file-row context menus are unchanged, including in populated and filtered listings

#### Scenario: Tree folder actions
- **WHEN** a user right-clicks a tree folder and chooses New Folder, New File or Paste
- **THEN** the dialog or operation targets the clicked folder rather than the listing's location or selection
- **AND** Refresh, Copy Path, Copy Name, Add Bookmark and Properties also target that folder
- **AND** opening the menu does not change location, navigation history or expansion

### Requirement: The window shows a navigable pane tree
The central area SHALL render the session's `Layout` recursively: each leaf is a pane (location bar, file tree, file listing) and each split is a pair of panes separated by a draggable handle; at least one pane always exists.

#### Scenario: Fresh start
- **WHEN** the application starts a session with the default layout
- **THEN** exactly one pane is shown, occupying the whole central area, with its tree and listing visible

#### Scenario: Split renders two panes
- **WHEN** the user triggers `pane.split.horizontal` or `pane.split.vertical`
- **THEN** the focused pane's cell is replaced by a split containing the original pane and a new empty-location pane
- **AND** the new pane becomes focused

#### Scenario: Splitting a Preview or Media pane
- **WHEN** a Preview pane (or the Media pane) is split, for example when it is the only pane left
- **THEN** the new pane is a Files pane, not a second Preview; it keeps the folder the pane last listed when that is local or its server is still connected, and otherwise starts at the home folder, so no login dialog appears for a server that is not open

#### Scenario: Dragging a split handle
- **WHEN** the user drags the handle between two panes
- **THEN** the split ratio updates in the layout data (clamped to sane bounds) and both panes re-layout
- **AND** no filesystem call occurs during the drag

### Requirement: Panes can be split, closed, focused and swapped by command
All pane topology changes SHALL be engine commands; the UI never mutates the layout tree directly.

#### Scenario: Closing a pane keeps the tree valid
- **WHEN** `pane.close` runs with two or more panes
- **THEN** the focused leaf is removed and its sibling inherits the parent slot
- **AND** if the closed pane was focused and its previous-focus pane still exists, that pane becomes focused
- **AND** otherwise a surviving pane becomes focused

#### Scenario: Closing a non-focused pane keeps focus
- **WHEN** `pane.close {pane}` closes a pane other than the focused pane
- **THEN** the currently focused pane remains focused unless it is no longer available

#### Scenario: The last pane cannot be closed
- **WHEN** `pane.close` runs with exactly one pane
- **THEN** the command's `enabled` predicate is false (menu greyed) and dispatch is a no-op that returns `Ok` without changing the layout

### Requirement: Pane state survives splits and closes
Tree expansion, listing sort, selection, scroll and history SHALL live in `fm-layout`'s per-pane state, keyed by `PaneId`, so topology changes never reset a pane's view.

#### Scenario: Shift-wheel scrolls horizontally
- **WHEN** the user holds Shift and scrolls the mouse wheel over a pane's scrollable content
- **THEN** the gesture scrolls horizontally where the content overflows
- **AND** it does not fall back to vertical scrolling when there is no horizontal overflow
- **AND** ordinary wheel scrolling remains vertical and affects the hovered pane, not a different focused pane

#### Scenario: Split preserves view state
- **WHEN** a pane with an expanded tree, a non-default sort and a selection is split
- **THEN** the original pane keeps its expansion, sort and selection after the split

### Requirement: Split handles give feedback and stay usable
A split handle SHALL highlight and show a resize cursor on hover, SHALL never shrink a pane below its minimum size, and a double-click SHALL reset that split to its equalised ratio.

#### Scenario: Double-click resets
- **WHEN** the user double-clicks a split handle
- **THEN** that split's ratio becomes its equalised value

### Requirement: Panes have a close button
Every pane header SHALL show a close button while more than one pane exists, dispatching `pane.close {pane}`. `pane.close` SHALL accept an optional `pane` id (default: the focused pane) and reject a non-numeric or unknown id with `Err` and no change.

#### Scenario: Close by id
- **WHEN** `pane.close {"pane": <id>}` is dispatched with three panes open
- **THEN** exactly that pane is closed and the other two remain

