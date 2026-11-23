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

### Requirement: Pane zoom
Every pane header SHALL have a zoom button (a magnifying glass with a plus) just left of the pane type selector. Clicking it, or Enter/Space while it has keyboard focus, SHALL open the pane's zoom controls stacked vertically: zoom in (+), the current percentage (which resets to 100%) and zoom out (−). The popup SHALL close on a click outside it or on Escape, and Escape SHALL return keyboard focus to the zoom button. The controls SHALL meet WCAG 2.2 AA: while the pane's zoom is not 100% the button SHALL show the percentage as text left of the icon, plus an outline, so the state is not shown by colour alone (1.4.1); the button SHALL always reserve the width of the widest percentage, so showing or changing the percentage moves neither the button, the header beside it nor the open popup (no layout shift); the button and each popup button SHALL be at least 24×24 points (2.5.8); icon and text SHALL have at least 4.5:1 contrast and the outline and focus ring at least 3:1 against the header and its hover fill, in both themes and for any accent colour (1.4.3, 1.4.11); the button SHALL be named "Zoom" (or "Zoom, N%" while zoomed) and report whether its popup is open (expanded), and the popup buttons SHALL be named "Zoom … in", "Reset … zoom (N%)" and "Zoom … out" (1.1.1, 2.5.3, 4.1.2); Tab SHALL move from the button into the popup (2.1.1, 2.4.3). The controls SHALL dispatch `pane.scale.in`, `pane.scale.out` and `pane.scale.reset` (`{pane?}`, default the focused pane), which change the zoom of that pane only: a Terminal pane's terminal zoom, a Preview pane's preview zoom, and for Files, Network, SFTP, Trash and Media panes a pane zoom from 50% to 400% in ×1.2 steps that scales the pane's text, row height and spacing, and in a Media pane also its player controls (transport and toolbar buttons with their icons, seek and volume bars, clock, title, cover art, playlist numbers, marks and indents) (not its header), is saved with the pane in its workspace, and leaves the interface zoom and other panes unchanged. A stored pane zoom that is not a finite number in range SHALL be read as bounded (100% when not a number). These commands have no default shortcut. Ctrl+mouse wheel (Cmd on macOS) and a trackpad pinch over a pane SHALL run `pane.scale.in` / `pane.scale.out` for the pane under the pointer, focused or not and of any kind, one step per wheel notch (small pinch factors add up to a step), and never through a window or popup lying over the pane; the wheel without Ctrl keeps scrolling. Ctrl+= / - / 0 still zoom a focused Preview; Ctrl+Shift+= / - / 0 SHALL be the interface zoom in every pane, a terminal or Preview included.

#### Scenario: Controls stay out of the way
- **WHEN** a pane is shown
- **THEN** its header shows only the zoom button (and the percentage while zoomed), not the zoom controls, until the button is clicked or activated from the keyboard

#### Scenario: No layout shift
- **WHEN** the popup is open and the user clicks + (100% → 120%) and then − twice
- **THEN** the button, its icon and the popup stay exactly where they were; only the percentage text appears and changes

#### Scenario: Keyboard only
- **WHEN** the zoom button has focus and the user presses Enter, Tab, Space and then Escape
- **THEN** the popup opens, focus moves to zoom in, the pane zooms in one step, the popup closes, and focus is back on the button, which now reads "Zoom, 120%" and reports collapsed

#### Scenario: One Files pane larger
- **WHEN** two Files panes are open and the user opens the left one's zoom and clicks + twice
- **THEN** the left pane's rows and text are drawn at 144%, its zoom button reads 144% with an outline, and the right pane, both headers and the interface zoom are unchanged

#### Scenario: Ctrl+wheel zooms the pane under the pointer
- **WHEN** two panes are open and the user holds Ctrl and turns the wheel one notch up over the pane that is not focused, then one notch down
- **THEN** that pane zooms to 120% and back to 100%, and the other pane and the interface zoom are unchanged

#### Scenario: Media player controls zoom too
- **WHEN** a Media pane is zoomed to 200%
- **THEN** its previous, play, stop and next buttons and their icons are twice as large, the seek bar twice as tall, and the playlist numbers and names stay aligned without overlapping

#### Scenario: Terminal and Preview keep their own zoom
- **WHEN** the header zoom of a Terminal or Preview pane is used
- **THEN** it changes that terminal's or preview's existing zoom (the same value its keyboard shortcuts change), not a separate pane zoom

#### Scenario: Bad params
- **WHEN** `pane.scale.*` names an unknown pane or receives malformed params
- **THEN** it returns an error and no zoom changes

