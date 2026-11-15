## ADDED Requirements

### Requirement: Bookmark names and server bookmarks
A new bookmark's default name SHALL be the folder's name, or for a root (a drive, an SMB server or share root, an SFTP server root) its display form (see `path-display`). A saved bookmark whose name is still its own canonical uri (made before this rule) SHALL be shown with the default name; renaming it stores the new name. Opening a bookmark on an SMB or SFTP server without a connection SHALL start the sign-in and open the bookmark once connected.

#### Scenario: Bookmark of an SMB server root on Windows
- **WHEN** the user bookmarks `smb://192.168.0.50/` on Windows
- **THEN** the bookmark is named `\\192.168.0.50` and the prefs file stores `smb://192.168.0.50/`

#### Scenario: Old raw-address name
- **WHEN** the prefs file holds a bookmark named `smb://192.168.0.50/` for `smb://192.168.0.50/`
- **THEN** Places shows it as `\\192.168.0.50` on Windows and `smb://192.168.0.50/` elsewhere

#### Scenario: Open a server bookmark while disconnected
- **WHEN** the user clicks a bookmark for `smb://nas/media` with no connection to `nas`
- **THEN** the sign-in starts and the pane opens `smb://nas/media` once connected
