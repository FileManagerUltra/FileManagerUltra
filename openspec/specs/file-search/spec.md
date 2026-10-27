# file-search Specification

## Purpose
Find files below a folder by name and/or content without blocking the UI.

## Requirements
### Requirement: Find Files searches names and content in the background
`search.open {root?}` (Alt+F7) SHALL open the Find Files window for the focused folder. `search.start {name|content, root?, hidden?}` SHALL walk the tree on a background thread: `name` is a substring or a `* ? [a-z]` pattern (alternatives split by `;`), `content` is a case-insensitive text match; at least one is required. Hidden entries are skipped unless asked. `search.stop` cancels; closing the window (`search.close`) cancels too. Results stream in while the walk runs.

#### Scenario: Pattern search
- **WHEN** the user searches `/home/alice` for `*.txt; *.bin`
- **THEN** both matching files are listed with size and folder, and the status shows the number of matches and folders/files searched

### Requirement: Search is bounded and safe
The walker SHALL NOT follow symlinks, SHALL stop at 10 000 results or depth 64, SHALL skip files whose first 8 KiB contain a NUL when matching content, and SHALL read at most 64 MiB per file. SMB locations are not searched (they must stay on the SMB lane); local and SFTP are.

### Requirement: Results lead back to the file
Double-clicking a file result opens it with its default application; Show in Folder (`file.reveal {uri}`) navigates a Files pane to the parent and selects the item; Copy Path copies its path.
