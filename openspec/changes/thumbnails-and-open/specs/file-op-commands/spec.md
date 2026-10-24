## ADDED Requirements

### Requirement: File activation is a command
Double-clicking or pressing Enter on an entry SHALL resolve to an activation plan and dispatch engine commands only: a directory runs `nav.open`, a document runs `file.open {uri, mode: "default", confirm: true}`, and a program asks first. `file.open` SHALL validate params (`uri` string, `mode` one of `default`/`run`, `confirm` boolean) before touching the filesystem and SHALL reject special files.

#### Scenario: Open a document
- **WHEN** the user activates `notes.pdf` in a Files pane
- **THEN** `file.open` hands the local path to the system default application and the pane's location and selection are unchanged

#### Scenario: Activate a folder
- **WHEN** the user activates a folder in the listing or the tree
- **THEN** the pane navigates to it (`nav.open`), exactly as before thumbnails and activation landed

#### Scenario: Bad params
- **WHEN** `file.open` receives a missing/invalid `uri`, an unknown `mode`, or a non-boolean `confirm`
- **THEN** it returns `Err(InvalidParams)` and starts nothing

### Requirement: Programs need explicit confirmation
An entry classified as a program SHALL NOT start unless the caller passes `mode: "run", confirm: true`; the UI SHALL first ask *Run* / *Open as document* / *Cancel*. `confirm: true` with `mode: "default"` hands the file to the OS handler as a document, which can itself start it — so it is refused unless `confirm: true` is also present. Classification uses the executable bit plus a runnable extension on Unix (removable/network drives mark everything executable, so the bit alone is not enough) and a runnable-extension list on Windows; a macOS `.app` bundle counts as a program.

#### Scenario: A script
- **WHEN** the user activates an executable shell script
- **THEN** a dialog appears with *Run*, *Open as document* and *Cancel*; nothing starts until one is chosen

#### Scenario: Unconfirmed run
- **WHEN** `file.open` is dispatched with `mode: "run"` and no `confirm: true`
- **THEN** it returns `Err(InvalidParams)` and the process is not spawned

### Requirement: Activation is local-only and shell-free
`file.open` SHALL refuse non-`file://` uris with an actionable message ("copy it to a local folder first"), require an absolute local path, and never pass the path through a shell: a run uses a direct process spawn with the file as the program (its own process group, stdin/stdout/stderr null, current directory = its parent), a document hand-off uses the OS opener API. Neither waits for the child; quitting the file manager SHALL NOT kill a program it started.

#### Scenario: Remote document
- **WHEN** `file.open` receives an `sftp://` uri
- **THEN** it returns `Err(Unsupported)` with a copy-first hint and starts nothing

#### Scenario: Hostile name
- **WHEN** a program is named `; rm -rf ~` or `-i evil`
- **THEN** it is spawned as a single absolute path argument, no shell and no expansion is involved


### Requirement: Cut, copy and paste
`file.clip_copy` and `file.clip_cut` SHALL remember the focused selection (Files pane or thumbnail grid) and mirror its paths to the OS clipboard as text; `file.clip_paste` SHALL copy (or, after a cut, move and clear the clipboard) the remembered items into the focused folder through the ordinary pipelines with conflict policy `ask`. Paste with `os_text` that differs from what was mirrored SHALL fail with a notice and clear the clipboard. Ctrl/Cmd+X/C/V arrive as egui clipboard events and SHALL be mapped to these commands except while a terminal has focus, text is being edited or a dialog is open.

#### Scenario: Cut and paste
- **WHEN** a file is cut and pasted into another folder
- **THEN** it exists there, is gone from the source, and the clipboard is empty

### Requirement: Navigation from a preview targets its Files pane
`nav.open` dispatched while a Preview pane is focused (e.g. double-clicking a folder thumbnail) SHALL navigate the Files pane the preview shows.

#### Scenario: Folder thumbnail
- **WHEN** a folder thumbnail is double-clicked in a focused grid
- **THEN** the connected Files pane navigates into that folder
