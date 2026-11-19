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
History SHALL be per pane: forward commits push onto the back stack and clear the forward stack; `nav.back`/`nav.forward` move through it without re-reading when the target is already cached. They act on the focused pane, or on `{pane}` when given (an unknown pane or a non-numeric `pane` is an `Err`). History lasts for the session only: it is never saved with a workspace, so every start and restored workspace begins without history (a split copies the history of the pane it splits, closing a pane drops its history), and each direction keeps at most 100 folders (the oldest are dropped).

#### Scenario: Arrows in the header
- **WHEN** a Files pane is shown
- **THEN** its header has Back and Forward arrows before the path, greyed out and inert when there is nowhere to go, at least 24×24 px, with the shortcuts in their tooltips; clicking one focuses that pane and steps its history

#### Scenario: Mouse back and forward buttons
- **WHEN** the mouse's back (or forward) thumb button is pressed over a Files pane, focused or not, and no window or popup lies over it
- **THEN** that pane steps back (or forward) and keyboard focus stays where it was

#### Scenario: Not saved
- **WHEN** a workspace with a pane that has history is saved and reopened
- **THEN** the pane is at its saved folder with no back or forward history

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
Each pane SHALL show its current location in a chrome bar, in the location's display form (see `path-display`); the bar reflects committed navigation including remote locations. Starting to edit SHALL put the same display form in the editor.

#### Scenario: Local path display
- **WHEN** a pane is at `/home/user/Documents` on Linux
- **THEN** the location bar reads `/home/user/Documents` and updates on every committed navigation

#### Scenario: Windows paths
- **WHEN** a pane on Windows is at `file:///C:/Users/alice` or at `smb://nas/media`
- **THEN** the location bar reads `C:\Users\alice` or `\\nas\media`

#### Scenario: SFTP folder
- **WHEN** a pane is at `sftp://alice@host/srv/www`
- **THEN** the location bar reads `sftp://alice@host/srv/www`

### Requirement: Opening an unconnected SMB location signs in
`nav.open` on an `smb://` location without a connection SHALL start the sign-in (straight away with a remembered login, otherwise the Authentication Required popup) instead of failing, keep the pane where it is while connecting, and open the location once connected. Cancelling the sign-in SHALL leave the pane unchanged.

#### Scenario: Bookmark on a disconnected share
- **WHEN** the user opens a bookmark for `smb://192.168.0.50/media` with no connection to that server
- **THEN** the Authentication Required popup opens for `192.168.0.50`, no "no backend" error is shown, and after signing in the pane shows `media`

