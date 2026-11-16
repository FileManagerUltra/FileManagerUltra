## ADDED Requirements

### Requirement: The search folder is shown and typed in the system's form
The Find Files folder field SHALL show the search folder in its display form (see `path-display`): Search everywhere shows `This PC` on Windows and `/` on Linux and macOS. `search.start {root}` SHALL accept the same absolute forms as the location bar (display forms, `This PC`, a bare drive such as `D:`, `~`, UNC paths, `smb://` and `sftp://` addresses) and reject relative text with an error.

#### Scenario: Search everywhere on Windows
- **WHEN** the user presses Ctrl+Shift+F on Windows
- **THEN** the folder field reads `This PC`, not `file:///`

#### Scenario: Folder field on Windows
- **WHEN** the user opens Find Files in `C:\Users\alice` on Windows
- **THEN** the folder field reads `C:\Users\alice`, and typing `D:` searches the D: drive

#### Scenario: Relative folder text
- **WHEN** `search.start` gets `{"root": "Documents", "name": "x"}`
- **THEN** it returns an error and no search starts
