# path-picker Specification

## Purpose
A non-blocking picker window, driven by commands, for choosing a folder or file on any backend on behalf of another command or dialog.
## Requirements
### Requirement: Path picker
The app SHALL provide a picker window, driven only by `picker.*` commands, that lists a folder of any backend without blocking the UI, lets the user go up, type a location, enter folders and select an entry, and returns the chosen folder or file to the requester identified by a purpose string.

#### Scenario: Choosing a folder
- **WHEN** `picker.open` is dispatched with kind `folder` and the user presses "Choose this folder"
- **THEN** the selected folder (or the folder being viewed when none is selected) is returned and the picker closes

#### Scenario: Choosing a file
- **WHEN** kind is `file` and the user selects a file and presses "Choose file"
- **THEN** that file is returned; accepting with no file or with a folder selected is refused with an actionable message

#### Scenario: Unreadable or vanished folder
- **WHEN** the listed folder cannot be read
- **THEN** the message is shown in the picker and the user can still go up or type another location

#### Scenario: Hostile params
- **WHEN** any `picker.*` command gets malformed params
- **THEN** it returns an error and never panics

#### Scenario: Sync folders
- **WHEN** the picker is opened for the purpose `sync.left` or `sync.right` and accepted
- **THEN** the engine sets that folder on the open sync profile

### Requirement: Places sidebar
The picker SHALL show a sidebar with Places, Bookmarks, Devices, Computer (File system / This PC) and Network (Network, Saved servers, each saved SFTP server). Empty sections are hidden.

#### Scenario: Network is browsable but not choosable
- **WHEN** the user opens Network or Saved servers
- **THEN** the entries are listed and "Choose folder" is disabled there

#### Scenario: Unconnected server
- **WHEN** the user opens a saved SFTP/SMB server that is not connected
- **THEN** the sign-in popup opens over the picker and, once connected, the picker lists the server

