## ADDED Requirements

### Requirement: SMB connects through the Network pane
There SHALL be no SMB pane type in the selector. SMB is connected from the Network pane: its header "Connect…" button and File ▸ Connect to SMB Server… (`network.connect.dialog`) open a popup asking for host, optional port, optional share, username, domain, password and "Require encryption". Once connected, the pane SHALL behave as a Files pane on the share with a Disconnect button and "Delete Permanently" instead of "Move to Trash". `PaneKind::Smb` SHALL remain deserialisable only so saved workspaces load; it SHALL restore as a Network pane.

#### Scenario: Connect to Server
- **WHEN** the user presses Connect… in a Network pane and submits a host, user and password
- **THEN** the pane lists the share (or the server's shares when none was given) and shows Disconnect

#### Scenario: Legacy workspace
- **WHEN** a saved workspace contains a pane of kind `Smb`
- **THEN** it restores as a Network pane
