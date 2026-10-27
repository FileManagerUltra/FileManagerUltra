# panes Specification

## Purpose
TBD - created by archiving change multi-pane-ui. Update Purpose after archive.
## Requirements
### Requirement: The window shows a navigable pane tree
The central area SHALL render the session's `Layout` recursively: each leaf is a pane (location bar, file tree, file listing) and each split is a pair of panes separated by a draggable handle; at least one pane always exists.

#### Scenario: Fresh start
- **WHEN** the application starts a session with the default layout
- **THEN** exactly one pane is shown, occupying the whole central area, with its tree and listing visible

#### Scenario: Split renders two panes
- **WHEN** the user triggers `pane.split.horizontal` or `pane.split.vertical`
- **THEN** the focused pane's cell is replaced by a split containing the original pane and a new empty-location pane
- **AND** the new pane becomes focused

#### Scenario: Dragging a split handle
- **WHEN** the user drags the handle between two panes
- **THEN** the split ratio updates in the layout data (clamped to sane bounds) and both panes re-layout
- **AND** no filesystem call occurs during the drag

### Requirement: Panes can be split, closed, focused and swapped by command
All pane topology changes SHALL be engine commands; the UI never mutates the layout tree directly.

#### Scenario: Closing a pane keeps the tree valid
- **WHEN** `pane.close` runs with two or more panes
- **THEN** the focused leaf is removed and its sibling inherits the parent slot
- **AND** the remaining pane becomes focused

#### Scenario: The last pane cannot be closed
- **WHEN** `pane.close` runs with exactly one pane
- **THEN** the command's `enabled` predicate is false (menu greyed) and dispatch is a no-op that returns `Ok` without changing the layout

#### Scenario: Tab swaps the two most recent panes
- **WHEN** the user presses Tab (`pane.swap`) after focusing pane B following pane A
- **THEN** the subtrees rooted at A and B exchange places and focus moves to the swap partner
- **AND** per-pane state (location, selection, history, tree expansion) travels with its pane

#### Scenario: Focus cycles
- **WHEN** `pane.focus.next` runs
- **THEN** focus advances to the next leaf in tree order, wrapping at the end

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
