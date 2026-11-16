## MODIFIED Requirements

### Requirement: Cut, copy and paste
`file.clip_copy` and `file.clip_cut` SHALL remember the focused selection (Files pane or thumbnail grid) and mirror its paths to the OS clipboard as text; `file.clip_paste` SHALL copy (an item already in the destination folder is duplicated as `name (2).ext` via `ops::duplicate`, never skipped; or, after a cut, move and clear the clipboard) the remembered items into the focused folder through the ordinary pipelines with conflict policy `ask`. Paste with `os_text` that differs from what was mirrored SHALL fail with a notice and clear the clipboard. Ctrl/Cmd+X/C/V arrive as egui clipboard events and SHALL be mapped to these commands except while a terminal has focus, text is being edited or a dialog is open. The clipboard belongs to the application, not to a workspace: switching, opening or closing workspace tabs SHALL NOT clear or change it.

#### Scenario: Cut and paste
- **WHEN** a file is cut and pasted into another folder
- **THEN** it exists there, is gone from the source, and the clipboard is empty

#### Scenario: Across workspaces
- **WHEN** a file is copied (or cut) in a pane of one workspace tab, the user switches to another tab and pastes into its focused folder
- **THEN** the result is the same as within one tab: a copy keeps the source and can be pasted again, a cut moves the file once and empties the clipboard, and conflicts ask as usual
- **AND** this holds for local, SMB and SFTP sources and destinations

#### Scenario: Explicit folder-menu destination
- **WHEN** `file.clip_paste` receives an optional `dest` URI from a folder context menu
- **THEN** it pastes into that folder instead of the focused listing's location
- **AND** malformed destination values fail before any work is submitted
