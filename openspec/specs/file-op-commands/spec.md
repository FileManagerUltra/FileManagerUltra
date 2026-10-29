# file-op-commands Specification

## Purpose
TBD - created by archiving change dnd-and-file-ops. Update Purpose after archive.
## Requirements
### Requirement: File commands are registered
The registry SHALL contain `file.copy` (F5), `file.move` (F6), `file.mkdir` (F7), `file.delete` (F8, Delete), `file.delete_permanent` (Shift+Delete), `file.rename` (F2), `edit.undo` (Ctrl+Z), `job.cancel`, `job.resolve_conflict` and `dnd.drop`, each with id, label, menu path, shortcut, params doc, `enabled` and `run`, and menu/shortcut wiring checked by brand-check.

#### Scenario: Registry completeness
- **WHEN** brand-check cross-checks command ids against the app tables
- **THEN** every new id literal exists in the registry

### Requirement: Other-pane destination
`file.copy`/`file.move` SHALL use the selection of the focused pane and the directory of the most-recently-focused other pane as destination; with one pane they SHALL ask for a destination in a dialog.

#### Scenario: F5 with two panes
- **WHEN** two panes are open and F5 is pressed with a selection
- **THEN** a copy job targets the other pane's directory

#### Scenario: Nothing selected
- **WHEN** nothing is selected
- **THEN** the command is disabled and dispatch is an `Ok` no-op

### Requirement: Commands validate params
Every file command SHALL reject malformed, oversized or deep params with `Err` before doing any work, and the fuzz suite SHALL cover all of them.

#### Scenario: Bad params
- **WHEN** `file.rename` receives a name containing `/`
- **THEN** it returns `Err(InvalidParams)` and nothing changes

### Requirement: Input dialogs
mkdir and rename SHALL use input dialogs (prefilled name for rename) that validate the name live and dispatch the command only on confirm.

#### Scenario: Rename dialog
- **WHEN** F2 is pressed on one selected item
- **THEN** a dialog opens with its name selected and Enter dispatches `file.rename`

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
`file.clip_copy` and `file.clip_cut` SHALL remember the focused selection (Files pane or thumbnail grid) and mirror its paths to the OS clipboard as text; `file.clip_paste` SHALL copy (an item already in the destination folder is duplicated as `name (2).ext` via `ops::duplicate`, never skipped; or, after a cut, move and clear the clipboard) the remembered items into the focused folder through the ordinary pipelines with conflict policy `ask`. Paste with `os_text` that differs from what was mirrored SHALL fail with a notice and clear the clipboard. Ctrl/Cmd+X/C/V arrive as egui clipboard events and SHALL be mapped to these commands except while a terminal has focus, text is being edited or a dialog is open.

#### Scenario: Cut and paste
- **WHEN** a file is cut and pasted into another folder
- **THEN** it exists there, is gone from the source, and the clipboard is empty

#### Scenario: Explicit folder-menu destination
- **WHEN** `file.clip_paste` receives an optional `dest` URI from a folder context menu
- **THEN** it pastes into that folder instead of the focused listing's location
- **AND** malformed destination values fail before any work is submitted

### Requirement: Navigation from a preview targets its Files pane
`nav.open` dispatched while a Preview pane is focused (e.g. double-clicking a folder thumbnail) SHALL navigate the Files pane the preview shows.

#### Scenario: Folder thumbnail
- **WHEN** a folder thumbnail is double-clicked in a focused grid
- **THEN** the connected Files pane navigates into that folder


### Requirement: Everyday item commands
The context menu and File/Edit menus SHALL offer: `file.open_with.dialog` (Open With…: installed applications, those registered for the file type first and the default marked — XDG desktop entries on Linux, `/Applications` on macOS, the system dialog on Windows; local files only, launched without a shell), `file.duplicate` (Ctrl+Shift+D, a "copy" next to the original), `file.copy_path` / `file.copy_name` (to the clipboard), `bookmark.add` and `file.properties` (Alt+Enter). Properties SHALL show name, location, kind, modified time and a live size/count computed on a background walk that stops when the window closes; for one item on a backend that supports it, permissions are editable (rwx grid plus setuid/setgid/sticky) and applied with `file.chmod {mode, sources?}` (local unix and SFTP; symlinks are refused).

#### Scenario: Bad chmod params
- **WHEN** `file.chmod` is dispatched with mode `"9999"` or no mode
- **THEN** it returns an error and no job is submitted
