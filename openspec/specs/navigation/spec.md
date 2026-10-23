# navigation Specification

## Purpose
TBD - created by archiving change multi-pane-ui. Update Purpose after archive.
## Requirements
### Requirement: Panes navigate by command
Opening a directory, going to the parent, back, forward and refreshing SHALL each be engine commands (`nav.open`, `nav.up`, `nav.back`, `nav.forward`, `nav.refresh`) dispatched by id; the UI never navigates on its own.

#### Scenario: Open by tree or listing
- **WHEN** the user activates a directory in the tree or double-clicks a directory row in the listing
- **THEN** `nav.open` runs with that uri, the pane's location and history update, and a read job fills the listing

#### Scenario: Open is validated before committing
- **WHEN** `nav.open` receives a uri that does not parse
- **THEN** it returns `Err(InvalidParams)` and the pane is unchanged

#### Scenario: Re-opening the current directory is a reload, not a jump
- **WHEN** `nav.open` receives the uri the pane is already at
- **THEN** the pane keeps its history, selection, focus and scroll unchanged and no duplicate history entry is pushed

#### Scenario: Refresh keeps the pane in place
- **WHEN** `nav.refresh` runs in a pane with history, selection and scroll state
- **THEN** the location and history are unchanged and only the cached entries are re-read

### Requirement: Each pane keeps back/forward history
History SHALL be per pane: forward commits push onto the back stack and clear the forward stack; `nav.back`/`nav.forward` move through it without re-reading when the target is already cached.

#### Scenario: Back returns to the previous directory
- **WHEN** the user navigates A → B and triggers `nav.back`
- **THEN** the pane returns to A, and `nav.forward` returns to B

#### Scenario: New navigation clears the forward stack
- **WHEN** the user is in the middle of history and opens a new directory
- **THEN** the forward stack is emptied

#### Scenario: Back at the start of history
- **WHEN** `nav.back` runs with an empty back stack
- **THEN** the command is disabled (or a no-op returning `Ok`) and the pane is unchanged

### Requirement: Shortcuts reach navigation commands
The shortcut table SHALL map Backspace to `nav.up`, Alt+Left to `nav.back`, Alt+Right to `nav.forward` and Ctrl+R to `nav.refresh`, each existing in the registry (checked by the brand-check cross-check).

#### Scenario: Backspace goes up
- **WHEN** the user presses Backspace in a pane that is not at its root
- **THEN** the pane navigates to the parent directory

#### Scenario: Up at the root
- **WHEN** Backspace is pressed at the pane's tree root
- **THEN** nothing happens and no error surfaces

### Requirement: The location bar shows where the pane is
Each pane SHALL show its current location (path or uri) in a chrome bar; the bar reflects committed navigation including remote-shaped uris (so change 4 needs no chrome rework).

#### Scenario: Local path display
- **WHEN** a pane is at `/home/user/Documents`
- **THEN** the location bar reads `/home/user/Documents` (or the equivalent `file://` display) and updates on every committed navigation

