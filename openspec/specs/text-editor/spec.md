# text-editor Specification

## Purpose
TBD - created by archiving change add-text-editor. Update Purpose after archive.
## Requirements
### Requirement: Editing is a mode of the Preview pane
A Preview pane SHALL have an Edit mode in which the previewed text file is shown in an editable view. The pane kind SHALL remain `Preview`; no pane type is added. `editor.toggle` (F4, View menu, and a pencil button in the Preview toolbar) SHALL switch the focused Preview pane between the rendered preview and Edit mode for the same file, keeping the scroll position as close as possible. Leaving Edit mode SHALL first save a dirty buffer (autosave on) or show the unsaved-changes warning (autosave off), then show the preview of the saved file. Toolbar and header SHALL show the mode and an unsaved-changes marker while the buffer is dirty. Edit mode SHALL look unmistakably different from the read-only preview: while the editor takes typing, the pane SHALL be framed in a dedicated editing colour (distinct from the focus accent and from warnings, in dark, light and high-contrast schemes), the header SHALL show a filled *Editing* badge with a pencil in that colour, and the status bar SHALL carry an edge in it. A read-only or blocked editor SHALL show *Viewing* without the frame. Files that cannot be edited (images, media, PDFs, archives, directories) SHALL NOT offer Edit mode.

#### Scenario: Toggle into edit
- **WHEN** a Preview pane shows `notes.md` and the user presses F4
- **THEN** the pane shows the editable Markdown text with the caret at the first visible line, framed in the editing colour, and the header shows the "Editing" badge

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

### Requirement: Code folding
Edit mode SHALL show a fold gutter next to the line numbers with a chevron on each line that starts a foldable range. Ranges SHALL come from the syntax tree for supported languages (blocks, functions, classes, objects/arrays, Markdown sections, markup elements, block comments) and from indentation for other text, and SHALL span at least two lines. Clicking a chevron, `editor.fold` / `editor.unfold` (Ctrl+Shift+[ / ]) at the caret, `editor.toggle_fold`, `editor.fold_all` and `editor.unfold_all` SHALL change fold state. A folded range SHALL show its first line followed by a placeholder like `… 12 lines`, and line numbers SHALL stay those of the buffer. Moving the caret into, searching into, or jumping to a line inside a folded range SHALL unfold it; editing inside it SHALL unfold it; edits above it SHALL keep it folded on the same text. Copying a selection that spans a folded range SHALL include the hidden text. Folding SHALL cost per frame in proportion to the visible lines only; fold state SHALL be kept in the pane's view state (workspace snapshot) as start lines and checked against the file when it loads. When no syntax tree is available because of the large-file rules, indentation folding SHALL still work, and for files over the editor's large-file threshold fold ranges SHALL be computed only for the visible window.

