## ADDED Requirements

### Requirement: One display form per location, chosen by the OS
Every location SHALL have exactly one display form, decided by the OS the app runs on:

| Location | Windows | Linux and macOS |
|---|---|---|
| Local folder | `C:\Users\alice`; a native share `\\server\share\dir` | `/home/alice` |
| Local root | `This PC` | `/` |
| SMB through the app's connection | `\\host\share\dir`, server root `\\host` | `smb://host/share/dir` |
| SMB with a user or non-default port | `smb://alice@host:1445/share/dir` | same |
| SFTP | `sftp://user@host/dir` (`:port` when not 22) | same |
| Network, Servers | `network:///`, `servers:///` | same |

#### Scenario: Local path on Windows
- **WHEN** the app runs on Windows and a pane shows `file:///C:/Users/alice`
- **THEN** its display form is `C:\Users\alice`

#### Scenario: SMB share on Windows
- **WHEN** the app runs on Windows and a pane shows `smb://192.168.0.50/media/Movies`
- **THEN** its display form is `\\192.168.0.50\media\Movies`

#### Scenario: SMB share on Linux
- **WHEN** the app runs on Linux and a pane shows `smb://192.168.0.50/media/Movies`
- **THEN** its display form is `smb://192.168.0.50/media/Movies`

#### Scenario: SMB address UNC cannot express
- **WHEN** the app runs on Windows and a pane shows `smb://alice@nas/media`
- **THEN** its display form is `smb://alice@nas/media`

### Requirement: Every shown location uses the display form
The location bar, the location editor's starting text and suggestions, recent locations, the Find Files folder field, result folders, status line and recent searches, bookmark default names and Places/tree/listing/preview hover texts, Properties, Copy Path and device labels SHALL show locations in their display form. Canonical uris (`file://`, `smb://` on Windows) SHALL NOT appear in these places except where the table above says so.

#### Scenario: Copy Path of an SMB file on Windows
- **WHEN** the user copies the path of `smb://nas/media/a.mkv` on Windows
- **THEN** the clipboard holds `\\nas\media\a.mkv`

### Requirement: Display forms can be typed back
Wherever an address is typed (location bar, Find Files folder field) every display form SHALL be accepted and SHALL lead to the location it was shown for. On Windows a typed `\\host\share\...` SHALL open through the app's own SMB connection when one to that host is open (or the pane is an SMB or Network pane), and through Windows' own access otherwise. A recent location picked unchanged from the suggestions SHALL return to the location it was recorded from, whatever is connected now.

#### Scenario: Paste a copied SMB path on Windows
- **WHEN** the app is connected to `nas` and the user types `\\nas\media\Movies` in the location bar on Windows
- **THEN** the pane opens `smb://nas/media/Movies` through that connection

#### Scenario: UNC path without an app connection on Windows
- **WHEN** the app has no connection to `fileserver` and the user types `\\fileserver\public` on Windows
- **THEN** the pane opens the share through Windows' own access, as before

#### Scenario: Recent SMB location while disconnected
- **WHEN** the user picks a recent location recorded from `smb://nas/media` after disconnecting
- **THEN** the app signs in to `nas` and opens `smb://nas/media`

### Requirement: Stored addresses stay canonical
Bookmarks, workspaces, recent searches and recent locations SHALL store canonical uris, never display forms, so the same settings files work on every OS.

#### Scenario: Bookmark saved on Windows
- **WHEN** the user bookmarks the folder shown as `\\nas\media` on Windows
- **THEN** the prefs file stores `smb://nas/media`
