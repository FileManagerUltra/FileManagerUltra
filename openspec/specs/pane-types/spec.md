# pane-types Specification

## Purpose
TBD - created by archiving change pane-types. Update Purpose after archive.
## Requirements
### Requirement: Pane kinds
Every pane SHALL have exactly one `PaneKind` from `Files`, `Terminal`, `Sftp`, `Network`; new panes are `Files`. The kind is view state in `fm-layout` and changes only through the `pane.kind.set {kind}` command.

#### Scenario: Switch a pane to a terminal
- **WHEN** `pane.kind.set {"kind":"terminal"}` runs on the focused pane
- **THEN** that pane's kind is `Terminal` and other panes are unchanged

#### Scenario: Unknown kind
- **WHEN** `pane.kind.set` runs with an unknown kind, a missing or non-string `kind`, or non-object params
- **THEN** it returns an error and the pane is unchanged

### Requirement: Leaving a terminal ends it
Switching a pane away from `Terminal` SHALL terminate its shell and everything running in it. When a command other than the shell owns the terminal (where the platform can tell), `pane.kind.set` MUST NOT switch until the user confirms (`confirm: true`); the UI shows a confirmation and cancelling leaves the terminal untouched.

#### Scenario: Idle terminal
- **WHEN** a terminal with only its shell running switches to `Files`
- **THEN** the shell is terminated and the pane is `Files`, with no prompt

#### Scenario: Busy terminal
- **WHEN** a terminal running `sleep 30` switches to `Files` without `confirm`
- **THEN** the pane stays `Terminal`, a confirmation is pending, and nothing is killed
- **WHEN** the switch is repeated with `confirm: true`
- **THEN** the terminal and its processes are terminated and the pane is `Files`

### Requirement: Files state survives a kind switch
Switching a pane away from `Files` and back SHALL restore its location, history, selection, sort and tree state unchanged.

#### Scenario: Round trip
- **WHEN** a Files pane at `/a/b` with a selection switches to `Terminal` and back to `Files`
- **THEN** it shows `/a/b` with the same selection and history

### Requirement: Placeholder kinds
`Network` SHALL appear in the selector and command but be disabled until its backend lands; running the command for it returns an actionable error and never changes the pane. `Sftp` SHALL be a working kind: a pane of kind `Sftp` that has no server yet shows an inline connect form (profile picker, host, port, user, auth, start folder) instead of a listing, and once connected behaves as a Files pane on the server (tree, listing, history, file commands). `remote.connect.dialog` (File ▸ Connect to Server…) switches the focused pane to `Sftp`.

#### Scenario: Selecting Sftp
- **WHEN** `pane.kind.set {"kind":"sftp"}` runs
- **THEN** the pane's kind is `Sftp` and it shows the connect form until a connection is made

#### Scenario: Selecting Network
- **WHEN** `pane.kind.set {"kind":"network"}` runs
- **THEN** it returns an error naming the missing backend and the pane keeps its kind

### Requirement: Kind selector
Each pane header SHALL offer a kind selector (and View ▸ Pane Type menu) that dispatches `pane.kind.set`; the UI MUST NOT mutate the kind directly. Colours come from theme tokens and render in dark and light.

#### Scenario: Selector dispatches the command
- **WHEN** the user picks Terminal in the selector
- **THEN** `pane.kind.set` is dispatched for that pane

### Requirement: Kind-aware commands
File commands (`file.*`, `nav.*`, drag & drop) SHALL be disabled when the focused pane is not `Files`; a non-Files pane is never a drop target or an F5/F6 destination.

#### Scenario: Copy with a terminal focused
- **WHEN** a Terminal pane is focused and `file.copy` is queried
- **THEN** it is disabled (running it anyway is a no-op)

### Requirement: Preview is a supported pane kind
`PaneKind::Preview` SHALL appear in the type switcher and be accepted by `pane.kind.set`; switching to it SHALL not change any Files pane, and switching away SHALL stop any playback.

#### Scenario: Switch to preview
- **WHEN** `pane.kind.set {kind: "preview"}` runs on a Files pane
- **THEN** the pane becomes a Preview following the most recently focused other Files pane

### Requirement: Remote pane controls
A remote pane SHALL show a Disconnect button left of the pane-type dropdown, and its location label SHALL include the host. Delete on a remote pane SHALL be labelled "Delete Permanently".

#### Scenario: Disconnect
- **WHEN** the user presses Disconnect
- **THEN** the pane returns to the local home folder

### Requirement: SMB connects through the Network pane
There SHALL be no SMB pane type in the selector. SMB is connected from the Network pane: its header "Connect…" button and File ▸ Connect to SMB Server… (`network.connect.dialog`) open a popup asking for host, optional port, optional share, username, domain, password and "Require encryption". Once connected, the pane SHALL behave as a Files pane on the share with a Disconnect button and "Delete Permanently" instead of "Move to Trash". `PaneKind::Smb` SHALL remain deserialisable only so saved workspaces load; it SHALL restore as a Network pane.

#### Scenario: Connect to Server
- **WHEN** the user presses Connect… in a Network pane and submits a host, user and password
- **THEN** the pane lists the share (or the server's shares when none was given) and shows Disconnect

#### Scenario: Legacy workspace
- **WHEN** a saved workspace contains a pane of kind `Smb`
- **THEN** it restores as a Network pane

### Requirement: Saved servers in the SFTP pane
An SFTP pane without a connection SHALL show `servers:///` as its tree root, listing the saved SFTP profiles as folders; it SHALL NOT show a full-page form. Opening a saved server SHALL connect (silently with a remembered password, otherwise through the credentials popup). The pane header SHALL have a Connect… button that opens a popup to connect to a new server and optionally save it under a name.

#### Scenario: Open a saved server
- **WHEN** the user switches a pane to SFTP and opens `nas` in the tree
- **THEN** the credentials popup appears (or it connects silently) and the pane then lists the server

#### Scenario: Add a server
- **WHEN** the user presses Connect…, fills the popup with a "Save as" name and connects
- **THEN** the pane connects and the server appears under Saved servers on the next visit

### Requirement: Managing saved servers
A context menu on a saved server SHALL offer Connect, Edit and Delete. Edit SHALL reopen the popup prefilled (never with a password). Delete SHALL remove the profile after the existing command rules and SHALL NOT touch an open connection.

#### Scenario: Delete
- **WHEN** the user chooses Delete on `nas`
- **THEN** the profile is removed from the list and from `prefs.toml` on exit

