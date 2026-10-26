## ADDED Requirements

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
