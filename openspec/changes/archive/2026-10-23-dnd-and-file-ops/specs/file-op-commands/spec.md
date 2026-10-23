## ADDED Requirements

### Requirement: File commands are registered
The registry SHALL contain `file.copy` (F5), `file.move` (F6), `file.mkdir` (F7), `file.delete` (F8, Delete), `file.delete_permanent` (Shift+Delete), `file.rename` (F2), `edit.undo` (Ctrl+Z), `job.cancel`, `job.resolve_conflict` and `dnd.drop`, each with id, label, menu path, shortcut, params doc, `enabled` and `run`, and menu/shortcut wiring checked by brand-check.

#### Scenario: Registry completeness
- **WHEN** brand-check cross-checks command ids against the app tables
- **THEN** every new id literal exists in the registry

### Requirement: Other-pane destination
`file.copy`/`file.move` SHALL use the selection of the focused pane and the directory of the most-recently-focused other pane as destination; with one pane they SHALL ask for a destination in a dialog.

#### Scenario: F5 with two panes
- **WHEN** two panes are open and F5 is pressed with a selection
- **THEN** a copy job targets the other pane's directory

#### Scenario: Nothing selected
- **WHEN** nothing is selected
- **THEN** the command is disabled and dispatch is an `Ok` no-op

### Requirement: Commands validate params
Every file command SHALL reject malformed, oversized or deep params with `Err` before doing any work, and the fuzz suite SHALL cover all of them.

#### Scenario: Bad params
- **WHEN** `file.rename` receives a name containing `/`
- **THEN** it returns `Err(InvalidParams)` and nothing changes

### Requirement: Input dialogs
mkdir and rename SHALL use input dialogs (prefilled name for rename) that validate the name live and dispatch the command only on confirm.

#### Scenario: Rename dialog
- **WHEN** F2 is pressed on one selected item
- **THEN** a dialog opens with its name selected and Enter dispatches `file.rename`
