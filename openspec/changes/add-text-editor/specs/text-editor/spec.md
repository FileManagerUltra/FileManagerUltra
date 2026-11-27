## ADDED Requirements

### Requirement: Editing is a mode of the Preview pane
A Preview pane SHALL have an Edit mode in which the previewed text file is shown in an editable view. The pane kind SHALL remain `Preview`; no pane type is added. `editor.toggle` (F4, View menu, and a pencil button in the Preview toolbar) SHALL switch the focused Preview pane between the rendered preview and Edit mode for the same file, keeping the scroll position as close as possible. Leaving Edit mode SHALL first save a dirty buffer (autosave on) or show the unsaved-changes warning (autosave off), then show the preview of the saved file. Toolbar and header SHALL show the mode and an unsaved-changes marker while the buffer is dirty. Files that cannot be edited (images, media, PDFs, archives, directories) SHALL NOT offer Edit mode.

#### Scenario: Toggle into edit
- **WHEN** a Preview pane shows `notes.md` and the user presses F4
- **THEN** the pane shows the editable Markdown text with the caret at the first visible line and the header shows "Editing"

#### Scenario: Back to preview saves first
- **WHEN** the user edits in Edit mode with autosave on and toggles back to the rendered view
- **THEN** the file is saved and the preview shows the saved text

#### Scenario: Not editable
- **WHEN** the Preview pane shows a PNG
- **THEN** `editor.toggle` returns an error saying the file type cannot be edited and the pane is unchanged

### Requirement: Editing basics
Edit mode SHALL support typing, deletion, selection by keyboard and pointer, cut/copy/paste, select all, undo/redo with coalesced typing runs, auto-indent on newline, indent/outdent of the selection with Tab/Shift+Tab, duplicate line, delete line, move line up/down, toggle line comment where the language defines one, line numbers (toggleable), optional soft wrap, bracket matching (highlight of the partner of the bracket at the caret) and drag-and-drop-free text input through the platform IME. All positions SHALL be bounds-checked; an out-of-range edit SHALL return an error and leave the buffer unchanged.

#### Scenario: Undo history depth
- **WHEN** the undo depth is set to 50 and the user makes 80 separate edits
- **THEN** only the last 50 can be undone, and the editor notes that older history was dropped

#### Scenario: Undo a typing run
- **WHEN** the user types a word and presses Ctrl+Z once
- **THEN** the whole word is removed, and Ctrl+Y restores it

#### Scenario: Auto-indent
- **WHEN** the caret is after `    if x:` (4 spaces) and the user presses Enter in a Python file
- **THEN** the new line starts indented by 8 spaces

#### Scenario: Bracket match
- **WHEN** the caret is next to `(` in `f(a, b)`
- **THEN** both parentheses are highlighted

### Requirement: Undo history depth is configurable
The number of undo steps kept per open file SHALL be a preference, `prefs.editor_undo_steps {steps}` and Preferences → Editor, with a default of 1000 and an accepted range of 10 to 100000. A step is one undo group (a coalesced typing run, one replace-all, one line move). Changing the preference SHALL apply at once to open editors, dropping the oldest steps when the new depth is smaller. Independently of the step count, the history SHALL be capped at 64 MiB of stored text per file, dropping the oldest steps first, so a huge replace cannot exhaust memory. Redo history SHALL be cleared by any new edit.

#### Scenario: Lower the depth
- **WHEN** an editor holds 800 undo steps and the depth is set to 100
- **THEN** the oldest 700 steps are dropped and 100 undos remain

#### Scenario: Out-of-range value
- **WHEN** `prefs.editor_undo_steps` receives `5`, `1000000`, a string or no `steps`
- **THEN** it returns `Err(InvalidParams)` and the preference is unchanged

#### Scenario: Memory cap
- **WHEN** a replace-all rewrites 100 MB of text in an editor with a 64 MiB history cap
- **THEN** the edit succeeds, the oldest history is dropped to stay under the cap and the buffer stays correct

### Requirement: Jump to line
`editor.goto_line {line, column?}` (the prompt opens with Ctrl+G, `editor.goto_prompt`) SHALL move the caret to the 1-based line and optional column, scrolling it into view, and SHALL accept input like `120` or `120:8` in a small inline prompt. A line beyond the end of the file SHALL go to the last line; zero, negative or non-numeric input SHALL return `Err(InvalidParams)`.

