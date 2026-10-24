## ADDED Requirements

### Requirement: Page keys move a screenful
PageUp and PageDown SHALL move the focused row by one visible screenful (minus one row so a row of context survives), with Shift extending the selection exactly like the arrow keys; the list SHALL scroll to keep the focused row visible on the same frame.

#### Scenario: Page down
- **WHEN** the user presses PageDown in the listing
- **THEN** the focused row advances by one screenful and the listing scrolls to it

#### Scenario: Shift page down
- **WHEN** the user holds Shift and presses PageDown
- **THEN** the selection grows from the anchor through the new focused row


### Requirement: Items have a shared context menu
Right-clicking a listing row or a thumbnail SHALL open one menu (Open, Copy, Move, Rename, New Folder, New File, Move to Trash, Delete Permanently) built from the command registry (label, shortcut, `enabled`). Choosing an entry SHALL dispatch its command through the same path as the menu bar, so input dialogs appear as for shortcuts. A right-click focuses the pane and selects the item unless it is already selected. `Delete` SHALL behave like `F8`.

#### Scenario: Registry-driven entries
- **WHEN** a context-menu entry's id is not a registered command
- **THEN** the unit test fails (no dead ids)

### Requirement: Clicking the only selected item releases it
A plain click on the single selected row SHALL clear the selection and put the cursor on the `.` row (the folder itself); a double-click still opens the item.

#### Scenario: Second click
- **WHEN** one file is selected and the user clicks it again
- **THEN** nothing is selected and `.` is the cursor row
