## ADDED Requirements

### Requirement: Properties counts folders on every backend
Properties SHALL compute the size and item count of folders on local volumes, SFTP servers and SMB shares with the same background walk and caps (no symlinks, 5 M entries or depth 256, stops when the window closes). Locations without folder contents to count (the Network and Servers folders, an SMB server root) SHALL show the cached listing only, with a note saying so.

#### Scenario: Folder on an SMB share
- **WHEN** the user opens Properties on `smb://nas/media/Movies`
- **THEN** the size and the number of files and folders fill in as the walk runs, without the "not counted" note
