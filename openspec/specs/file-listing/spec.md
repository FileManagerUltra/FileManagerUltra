# file-listing Specification

## Purpose
TBD - created by archiving change multi-pane-ui. Update Purpose after archive.
## Requirements
### Requirement: The listing is virtualized
A pane's file listing SHALL render a fixed row height and instantiate widgets only for the visible rows, so a directory at the entry cap scrolls without per-row widget cost.

#### Scenario: 200 000 rows stay one screenful
- **WHEN** the listing is bound to a fixture directory of 200 000 entries
- **THEN** only about a screenful of row widgets exists at any time, scrolling is smooth, and the frame does not allocate per-row closures for off-screen rows

#### Scenario: The parent entry is always first
- **WHEN** the pane is not at a root location
- **THEN** the listing starts with a `..` row that navigates to the parent

### Requirement: Columns show identity, size, time and kind, and are sortable
The listing SHALL show name, size, modified time and entry kind; clicking a column header SHALL sort by that column, toggling ascending/descending on repeat, with directories-first as a toggleable option — sorting is a command, executed over the cached entries.

#### Scenario: File times use the operating system timezone
- **WHEN** a file timestamp is displayed in a listing, Properties or a conflict dialog
- **THEN** it uses the operating system timezone detected at startup, respecting daylight-saving rules at the file's timestamp and the selected date format
- **AND** local and remote files use the same local display timezone; stored timestamps and sorting remain unchanged
- **AND** timezone detection and timezone-data loading occur before rendering, never per row or per frame
- **AND** if timezone detection fails, UTC is used with a visible notice and a diagnostic rather than silently claiming local time
- **AND** explicitly UTC log filters and log source timestamps are unchanged

#### Scenario: Header click sorts without a re-read
- **WHEN** the user clicks the Size header
- **THEN** the cached entries are re-sorted in memory, the header shows the sort direction, and no job is submitted

#### Scenario: Directories first
- **WHEN** `view.sort.dirs_first` is on (default)
- **THEN** directories sort ahead of files within every sort column

### Requirement: Content-aware, elastic column resizing
Size, Modified and Type SHALL initially fit their displayed content and headers, including space for a sort indicator. Name SHALL take the remaining width and SHALL NOT auto-size to the longest filename. Content measurements SHALL be cached per directory revision and font/scale, not recalculated over the directory during every drag frame.

Dragging a header divider SHALL follow the pointer using the drag's original geometry, redistributing neighboring space rather than moving every earlier column unnecessarily. Extra whitespace SHALL be reclaimed before compressing content; Name supplies spare room when neighboring content already fits. Where a dragged boundary leaves insufficient room, detail columns may compress. Reversing a drag after reaching a limit SHALL not accumulate jumps. Header and rows SHALL paint the updated geometry in the same frame.

When a pane narrows, columns SHALL compress continuously, rather than disappearing at fixed-width breakpoints. Extremely narrow panes SHALL progressively give up Type, then Size, then Modified space while retaining a usable Name column. Text SHALL be truncated and clipped within its cell; no filename, detail or header may overlap a neighboring column. Automatic compression SHALL NOT overwrite stored width preferences, and expanding the pane SHALL restore them.

Preferred widths SHALL persist in the pane's workspace state. Zero means content fitting; manually chosen widths remain bounded to 40..=400 logical pixels. Double-clicking a divider SHALL restore content fitting for the detail column immediately to its right. Resize and content-fit actions SHALL dispatch `view.columns.resize {pane?, widths: [size, modified, type]}`; malformed input SHALL fail without changing state. Legacy fixed defaults `[90, 140, 72]` SHALL migrate to content fitting; other saved manual widths SHALL be retained.

#### Scenario: Make Modified wider without shifting the filename boundary
- **WHEN** the user drags the Size/Modified divider left and Size has spare width
- **THEN** Modified grows while Size gives up that spare width, and the Name boundary remains in place

#### Scenario: Neighboring columns already fit their content
- **WHEN** a divider is dragged between two content-fitted columns
- **THEN** it is not locked: surrounding room is redistributed, with compression only where the new geometry cannot fit the content

#### Scenario: Fit a manually resized column
- **WHEN** a divider is double-clicked
- **THEN** the column to its right returns to content fitting and that choice survives saving/restoring the workspace

#### Scenario: Resize a narrow pane and restore it
- **WHEN** a pane becomes too narrow for its preferred columns and is then expanded
- **THEN** detail widths compress smoothly and later recover, without altering the saved preferences