#### Scenario: Go to line
- **WHEN** the user presses Ctrl+G, types `1200` and Enter in a 5000-line file
- **THEN** line 1200 is visible and the caret is on it

#### Scenario: Past the end
- **WHEN** `editor.goto_line {line: 99999999}` runs in a 10-line file
- **THEN** the caret is on line 10

### Requirement: Find and replace
Edit mode SHALL provide find and replace (Ctrl+F; replace is a toggle in the find bar, because Ctrl+H already shows hidden files everywhere) with next/previous, case-sensitive, whole-word and regular-expression options, a match count and replace / replace all as a single undo step. Searching SHALL run off the UI thread on files larger than 1 MiB, be cancellable, and a regular expression SHALL be compiled with a size limit and run with a time budget; an invalid or over-limit pattern SHALL report an error without changing the buffer.

#### Scenario: Replace all
- **WHEN** the user replaces `foo` with `bar` for 300 matches and presses Ctrl+Z
- **THEN** all 300 replacements are undone together

#### Scenario: Bad regular expression
- **WHEN** the pattern `(` is entered with regex enabled
- **THEN** an error is shown, no search runs and the buffer is unchanged

### Requirement: Syntax colouring
Edit mode SHALL colour source text with tree-sitter for the supported languages (Rust, TOML, JSON, YAML, Markdown, shell, Python, JavaScript/TypeScript, C/C++, Go, Java, HTML/CSS, XML, SQL, PHP), chosen by file extension, then well-known file name, then a shebang line. Parsing SHALL be incremental, run off the UI thread, and highlight only the visible range plus a margin. Colours SHALL come from semantic theme tokens defined for both dark and light themes. Files larger than `syntax_max_bytes`, files whose parse exceeds its time budget, and unknown languages SHALL be shown as plain text with a short notice that colouring is off, and editing SHALL remain fully usable.

#### Scenario: Colours follow the theme
- **WHEN** the theme switches between dark and light with a Rust file open
- **THEN** keyword, string, comment, number and type colours change to that theme's tokens

#### Scenario: Large file degrades gracefully
- **WHEN** a 200 MB log opens in Edit mode
- **THEN** it opens as plain text with the notice "Syntax colouring is off for large files" and scrolling stays smooth

#### Scenario: Typing stays responsive
- **WHEN** the user types in a 5 MB Rust file
- **THEN** no frame waits on parsing; colours catch up when the worker result arrives

### Requirement: Large files stay smooth
The editor SHALL render and scroll files up to the 1 GiB edit limit without laying out more than the visible lines plus a small margin, without a per-frame cost that grows with file size, and with a load that streams in chunks, shows the first screen as soon as it is available, and can be cancelled. While loading the buffer SHALL be read-only. Lines longer than a fixed threshold SHALL be rendered as a clipped window around the viewport. Files above 1 GiB SHALL open read-only in the existing preview with the external editor offered.

#### Scenario: Open a big file
- **WHEN** a 500 MB text file is opened for editing
- **THEN** the first lines appear before loading finishes, a progress indicator shows the rest, and editing is enabled when it completes

#### Scenario: Over the limit
- **WHEN** a 2 GB file is opened with F4
- **THEN** it stays in preview mode and a message offers *Open in external editor*

#### Scenario: Very long line
- **WHEN** a file has one 50 MB line
- **THEN** it can be scrolled and edited without freezing the UI

### Requirement: Encoding and line endings are preserved
On load the editor SHALL detect UTF-8 (with or without BOM), UTF-16 LE/BE (with BOM) and fall back to Latin-1 for invalid UTF-8, and detect LF, CRLF or mixed line endings. Saving SHALL write the same encoding, BOM and dominant line ending unless the user changes them with `editor.set_encoding`/`editor.set_eol`. Content that looks binary (a NUL byte in the first 64 KiB for an 8-bit encoding) SHALL open read-only with the external editor offered.

#### Scenario: CRLF round trip
- **WHEN** a CRLF file is opened, one line edited and saved
- **THEN** every line ending in the saved file is still CRLF

#### Scenario: Binary file
- **WHEN** a file containing NUL bytes is opened with F4
- **THEN** it is not editable and a message offers the external editor

