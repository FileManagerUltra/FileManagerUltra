## MODIFIED Requirements

### Requirement: Find Files searches names and content in the background
`search.open {root?}` (Ctrl+F in a Files pane) SHALL open the Find Files window for the focused folder; `search.open_everywhere` (Ctrl+Shift+F) SHALL open it for the whole computer (`/` on Linux and macOS, every drive on Windows). `search.start {name|content, root?, hidden?}` SHALL walk the tree on background threads: `name` is a substring or a `* ? [a-z]` pattern (alternatives split by `;`), `content` is a case-insensitive text match; at least one is required. Hidden entries are skipped unless asked. `search.stop` cancels; closing the window (`search.close`) cancels too. Results stream in while the walk runs.

#### Scenario: Pattern search
- **WHEN** the user searches `/home/alice` for `*.txt; *.bin`
- **THEN** both matching files are listed with size and folder, and the status shows the number of matches and folders/files searched

#### Scenario: Search everywhere
- **WHEN** the user presses Ctrl+Shift+F and searches for `notes.txt`
- **THEN** every local drive is searched, and kernel or system pseudo-folders such as `/proc`, `/sys` and `/dev` are not entered

### Requirement: Search is bounded and safe
The walker SHALL NOT follow symlinks, SHALL stop at 10 000 results or depth 64, SHALL skip files whose first 8 KiB contain a NUL when matching content, and SHALL read at most 64 MiB per file. SMB locations SHALL NOT be searched (they must stay on the SMB lane); local and SFTP SHALL be. Local searches SHALL use at most 8 threads and SHALL NOT read metadata of entries that do not match.

#### Scenario: Symlink loop and binary files
- **WHEN** a tree contains a symlink cycle and binary files and a content search runs
- **THEN** the symlink is not followed and binary files are skipped
- **AND** the search ends once 10 000 results are found

## ADDED Requirements

### Requirement: Searches run in the background like jobs
A running search SHALL be minimisable to the jobs panel (`search.minimize`), where it shows its query, matches found and folders searched, with buttons to show the window again (`search.restore`) and to stop it. The status bar's job text SHALL include a running search. Minimising SHALL NOT stop or slow the search.

#### Scenario: Minimise and come back
- **WHEN** the user minimises a running search, keeps working and then chooses Show in the jobs panel
- **THEN** the search kept running and the window shows all results found so far

### Requirement: Recent searches can be re-used
The last 20 searches (folder, name, text and hidden setting; newest first, without duplicates) SHALL be remembered across restarts and offered in the Find Files window, where choosing one fills the form and `search.history.run {index}` runs it; `search.history.clear` forgets them. Stored values SHALL be capped like search parameters and SHALL never contain credentials.

#### Scenario: Re-run a search
- **WHEN** the user picks yesterday's `*.pdf` search in `/home/alice/Documents` from Recent
- **THEN** the form is filled with it and the search runs again