### Requirement: Selection is multi-select and survives re-reads
The listing SHALL support click-to-select, Shift-range selection, Ctrl-toggle, Ctrl+A select-all and Escape clear; selection SHALL be stored by entry name so a re-read of the same directory preserves it, pruning entries that vanished.

#### Scenario: Shift range
- **WHEN** the user clicks row 3, then Shift-clicks row 7
- **THEN** rows 3–7 inclusive are selected and the focused row is 7

#### Scenario: Selection survives refresh
- **WHEN** rows are selected and `nav.refresh` re-reads the directory with the same names present
- **THEN** the selection is unchanged

#### Scenario: Vanished entries are pruned
- **WHEN** a re-read no longer contains a selected name
- **THEN** it drops out of the selection without error

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

### Requirement: The listing renders empty, loading and error states
A pane whose cache has no revision yet SHALL show a loading state; a directory with no entries SHALL show an empty state; a failed read SHALL show the error message with retry — never a blank panel.

#### Scenario: Loading then content
- **WHEN** a pane is navigated to a slow directory
- **THEN** a loading indicator shows until the read completes and the rows appear without a layout jump in chrome

### Requirement: A refresh never moves the pane
`nav.refresh` SHALL reload the cached entries in place: across the loading gap the pane's focused row, selection and scroll offset SHALL be preserved, so a reload does not change where the user is looking.

#### Scenario: Focus and scroll survive the reload
- **WHEN** the listing is scrolled partway down with a focused row and `nav.refresh` re-reads the directory with the same names present
- **THEN** the selection and focused row are unchanged and the listing returns to the same scroll offset once the rows reappear

#### Scenario: The loading frames do not write back a collapsed offset
- **WHEN** the directory is re-reading and the pane renders its loading state
- **THEN** the pane's stored scroll offset is not overwritten by the collapsed content height

### Requirement: Page keys move a screenful
PageUp and PageDown SHALL move the focused row by one visible screenful (minus one row so a row of context survives), with Shift extending the selection exactly like the arrow keys; the list SHALL scroll to keep the focused row visible on the same frame.

#### Scenario: Page down
- **WHEN** the user presses PageDown in the listing
- **THEN** the focused row advances by one screenful and the listing scrolls to it

#### Scenario: Shift page down
- **WHEN** the user holds Shift and presses PageDown
- **THEN** the selection grows from the anchor through the new focused row

### Requirement: Items have a shared context menu
Right-clicking a listing row or a thumbnail SHALL open one menu (Open, Open in Preview Panel, Copy, Move, Rename, New Folder, New File, Move to Trash, Delete Permanently) built from the command registry (label, shortcut, `enabled`). Choosing an entry SHALL dispatch its command through the same path as the menu bar, so input dialogs appear as for shortcuts. A right-click focuses the pane and selects the item unless it is already selected. Open in Preview Panel SHALL retain the pointed URI until dispatch and preview only that item, reusing an existing Preview or creating one if absent. `Delete` SHALL behave like `F8`.

#### Scenario: Registry-driven entries
- **WHEN** a context-menu entry's id is not a registered command
- **THEN** the unit test fails (no dead ids)

### Requirement: Clicking the only selected item releases it
A plain click on the single selected row SHALL clear the selection and put the cursor on the `.` row (the folder itself); a double-click still opens the item.

#### Scenario: Second click
- **WHEN** one file is selected and the user clicks it again
- **THEN** nothing is selected and `.` is the cursor row

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

### Requirement: Quick filter and selection commands
Typing in a focused Files listing SHALL open a filter bar above the listing with the typed text (a leading space does not open it) and put the cursor in it; `view.filter {text?, focus?}` does the same from the menu. The bar narrows the rows to names containing the text, or matching it when it has wildcards (`* ? [..]`); the filter is transient, cleared by Esc, the bar's clear button (`view.filter.clear`) or navigating away, and the selection is pruned to visible rows. Typing SHALL NOT open the location bar; Ctrl+L or clicking the path does. `edit.select_all` (Ctrl+A), `edit.select_none` (Ctrl+Shift+A), `edit.invert_selection` (Ctrl+Shift+I) and `edit.select_pattern {pattern, mode?}` (Ctrl+Shift+S, a dialog) SHALL act on the visible rows only.

#### Scenario: Filter then select all
- **WHEN** the user types `o` in the listing and presses Ctrl+A
- **THEN** only the visible matching rows are selected

#### Scenario: Typing a path-like text
- **WHEN** the user types `/etc` in a focused listing
- **THEN** the listing is filtered by `/etc` and the location bar does not open

