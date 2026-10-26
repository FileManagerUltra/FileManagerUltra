## ADDED Requirements

### Requirement: Network location
`network:///` SHALL be a read-only virtual folder listing hosts discovered on the local network by mDNS/DNS-SD and WS-Discovery, each as a folder named `<host> (File Sharing)` (SMB) or `<host> (Remote Login)` (SFTP). Discovery SHALL run in the background and never block the UI; names from the network SHALL be length-capped and stripped of control characters, and the list SHALL be capped.

#### Scenario: Hosts appear
- **WHEN** a NAS advertises `_smb._tcp` and the user opens Network
- **THEN** `nas (File Sharing)` is listed and opening it browses `smb://nas/`

#### Scenario: Nothing found
- **WHEN** multicast is blocked
- **THEN** the Network folder is empty and Connect… still opens the manual SMB popup

### Requirement: Server share listing
`smb://host/` on a connected server SHALL list the server's disk shares, hiding administrative (`$`) shares; when the server refuses enumeration it SHALL list the shares connected so far.

#### Scenario: Browse shares
- **WHEN** the user opens a connected server
- **THEN** its disk shares appear as folders and opening one connects it on demand

### Requirement: Authentication popup
Opening an unconnected `smb://` host SHALL show a modal Authentication Required dialog (username, domain, password, remember choice) instead of an error. Remember choices SHALL be "Forget password immediately" and "Remember password until quit"; passwords SHALL never be written to disk. Cancel SHALL leave the pane unchanged; a refused login SHALL show the dialog again with the error.

#### Scenario: Expand a host in the tree
- **WHEN** the user expands `nas` under Network
- **THEN** the dialog asks for the login and, on success, the tree shows the shares

#### Scenario: Remembered login
- **WHEN** the user chose "Remember password until quit" and opens the same host again
- **THEN** it connects without asking

#### Scenario: No forever option
- **WHEN** the dialog is shown
- **THEN** no choice persists the password across restarts

### Requirement: Connect to a server by name
The Network pane header SHALL have a "Connect…" button, and File ▸ Connect to SMB Server… SHALL exist (`network.connect.dialog`). Both open the popup with extra Host, Port, Share and "Require encryption" fields; a bad host, share or port SHALL be reported without losing the popup.

#### Scenario: Server not discovered
- **WHEN** the user presses Connect…, enters `nas.example` and credentials
- **THEN** the pane connects and lists the server's shares

### Requirement: Live refresh and connection indicator
While a Network pane is visible, the app SHALL re-read `network:///` periodically and repaint without user input so newly discovered hosts appear on their own. Hosts with a ready connection SHALL show a success-coloured dot after their name in the tree.

#### Scenario: Host appears later
- **WHEN** a host is discovered after the folder was first listed
- **THEN** it appears without any mouse or keyboard input

### Requirement: Remote Login hosts
Opening a `(Remote Login)` host from a non-SFTP pane SHALL show a short popup offering "Open SFTP Pane" (Enter) or Cancel (Esc); it SHALL NOT switch the pane silently.

#### Scenario: Open SFTP host from Network
- **WHEN** the user opens `nas (Remote Login)`
- **THEN** the popup appears and, on Open SFTP Pane, the pane becomes an SFTP pane for that host

### Requirement: Discovery survives multiple networks
WS-Discovery probes SHALL be sent from every local IPv4 interface (at most 16) and repeated each second during the 4 s listen window, so a VPN default route does not hide LAN hosts. A host that answers with only an address SHALL be named by a NetBIOS node-status lookup (UDP 137, 400 ms timeout, reply accepted only from the queried address); without an answer it keeps its address.

#### Scenario: Windows PC behind a VPN default route
- **WHEN** the default route is a VPN and a Windows PC answers WS-Discovery
- **THEN** it appears in Network, under its machine name when NetBIOS answers

### Requirement: Local panes refresh in the background
Every visible local Files pane SHALL re-read its folder about every 2 seconds, focused or not, until a file watcher replaces the poll.

#### Scenario: File added behind the app's back
- **WHEN** another program creates a file in a folder shown in an unfocused pane
- **THEN** the file appears within a few seconds
