## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Opening an unconnected SMB location signs in
`nav.open` on an `smb://` location without a connection SHALL start the sign-in (straight away with a remembered login, otherwise the Authentication Required popup) instead of failing, keep the pane where it is while connecting, and open the location once connected. Cancelling the sign-in SHALL leave the pane unchanged.

#### Scenario: Bookmark on a disconnected share
- **WHEN** the user opens a bookmark for `smb://192.168.0.50/media` with no connection to that server
- **THEN** the Authentication Required popup opens for `192.168.0.50`, no "no backend" error is shown, and after signing in the pane shows `media`
