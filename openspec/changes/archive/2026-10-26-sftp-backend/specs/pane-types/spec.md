## MODIFIED Requirements

### Requirement: Placeholder kinds
`Network` SHALL appear in the selector and command but be disabled until its backend lands; running the command for it returns an actionable error and never changes the pane. `Sftp` SHALL be a working kind: a pane of kind `Sftp` that has no server yet shows an inline connect form (profile picker, host, port, user, auth, start folder) instead of a listing, and once connected behaves as a Files pane on the server (tree, listing, history, file commands). `remote.connect.dialog` (File ▸ Connect to Server…) switches the focused pane to `Sftp`.

#### Scenario: Selecting Sftp
- **WHEN** `pane.kind.set {"kind":"sftp"}` runs
- **THEN** the pane's kind is `Sftp` and it shows the connect form until a connection is made

#### Scenario: Selecting Network
- **WHEN** `pane.kind.set {"kind":"network"}` runs
- **THEN** it returns an error naming the missing backend and the pane keeps its kind

## ADDED Requirements

### Requirement: Remote pane controls
A remote pane SHALL show a Disconnect button left of the pane-type dropdown, and its location label SHALL include the host. Delete on a remote pane SHALL be labelled "Delete Permanently".

#### Scenario: Disconnect
- **WHEN** the user presses Disconnect
- **THEN** the pane returns to the local home folder
