## ADDED Requirements

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