### Requirement: Local and remote files are edited identically
Edit mode SHALL load and save through `Vfs`, so local, SMB, SFTP and archive-free remote files behave the same. Load and save SHALL run as jobs on the file's lane with progress and cancel, never on the UI thread. Save SHALL write a temporary sibling and rename it over the target when the backend supports rename-over, and write in place otherwise; permissions SHALL be preserved on local files. A save failure (permission, full disk, dead share) SHALL keep the buffer dirty and report an actionable error. Files inside archives SHALL open read-only.

#### Scenario: Save to SFTP
- **WHEN** a file on an SFTP pane is edited and saved
- **THEN** the content is written through the SFTP lane, the dirty marker clears and the UI stays responsive during the transfer

#### Scenario: Full disk
- **WHEN** a save fails because the volume is full
- **THEN** the buffer stays dirty, the original file is intact and an error explains what failed

#### Scenario: Archive member
- **WHEN** F4 is pressed on a file inside a zip
- **THEN** it opens read-only with a note that archive members cannot be edited

### Requirement: Changes on disk never get overwritten silently
The editor SHALL remember the file's modification time and size from load or last save. Before every save, including autosave, it SHALL compare them with the file's current state. When they differ it SHALL NOT write, and SHALL ask: Reload (discard edits), Overwrite, or Save as. When the file changes on disk while the buffer is clean, it SHALL reload in place keeping caret and scroll; while dirty, a non-modal bar SHALL say the file changed on disk and offer Reload and Keep mine (Keep mine makes the next save replace the file without asking again).

#### Scenario: Autosave meets an external change
- **WHEN** another program modifies the file after it was opened, the user edits, and focus leaves the editor
- **THEN** autosave does not write and the conflict prompt appears

#### Scenario: Clean buffer reloads
- **WHEN** the file is changed by another program while the buffer has no unsaved edits
- **THEN** the buffer reloads silently and the caret stays on the same line

### Requirement: Autosave on focus loss
By default the editor SHALL save a dirty buffer automatically when it loses focus: when focus moves to another pane, when the application window loses focus, when the pane's file changes (navigation, source follow, opening another file in the pane), when the pane closes, and when the application exits. `prefs.editor_autosave {enabled}` and Preferences → Editor SHALL turn this off. When it is off, any of those triggers with a dirty buffer SHALL show a warning with Save, Discard and Cancel, whose text mentions that *Autosave on focus loss* can be enabled in Preferences; switching to another application is the exception (no warning: it is not leaving the editor). A failed autosave SHALL leave the buffer dirty and report the error, and SHALL NOT retry in a tight loop.

#### Scenario: Focus moves to the Files pane
- **WHEN** autosave is on, the buffer is dirty and the user clicks a Files pane
- **THEN** the file is saved and the dirty marker clears

#### Scenario: Autosave off
- **WHEN** autosave is off, the buffer is dirty and the user closes the pane
- **THEN** a warning offers Save, Discard and Cancel and mentions the autosave preference; Cancel keeps the pane open

#### Scenario: Window blur
- **WHEN** autosave is on, the buffer is dirty and the user switches to another application
- **THEN** the file is saved

#### Scenario: Exit with unsaved edits
- **WHEN** autosave is off and the app is closed with a dirty buffer
- **THEN** the close guard shows the same warning before quitting

### Requirement: Editor commands
`editor.toggle`, `editor.open`, `editor.save`, `editor.save_as`, `editor.revert`, `editor.goto_prompt`, `editor.goto_line`, `editor.find`, `editor.find_next`, `editor.find_previous`, `editor.find_close`, `editor.replace`, `editor.undo`, `editor.redo`, `editor.input`, `editor.set_encoding`, `editor.set_eol`, `editor.toggle_wrap`, `editor.toggle_line_numbers`, `editor.open_external`, `editor.window_blur` and `editor.resolve` SHALL be commands (the `editor.` prefix keeps them apart from the file commands `edit.undo` and `edit.select_all`) with ids, labels, menu paths, shortcuts, `enabled` and bad-parameter tests. They SHALL be no-ops (disabled) unless the focused pane is a Preview in Edit mode, except `editor.toggle`, `editor.open`, `editor.open_external`, `editor.window_blur` and `editor.resolve`. Malformed params SHALL return `Err(InvalidParams)` and never panic.

#### Scenario: Bad params
- **WHEN** `editor.goto_line` receives a string, a missing line or a non-object
- **THEN** it returns `Err(InvalidParams)` and the caret does not move

#### Scenario: Wrong pane
- **WHEN** `editor.save` runs while a Files pane is focused
- **THEN** it is disabled and nothing happens
