## ADDED Requirements

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

#### Scenario: Header click sorts without a re-read
- **WHEN** the user clicks the Size header
- **THEN** the cached entries are re-sorted in memory, the header shows the sort direction, and no job is submitted

#### Scenario: Directories first
- **WHEN** `view.sort.dirs_first` is on (default)
- **THEN** directories sort ahead of files within every sort column

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
Arrow keys and Home/End SHALL move the focused row within the listing; with Shift held the movement extends the selection; plain movement clears a multi-selection to the focused row.

#### Scenario: Arrow down with a multi-selection
- **WHEN** three rows are selected and the user presses Down without a modifier
- **THEN** the selection collapses to the new focused row

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