#### Scenario: Fold a function
- **WHEN** the caret is on `fn main() {` in a Rust file and the user presses Ctrl+Shift+[
- **THEN** the function body collapses to `fn main() { … 12 lines`, and line numbers below it are unchanged

#### Scenario: Jump into a fold
- **WHEN** Find selects a match inside a folded region
- **THEN** the region unfolds and the match is visible

#### Scenario: Edits shift folds
- **WHEN** a line is inserted above a folded region
- **THEN** the region stays folded and still hides the same text

#### Scenario: Indentation fallback
- **WHEN** an unknown-language `.conf` file with indented sections is opened
- **THEN** each section header offers a fold chevron

#### Scenario: Copy across a fold
- **WHEN** the user selects from before a folded range to after it and copies
- **THEN** the clipboard contains the folded lines too

#### Scenario: Bad params
- **WHEN** `editor.fold` receives a non-object, a negative line or a non-numeric line
- **THEN** it returns `Err(InvalidParams)` and fold state is unchanged

### Requirement: Rainbow brackets
Edit mode SHALL colour matching bracket pairs `()`, `[]`, `{}` (and `<>` where the grammar treats them as brackets) by nesting depth from a cyclic palette of semantic theme tokens that exist in both the dark and the light theme. Brackets inside strings and comments SHALL NOT be coloured or counted. An unmatched bracket SHALL use a distinct error token. The feature SHALL be on by default, switchable with `prefs.editor_rainbow {enabled}` and in Preferences → Editor, and SHALL apply only while a syntax tree is available; otherwise brackets keep the plain colours and the existing bracket-match highlight still works. Colouring SHALL cover the visible lines and a margin, with per-line cost independent of file size.

#### Scenario: Nested brackets
- **WHEN** `f(a[b{c}])` is shown with rainbow on
- **THEN** the three pairs have three different colours and each pair shares one colour

#### Scenario: Brackets in strings
- **WHEN** a line is `let s = "(not a bracket";`
- **THEN** the parenthesis in the string keeps the string colour and does not shift the depth of later brackets

#### Scenario: Switch off
- **WHEN** the user turns rainbow brackets off in Preferences
- **THEN** brackets use the normal punctuation colour in every open editor

#### Scenario: Large file
- **WHEN** a file above the syntax threshold is open
- **THEN** rainbow brackets are off and a notice says colouring is limited for large files

#### Scenario: Bad params
- **WHEN** `prefs.editor_rainbow` receives a missing or non-boolean `enabled`
- **THEN** it returns `Err(InvalidParams)` and the preference is unchanged

### Requirement: Live preview source
`editor.open_live_preview` (File › Editor menu and the editor toolbar's Live button) SHALL open or reuse a second Preview pane beside the editor whose source is the editor pane, and return focus to the editor. The same source SHALL be selectable from a Preview pane's source picker ("Live: <name>"). The follower SHALL render the editor's current buffer, including unsaved text, through the Preview pane's normal renderers: the first render of a file at once, later ones after a typing pause of 250 ms plus 100 ms per MiB, re-rendering only when the buffer version changed and never blocking typing. Relative links and images SHALL resolve against the edited file's folder. Buffers over 8 MiB SHALL show "Live preview is off for files over 8 MiB" instead. When the editor leaves Edit mode the follower SHALL show the file as the editor pane previews it.

#### Scenario: Live Markdown
- **WHEN** the user opens a live preview of `README.md` and types a new heading
- **THEN** the follower shows the heading within about a quarter of a second after typing pauses, without the file being saved

#### Scenario: Relative image
- **WHEN** the buffer contains `![logo](img/logo.png)` for `/home/a/docs/README.md`
- **THEN** the follower loads `/home/a/docs/img/logo.png`

#### Scenario: Large buffer
- **WHEN** live preview is requested for a 20 MiB log file in Edit mode
- **THEN** the follower shows "Live preview is off for files over 8 MiB" and typing is unaffected

#### Scenario: No chains
- **WHEN** a follower is asked to follow itself or another follower, or a pane in Edit mode is asked to follow an editor
- **THEN** `pane.preview.source` returns `Err(InvalidParams)`

#### Scenario: Reuse the follower
- **WHEN** a Preview pane already follows this editor and the command runs again
- **THEN** that pane is focused or refreshed and no new pane is created

#### Scenario: Editor closes
- **WHEN** the editor pane closes while a follower exists
- **THEN** the follower reverts to following the most recently focused Files pane

#### Scenario: Wrong pane
- **WHEN** `editor.open_live_preview` runs while a Files pane is focused
- **THEN** it is disabled and nothing happens

### Requirement: Code colour schemes and line spacing
Preferences › Editor SHALL offer code colour schemes (App colours, Warm, High contrast), each with a dark and a light version chosen by the app's theme, through `prefs.editor_scheme {scheme}`. The scheme SHALL colour code in the editor and in the Preview pane alike. Every colour of a scheme SHALL keep at least 4.5:1 contrast on its background and on the caret line's band, and at least 7:1 in the High contrast scheme, including its rainbow bracket colours. `prefs.editor_line_spacing {percent}` (100–200, default 145) SHALL set the editor's line height relative to the font; the caret stays as tall as the text. Comments SHALL be shown in italics. The editor's toolbar, find bar and go-to bar SHALL use painted icon buttons with tooltips and accessible names, growing with the pane's zoom.

#### Scenario: High contrast
- **WHEN** the user picks High contrast with the dark app theme
- **THEN** code is drawn on black with every colour at 7:1 or more, in the editor and in the Preview pane

#### Scenario: Bad scheme
- **WHEN** `prefs.editor_scheme` receives an unknown name, or `prefs.editor_line_spacing` a value outside 100–200
- **THEN** it returns `Err(InvalidParams)` and the preference is unchanged

