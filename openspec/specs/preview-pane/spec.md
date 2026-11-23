# preview-pane Specification

## Purpose
TBD - created by archiving change pane-preview. Update Purpose after archive.
## Requirements
### Requirement: Preview zoom reset takes precedence over workspace switching
Ctrl+0 (Cmd+0 on macOS) SHALL reset only the focused Preview's zoom when no dialog or
text field owns the keyboard. It SHALL NOT switch
workspaces or change interface zoom. Ctrl+Shift+0 SHALL reset the interface zoom,
as in every other pane. Outside Preview, Ctrl+0 SHALL retain workspace 10
selection; terminal input and text fields SHALL keep their existing ownership.

#### Scenario: Reset zoom from Preview
- **WHEN** Preview is focused with no dialog or text field owning the keyboard and the user presses Ctrl+0
- **THEN** only that Preview's zoom resets
- **AND** the workspace and interface zoom do not change

### Requirement: Preview has pane-local Find
`Ctrl+F`, the Preview menu and a toolbar Find button (non-log views) SHALL open/focus Find in the
focused Preview, without opening the Files-pane filter. Text, structured text
(including raw/pretty mode) and rendered Markdown SHALL offer a query field,
case-sensitive, whole-word and regular-expression options, a current/total match
counter, Previous/Next buttons, and distinct current/other match highlighting in
both themes. Searches SHALL use already-loaded content only, not read files or
search hidden Markdown syntax. Text is searched per source line; Markdown is
searched per rendered paragraph/heading, code block or table cell; a Mermaid
code block drawn as a diagram is not searched (it has no text to highlight) and
is searched as code whenever it is shown as code.

#### Scenario: Navigate matches without leaving Find
- **WHEN** a query has matches and the user presses Enter/F3 or Shift+Enter/Shift+F3
- **THEN** the next/previous match is revealed vertically and horizontally, wrapping at either end
- **AND** Enter/Shift+Enter work while the query retains keyboard focus
- **AND** Escape/Close hides Find and its highlights without changing the document or its copyable text
- **AND** line-number gutters, code-language labels, copy buttons and other UI chrome are not searched

#### Scenario: Bounded asynchronous matching and content changes
- **WHEN** a query, option, loaded document or raw/pretty mode changes
- **THEN** matching runs off the UI thread, at most one worker per pane, and results for obsolete inputs are discarded
- **AND** the current match resets and match locations are recomputed for the new displayed content
- **AND** text rendering remains virtualized when wrapping is off
- **AND** queries are limited to 1024 Unicode characters, regex compilation is bounded, and at most 10 000 matches are retained with a visible `+` when more exist
- **AND** invalid patterns and worker failures are shown inline; empty/no-match queries disable navigation

#### Scenario: Content-specific search and keyboard ownership
- **WHEN** Ctrl+F is used in a log preview
- **THEN** the existing record-filter panel opens and its search field gains focus, retaining severity/time/regex controls and applied filters
- **AND** logs expose only the Filter toolbar button, not a duplicate Find button
- **AND** Escape hides that panel without clearing its filters
- **WHEN** the Preview is an image, video, folder, binary, error or multi-file card grid
- **THEN** Find is disabled with an explanation rather than implying unavailable text/OCR/byte search
- **AND** Files Ctrl+F, terminal input, dialogs and query-field Ctrl+A/Ctrl+C retain their existing behavior

The `preview.find.{open,close,set,next,previous}` commands SHALL validate their
parameters and remain pane-local; `set` accepts query/case/word/regex options and
the other commands accept only an optional pane.

The Find control SHALL share the main source/title toolbar row (preview zoom is in the pane header, see `panes` "Pane zoom"),
not consume a separate row. At narrow widths view controls SHALL move into the
existing View options overflow menu; the filename SHALL truncate to keep that
menu and the log Filter button visible. An open search/filter panel MAY appear
below the toolbar.

### Requirement: Preview identifies the folder in its toolbar
For a single item the Preview toolbar SHALL show its name and full containing
folder path; for a folder it SHALL show that folder's own full path. The current
folder entry (`.`) SHALL show the full folder path instead of a lone dot.
Local paths SHALL use native path notation and remote paths SHALL retain their
scheme and authority. Full names and paths SHALL remain available on hover.

#### Scenario: Resize a Preview pane
- **WHEN** the pane becomes too narrow for both the name and folder path
- **THEN** the name truncates first, preserving the folder path
- **AND** if the path still cannot fit, it shortens with an ellipsis in the middle, retaining its beginning and end
- **AND** the toolbar stays on one row and its controls remain accessible in both themes
- **AND** multi-file previews retain their selected-file count without implying a single folder

### Requirement: Items can be opened in an existing or new Preview pane
The shared file-row/thumbnail context menu SHALL offer **Open in Preview Panel**
through `preview.open {uri}`. It SHALL show the pointed item rather than a stale
selection, reusing a Preview pane in the active workspace (preferring one already
connected to the source Files pane). If no Preview pane exists it SHALL create
one to the right of that Files pane. The panel SHALL be connected to the source
Files pane and subsequent selections there SHALL continue to update it.

#### Scenario: Explicit preview opening is not a toggle
- **WHEN** the action is selected with a Preview pane already open
- **THEN** the pane is reused, retargeted to the source Files pane if necessary, and never closed
- **AND** repeated actions do not create extra panes, and pane zoom is cleared so the Preview is visible
- **AND** the source Files pane remains focused and only the pointed item is selected

#### Scenario: Open a thumbnail in Preview
- **WHEN** the action is selected on an item in a directory thumbnail grid
- **THEN** the connected Files pane navigates to its parent if necessary and selects it, and an existing Preview is reused
- **AND** all file reads and rendering remain background jobs

#### Scenario: Bad or stale target
- **WHEN** parameters are malformed or the requested URI is no longer in the source listing/grid
- **THEN** the command returns an error without changing the layout
- **AND** Ctrl+Shift+P retains its existing Preview toggle behavior

### Requirement: Markdown code blocks have copy controls
Every fenced or indented code block SHALL have a top-right copy icon, including
blocks nested in lists/quotes and Markdown multi-file cards. The control SHALL
be 50% opaque at rest and fully opaque on hover or keyboard focus, scale with
preview zoom, and reserve space so it does not cover code. Activating
`preview.code.copy {pane?, block, item?}` SHALL copy only that loaded block's
plain code text, without language labels, Markdown fences or UI chrome.

#### Scenario: Selecting into text that overflows
- **WHEN** the user drags a text selection to or past the right (or bottom) edge of the text, source view, a Markdown code block or table, and holds the mouse still
- **THEN** that area keeps scrolling that way, faster the further the pointer is past the edge, so the selection extends into the hidden part; it stops when the button is released, and a drag that started on a scroll bar does not trigger it

#### Scenario: Copy code without polluting selection
- **WHEN** a user clicks a code block's copy control
- **THEN** the clipboard contains exactly that block's loaded code, independent of wrapping or syntax highlighting
- **AND** the copy control and language label are not selectable text and never appear in drag-selection or Ctrl+A/Ctrl+C document copies
- **AND** existing whole-document selection is not replaced by the control's label

#### Scenario: Invalid block target
- **WHEN** the command receives malformed parameters, an unavailable preview or an out-of-range block/grid item
- **THEN** it returns an error without changing the clipboard or performing filesystem I/O

### Requirement: Preview follows a source Files pane
A Preview pane SHALL display the entry focused in its source Files pane. The source SHALL default to the most recently focused Files pane and MAY be pinned to a specific Files pane from the pane header; a pinned source that closes SHALL revert to following focus. A Preview pane is never its own source. While a Preview holds unsaved PDF form values or placed signatures, it SHALL keep showing that document instead of following the source, until the changes are saved or discarded.

#### Scenario: Follow focus
- **WHEN** two Files panes and a Preview pane exist and the user focuses the second Files pane and moves its cursor
- **THEN** the preview shows the entry under that cursor

#### Scenario: Pinned source closes
- **WHEN** the pinned source pane is closed
- **THEN** the preview follows the most recently focused Files pane again

#### Scenario: Unsaved PDF holds the preview
- **WHEN** a Preview has unsaved PDF form values and the source cursor moves to another file
- **THEN** the Preview keeps the PDF and its values, and following resumes after Save or Discard

### Requirement: Preview never blocks the UI
Reading and rendering SHALL happen off the UI thread, debounced while the cursor moves rapidly; results for a no-longer-selected entry SHALL be discarded.

#### Scenario: Rapid cursor movement
- **WHEN** the cursor moves through 100 files in one second
- **THEN** no frame waits on I/O and only the final entry's preview is shown

### Requirement: Content is identified by sniffing
The renderer SHALL use the first matching configured filename association, with text validity protecting text-based renderers from binary input. Unmatched files SHALL use magic bytes and content detection without restoring removed filename associations; unknown binary content SHALL fall back to a hex dump with a metadata card.

#### Scenario: Misleading extension
- **WHEN** a PNG file is named `notes.txt`
- **THEN** it is previewed as an image

### Requirement: Configurable preview filename filters
Preferences > Preview SHALL show, for each of Logs, Images, Video, Audio, PDF, Markdown, SVG, JSON, YAML, TOML, XML, HTML, CSV and Text, an entry in a type list (with its pattern count) beside a list of the selected type's persisted filename patterns, each list with an icon toolbar (types: move up/down; patterns: add, remove, edit). Each pattern is one case-insensitive full-filename glob (`*`, `?`, character classes or an exact name); a pattern is edited in place (toolbar, a second click or F2; Enter or leaving the field saves, Escape cancels without closing Preferences), Delete removes the focused one, Up and Down move the selection in either list while it has keyboard focus, Tab, Shift+Tab, Left and Right move the keyboard between the two lists (Tab continues to the next control when the selected type has no patterns), and clicking a row gives it keyboard focus. While Preferences (or another dialog that owns the keyboard) is open, arrow keys SHALL NOT move the cursor of the Files listing, thumbnail grid, preview or playlist behind it. A short explanation of the pattern syntax is shown below the lists. Groups SHALL be checked from top to bottom and the first match SHALL win; users SHALL be able to move types up and down to change priority. A group with no rows disables filename matching for that type, not content detection. HTML SHALL use existing text rendering and never execute markup; CSV SHALL open in the table view (see `table-preview`).

Defaults SHALL cover supported extensions and common system logs (`syslog`, `messages`, `secure`, `maillog`, `cron`, `debug`, `dmesg`), including gzip and numeric/date rotations. Rotation patterns SHALL match a numeric-starting suffix using globs, not an exact rotation-name parser. SVG defaults to `*.svg` and `*.svgz` and comes before XML. Settings are stored as an ordered list of `{type, patterns}` groups; the older one-string-per-type form SHALL migrate automatically and missing or unreadable groups SHALL use defaults. Stored settings without an SVG group SHALL gain one directly above XML, and the default `*.svg` pattern SHALL be removed from XML; other user patterns SHALL be kept. A restore action SHALL reset all groups and order.

#### Scenario: Custom associations apply immediately
- **WHEN** the user removes the Logs rows and adds `*.trace` and `server-output`
- **THEN** matching files use log preview and removed `.log` associations are not reapplied by filename fallback
- **AND** existing single and multi-file previews refresh, and directory thumbnails follow configured image/video associations
- **AND** all existing read, decoder, memory and time limits remain enforced

#### Scenario: Overlap, invalid input and reset
- **WHEN** the same filename matches multiple rows
- **THEN** the first displayed group wins, and moving a group up or down changes the result
- **WHEN** a row is empty, longer than 256 bytes, contains path separators, list separators (`;` `,` `|`) or control characters, or a group exceeds 64 rows or 1024 bytes in total
- **THEN** it is rejected visibly without partially changing saved preferences
- **WHEN** the user restores default file filters
- **THEN** every category and its editable field returns to its default

#### Scenario: Older settings gain the SVG type
- **WHEN** preferences saved before the SVG type existed are loaded
- **THEN** SVG appears directly above XML with `*.svg`, XML no longer lists `*.svg`, and every other pattern and the order of the other types are unchanged

### Requirement: Supported previews
The preview SHALL render: text and source with syntax highlighting; JSON, YAML, TOML and XML as raw highlighted text with an optional pretty-printed view; Markdown; png, jpeg, gif (animated), webp, bmp and ico images; SVG drawings rendered as a picture with a switch to their source; PDF documents as rendered pages (see `pdf-view`), with form filling and signing (see `pdf-forms`, `pdf-signing`); CSV and TSV files as a sortable, filterable table (see `table-preview`); audio metadata; video with playback; a directory summary; and a hex dump for everything else.

Directory and file-information summaries SHALL use the dialog style guide's
proportional text, neutral section headings, muted labels, aligned wrapping values
and flat spacing, without changing document/code/log fonts. Directory summaries
SHALL separate subfolder totals from the overview. When the previewed directory
is on a listed device, Preview SHALL show its name, mount location, filesystem,
capacity, free space and a used-space indicator (when capacity is known), using
only cached device data. Nested mounts SHALL use the closest containing volume.
Unavailable capacity and unsupported removal explanations SHALL remain explicit.
Thumbnail mode SHALL retain volume information in an expandable section.
Read-only summary rows SHALL use compact spacing, not dialog-control heights.
Subfolder totals SHALL use a compact Name / Size / Files table with single-line
truncated folder names and full-name hover text.

#### Scenario: Preview a mounted directory
- **WHEN** a mounted device or a directory inside it is previewed
- **THEN** its volume information appears above the folder summary
- **AND** long values wrap within narrow panes and details update with the cached mount list
- **AND** directories outside listed volumes retain their normal summary without a volume section

#### Scenario: Structured files open raw
- **WHEN** a minified `.json` file is previewed
- **THEN** it is shown exactly as stored, and a Pretty button is available in the preview header

#### Scenario: Pretty on demand
- **WHEN** the user presses Pretty (or runs `preview.toggle_pretty`)
- **THEN** the file is shown re-indented, pressing it again returns to the raw text, and selecting another file resets to raw

#### Scenario: Invalid structured file
- **WHEN** a `.json` file is not valid JSON
- **THEN** it is shown raw, and Pretty is disabled with a "not valid JSON" tooltip

#### Scenario: SVG opens as a picture
- **WHEN** an `.svg` file is previewed
- **THEN** it is shown rendered, with Fit and 100% controls and a Preview toggle in the header

#### Scenario: PDF opens as pages
- **WHEN** a `.pdf` file (or a file whose content starts with `%PDF`) is previewed
- **THEN** its pages are shown instead of a metadata card, and its file information is available as an information section

#### Scenario: Delimited text opens as a table
- **WHEN** a `.csv` or `.tsv` file is previewed
- **THEN** it is shown as a table with a header row and row numbers, and its raw text is one toggle away

### Requirement: Native document-quality Markdown
Markdown SHALL be rendered natively, without a webview or editor, using a bounded
CommonMark document tree with tables, task lists and strikethrough enabled.
Paragraphs SHALL use a proportional reading face with real regular/bold weights,
italic, combined emphasis, inline-code styling, link styling and preserved hard
line breaks. Headings SHALL have distinct scale/weight and document spacing;
soft breaks SHALL flow as spaces. Document width SHALL be limited for readability.

Lists SHALL preserve nesting, ordered start numbers, multiple paragraphs and
embedded blocks. Task checkboxes SHALL show their checked state without permitting
edits. Blockquotes SHALL retain nested paragraphs/blocks with an accent edge.
Fenced code SHALL show its language token and semantic syntax highlighting
computed on the preview worker, with a plain-text fallback for unknown languages
and overlong lines. Code SHALL preserve whitespace and support wrapping or
horizontal scrolling.

Tables SHALL retain header cells, rows, inline formatting and Markdown column
alignment. Each row SHALL have one shared height derived from its tallest cell,
continuous column boundaries and top-aligned cell text. Text SHALL NOT be
justified or letter-spaced to fill cells. Narrow panes SHALL scroll tables
horizontally rather than squeeze columns below a readable minimum.

HTML SHALL remain literal inert text. Images SHALL be rendered as specified in
"Markdown images"; links SHALL NOT be fetched or opened except by an explicit click.
Markdown SHALL enforce a 1 MiB source cap, the configured text-line cap,
5 000 block/list-item nodes, nesting depth below 32, 30 000 inline spans,
4 096 table cells and 64 columns. Exceeding any cap SHALL show truncation.

#### Scenario: Preserve document structure
- **WHEN** Markdown includes a loose nested ordered list, checked/unchecked tasks,
  nested blockquotes, a table and fenced Rust code
- **THEN** all structures remain distinct, ordered numbers are preserved, tasks are
  read-only and the code is highlighted without UI-thread parsing
- **AND** Ctrl+A/Ctrl+C includes all loaded blocks, task markers and tab-separated
  table cells in document order without formatting syntax

#### Scenario: Uneven table cells
- **WHEN** one table column has short symptoms and another has multi-line explanations with inline code
- **THEN** both cells start at the same row top, the next row starts below the
  taller cell, borders remain connected, and ordinary word spacing is preserved
- **AND** resizing or zooming does not cause staggered rows or overlapping columns

### Requirement: Pane-local preview magnification
Every Preview body SHALL support independent 50%-400% magnification in 20% steps
through `preview.scale.in`, `preview.scale.out` and `preview.scale.reset`.
The pane header's zoom (`panes` requirement "Pane zoom", which dispatches
`pane.scale.*` and routes it to these commands) SHALL offer zoom in, the
percentage/reset and zoom out for every view type, including text, Markdown,
structured text, logs, images, hex, metadata, folder summaries, thumbnails,
multi-file cards and video; the Preview toolbar SHALL NOT repeat them.
Only that pane's content SHALL scale; pane chrome, other panes and the application
zoom preference SHALL remain unchanged. Scale SHALL survive changes of previewed
file within the same pane. Images SHALL retain Fit/Actual modes; choosing either
SHALL reset additional magnification. Zoom SHALL NOT increase the file size or pixel limits.
A still image is decoded for Fit with its longest side at most 2048 px; when it is drawn
larger than that copy on screen (100%, magnified, or Fit in a large pane), the pane SHALL ask
once per picture for a sharper decode with `preview.image.full {pane?, edge}` (edge at most
8192 px and the GPU's largest texture, never above the original), run on the job lanes, which
replaces the picture in place when done; a failed decode keeps the shown picture.
A picture larger than the pane SHALL pan by clicking and dragging it with the mouse (it
follows the pointer), besides the scroll bars and wheel; the pointer SHALL show an open hand
over such a picture and a closed hand while dragging. A right-click still opens its menu.

A focused Preview SHALL accept Ctrl/Cmd+= (or +), Ctrl/Cmd+- and Ctrl/Cmd+0.
A shortcut SHALL do the same in every pane: Ctrl/Cmd+Shift+= (or +), Ctrl/Cmd+Shift+-
and Ctrl/Cmd+Shift+0 SHALL keep changing the interface zoom while a Preview has focus.
Dialogs, popups and focused text fields SHALL retain keyboard ownership.
Commands SHALL reject malformed parameters and invalid explicit panes.

#### Scenario: 100% shows the full-resolution picture
- **WHEN** a 4000×3000 photo is shown at Fit (decoded at 2048×1536) and the user picks 100%
- **THEN** the picture is decoded again at 4000×3000 in the background and drawn sharp, and the "(preview downscaled)" note disappears

#### Scenario: Zoom without changing the app
- **WHEN** a focused Preview is zoomed in, out or reset
- **THEN** its body changes size while the toolbar, Files panes, other previews,
  global zoom preference and egui zoom factor remain unchanged
- **AND** minimum/maximum scales are enforced and bad parameters return an error

#### Scenario: Interface zoom keys from a Preview
- **WHEN** a Preview has focus and the user presses Ctrl+Shift+=
- **THEN** the whole application zooms in one step, as from any other pane, and the Preview's own zoom is unchanged

### Requirement: Select and copy loaded preview text
A focused single text, source, structured-text or Markdown preview SHALL support
`Ctrl+A` (`Cmd+A` on macOS) followed by `Ctrl+C` (`Cmd+C`) to copy all loaded
document text, including rows outside the viewport. The commands
`preview.text.select_all` and `preview.text.copy` SHALL also be available in the
Preview menu, reject non-null parameters and report unsupported or missing selections.
Selection SHALL be highlighted in both themes. Clicking document content or Escape
SHALL restore ordinary selection; dialogs, popups and focused text fields retain
their own shortcuts.

Copy SHALL omit line-number gutters and follow the current raw/pretty view.
Markdown SHALL copy rendered text, code and list markers rather than Markdown source
formatting. Existing preview size/line/block caps SHALL remain unchanged: truncated
previews copy only their loaded content, without additional reads. Changing document
identity or raw/pretty mode SHALL invalidate a previous whole-document selection.

#### Scenario: Copy offscreen text
- **WHEN** a text preview has more loaded rows than fit in the viewport and the user presses Ctrl+A then Ctrl+C
- **THEN** the clipboard includes every loaded row, not just visible labels
- **AND** selecting another file prevents copying the old document selection

#### Scenario: Copy rendered Markdown
- **WHEN** a Markdown preview containing a heading, formatted paragraph and code block is selected and copied
- **THEN** their rendered text is copied in document order without heading or emphasis syntax

### Requirement: Hostile files are bounded
Previews SHALL enforce size, line, dimension, frame and time limits, check image dimensions before allocating, never follow links or load external resources, and convert any decoder panic, error or timeout into an error card. Tables SHALL cap a record at 1 MiB, a record at 16 384 fields, a displayed field at 64 KiB, sorting at 512 MiB of keys or 20 million rows, and filter results at 20 million rows. PDFs SHALL additionally follow the limits in `pdf-view` ("Hostile PDFs are bounded").

#### Scenario: Decompression bomb
- **WHEN** an image header declares more than 100 megapixels
- **THEN** it is not decoded and the card says it is too large to preview

#### Scenario: Corrupt media
- **WHEN** a decoder fails or panics
- **THEN** the pane shows "the file could not be previewed" and the app keeps running

#### Scenario: Unclosed quote
- **WHEN** a CSV file opens a quoted field that never closes
- **THEN** the record is cut after 1 MiB at the next line break, the rest of the file still shows as rows, and the toolbar counts the malformed record

#### Scenario: Corrupt PDF
- **WHEN** a file starts with `%PDF` but its structure is unreadable
- **THEN** the pane shows that the PDF could not be read, with its file information, and the app keeps running

### Requirement: Video playback is owned by the pane
A Preview pane SHALL play at most one video, with sound, starting automatically unless disabled in Preferences, controllable with play/pause, seek, volume and mute, decoded at a size fitted to the pane; leaving the file, switching the pane kind or closing the pane SHALL stop it and end the decoder process. This simple single-file playback SHALL remain independent of the Media pane's queue and SHALL NOT require a Media pane. Without a decoder available the pane SHALL show metadata and a hint instead; without an audio device it SHALL play silently with a hint.

#### Scenario: Pane closed while playing
- **WHEN** a Preview pane is closed during playback
- **THEN** the decoder child process no longer exists

#### Scenario: Video with sound
- **WHEN** a video with an audio track is previewed on a machine with an audio device
- **THEN** it plays with sound and the controls offer play/pause, seek, volume and mute

#### Scenario: No audio device
- **WHEN** a video is previewed on a machine with no audio device
- **THEN** it plays silently and the pane shows a hint

### Requirement: Preview commands
`pane.preview.source`, `preview.zoom`, `preview.toggle_wrap`, `preview.toggle_pretty` and `preview.video.play`, `preview.video.seek` and `preview.video.fit` SHALL be commands with ids, menu paths and bad-parameter tests; they are no-ops unless the focused pane is a Preview. The PDF view, form and signing commands are specified in `pdf-view`, `pdf-forms` and `pdf-signing`.

#### Scenario: Bad source
- **WHEN** `pane.preview.source` is given a pane that is not a Files pane
- **THEN** it returns an error and the source is unchanged

### Requirement: Preview works on every supported platform
Preview SHALL build and behave the same on Linux, Windows and macOS: no system libraries are required at build time, UTF-16 and CRLF text render correctly, the video decoder is discovered from a bundled sidecar or the platform's usual locations without opening a console window, and a missing decoder or audio device degrades to metadata or silent playback with a hint.

#### Scenario: UTF-16 text with CRLF
- **WHEN** a UTF-16LE file with a BOM and CRLF line endings is previewed
- **THEN** it renders as readable text with correct line breaks

#### Scenario: No decoder installed
- **WHEN** no ffmpeg can be found
- **THEN** a video shows its file metadata and an install hint, and nothing else breaks

### Requirement: Multi-file preview
The preview SHALL follow selected entries in file-pane display order, falling back to the cursor when nothing is selected. Multiple text/log files SHALL share search and filters. Users SHALL be able to switch logs between concatenation in file-pane order and chronological merging. A non-selectable filename gutter SHALL identify every record without contaminating copied text. Images and selections unsuitable for concatenation SHALL render as individually labelled cards in a responsive grid.

#### Scenario: Searching selected log rotations
- **WHEN** several logs or gzip rotations are selected
- **THEN** all selected sources contribute to one preview and literal/regex search, severity and time filters apply across all loaded records
- **AND** concatenation preserves source and record order; chronological mode sorts timestamps ascending, keeps equal timestamps stable, and puts records with unknown timestamps last
- **AND** source filenames remain in the gutter, including stack-trace lines, and never enter clipboard text
- **AND** changing ordering preserves the current filters

#### Scenario: Bounded multi-file loading
- **WHEN** selected files are huge, numerous, unreadable or replaced during loading
- **THEN** work remains off the UI thread, memory and source counts are bounded, stale results are discarded, and failures or truncation are explicit
- **AND** the preview explains that filters cover the loaded window, offers larger log windows up to the cap, and clearly disables live tail for multi-file previews

#### Scenario: Selection and preview formats
- **WHEN** selection membership or file-pane sorting changes
- **THEN** the preview updates even if the focused file is unchanged
- **AND** a single selected file retains its existing renderer and features
- **AND** mixed-format cards preserve per-file errors rather than silently dropping unreadable sources

#### Scenario: Selected files change after loading
- **WHEN** a selected source changes while a multi-file preview is loaded
- **THEN** the loaded document or grid stays unchanged instead of automatically reloading
- **AND** a Reload button appears only when newer content is detected
- **AND** Reload or View > Reload Selected Files Preview loads current sources on a worker, preserving filters, ordering and display settings
- **AND** multi-file logs show Refresh instead of Live tail in the filter toolbar, available even before source changes are detected; it reloads all selected sources through the same command, preserving filters, ordering and display settings
- **AND** single-file logs retain Live tail; Refresh is disabled while a multi-file log operation is busy
- **AND** changing the selection still loads its preview normally

### Requirement: Log mode
When the previewed file is a log (by name, location or content), the preview SHALL offer a Log mode that parses nginx/Apache access and error logs, Magento and Pimcore (Monolog) logs, Magento `var/report` JSON, PHP-FPM/PHP errors and generic text; colours records by level or HTTP status; supports level filter, text/regex search, time-range filter, folding of stack traces and a column view for access logs; reads `.gz` rotations; and offers Follow (live tail). Files of any size SHALL open with the end visible immediately, with older records loaded on demand in bounded chunks and memory capped.

#### Scenario: Huge log opens at the tail
- **WHEN** a multi-gigabyte log is previewed
- **THEN** the last records are shown without reading the whole file and the UI never blocks

#### Scenario: Extensionless system logs
- **WHEN** a text file is named `syslog`, `messages`, `secure`, `maillog`, `cron`, `debug` or `dmesg`, including numeric/date rotations and gzip rotations
- **THEN** it opens in Log mode without requiring a `.log` extension
- **AND** binary login databases such as `lastlog`, `wtmp` and `btmp` remain binary previews rather than being interpreted as text

#### Scenario: Follow survives rotation
- **WHEN** Follow is on and the file is truncated or replaced
- **THEN** the view reindexes and keeps following without error

#### Scenario: Live tail help and configuration
- **WHEN** the user hovers Live tail
- **THEN** help explains background size checks on local, SFTP and SMB files, no content download when size is unchanged, and offset-based reads of appended bytes
- **AND** it explains initial header/tail loading, markers surviving appends but resetting on shrink/reload, same-size replacements not being detected, and scrolling up pausing follow
- **AND** it points to Preferences > Preview for the polling interval and initial tail size
- **WHEN** the user changes log settings in Preferences > Preview
- **THEN** `prefs.preview.log` validates and persists a polling interval of 100..60000 ms (default 500) and initial tail size of 1..64 MiB (default 8), or zero for an explicit Unlimited initial-tail option
- **AND** Unlimited removes only the initial-window limit: reads remain bounded to the 64 MiB single-log safety window or 32 MiB shared multi-file window, with existing parser/count/time limits retained; larger logs still start at the tail
- **AND** interval changes reschedule active follow polls, while initial size changes apply to newly opened or reloaded logs (including follow reload after shrink), not by automatically reloading the current view
- **AND** invalid settings return an error without partially changing preferences, and missing saved fields restore the bounded defaults

#### Scenario: Choosing a log time range
- **WHEN** the user opens Time in the preview's expandable filter row, including in a narrow pane
- **THEN** the time dialog is centered in the application viewport
- **THEN** a separate window offers a quick-range list alongside optional From and To dates, editable `YYYY-MM-DD` dates with calendar pickers, and `HH:MM` or `HH:MM:SS` time fields
- **AND** UTC (UTC+00:00) is explicit, while log timestamps retain their original offsets
- **AND** quick ranges offer the last 5/15/30 minutes, 1/3/6/12/24 hours and 3/7 days, setting From to the current time minus the interval evaluated on Apply and leaving To unchecked/open-ended, without scanning or loading additional records
- **AND** applying a quick range produces a fixed From rather than a continuously sliding filter, allowing newly loaded Live tail records through without an upper bound; the dialog explains that filters affect only loaded records
- **AND** opening it restores applied bounds or suggests dates from the loaded log records
- **AND** choosing a quick range, editing fields or choosing calendar dates does not close the window or apply a filter; manual edits select the custom range
- **AND** Apply dispatches the time-range command, Clear removes the bounds, and Cancel or closing the window leaves the applied filter unchanged
- **AND** an invalid date/time or reversed range shows an inline error and keeps the window open; unchecked bounds remain open-ended

#### Scenario: Hostile regex
- **WHEN** the user enters an invalid or oversized regex
- **THEN** the filter shows an error and the view is unchanged

#### Scenario: Log filter help
- **WHEN** the user hovers over the severity dropdown or the `.*` search toggle
- **THEN** a tooltip explains minimum severity (including the order, a Warning example, All and unrecognised levels counting as Info) or regular-expression versus literal search (including an `error|warning` example), respectively
- **AND** the help clarifies that filtering changes only the preview, not the file
- **AND** the tooltips use proportional text, a distinct heading and spaced explanations rather than a dense multiline block
- **AND** hovering Expand explains showing all lines of multiline records (including stack traces) and collapsing them to their first line

#### Scenario: Multi-line record
- **WHEN** a stack trace follows an error line
- **THEN** it belongs to that record and can be folded

### Requirement: Narrow toolbar and clean text copy

Log and combined-text previews SHALL initially hide their options behind a prominent Filter button immediately next to the panel selector in the main toolbar. The button SHALL toggle a second row containing ordering, severity, search, regex, time and the other log controls. The row SHALL wrap in narrow panes rather than requiring an overflow popup. Hiding the row SHALL preserve applied filters and ordering.

Non-log preview toolbar options that do not fit the pane width SHALL move into a menu button at the right. In text previews the line-number gutter SHALL NOT be selectable, and copying selected text SHALL yield the text without line numbers or extra blank lines. Ctrl+C/X/V in Preview and Trash panes SHALL act on selected text, not on files.

#### Scenario: Clearing all preview filters
- **WHEN** the user clicks Clear (the last button in the filter row) or chooses View > Clear All Preview Filters
- **THEN** severity, search text, regex and both time bounds reset together and any filter error clears
- **AND** all loaded records are shown again in the existing source order, preserving folding, columns and live-tail settings
- **AND** the visible search and regex controls reflect the reset, and the button is disabled when no filters are set
- **AND** every filter-row button and control has a tooltip explaining its purpose

#### Scenario: Paging through preview content
- **WHEN** a Preview pane is focused and Page Up or Page Down is pressed
- **THEN** its main scrollable body moves by approximately one visible page with a small overlap
- **AND** paging a live log pauses live tail so it does not jump back to the end
- **AND** inactive previews, text inputs, open menus and date/time pickers do not steal these keys
- **AND** thumbnail grids retain their existing keyboard selection and paging behavior
- **AND** Up/Down scroll the focused preview vertically by one line without moving it sideways, respecting the same keyboard ownership rules
- **AND** Left/Right scroll horizontally while Home/End jump to the top/bottom of the loaded content, respecting the same keyboard ownership rules

#### Scenario: Copy text
- **WHEN** the user selects lines in a text preview and copies
- **THEN** the clipboard holds exactly those lines

### Requirement: Markdown links are clickable and Markdown has a source view
Clicking a link in a rendered Markdown preview (paragraphs, headings, list items and table cells) SHALL follow it with `preview.open_link {url, pane?}`. `http`, `https` and `mailto` URLs of at most 2048 characters without whitespace or control characters SHALL open in the default browser or mail client. A path relative to the previewed file (percent-decoded, `?query` and `#fragment` ignored, `.` and `..` resolved) SHALL be shown in a Files pane like `file.reveal`: its folder opens and the item is selected. Any other link (`file:`, `javascript:`, absolute or `//` paths, in-page `#anchors`, paths that climb above the root) SHALL NOT be launched and SHALL report an error. Link text SHALL show a pointing-hand cursor and its URL on hover, and selecting text by dragging SHALL NOT open a link.

A Markdown preview SHALL offer a toggleable Preview toolbar button (`preview.toggle_source`, View menu) that shows the highlighted Markdown text instead of the rendered document. Find and Copy SHALL operate on whichever view is shown, and the view SHALL reset to rendered when the previewed file changes. JSON, YAML, TOML and XML keep their existing Pretty toggle, whose off state is the raw source.

#### Scenario: Open a web link
- **WHEN** the user clicks the text of `[docs](https://example.org)` in a rendered document
- **THEN** `https://example.org` opens in the default browser

#### Scenario: Jump to a linked file
- **WHEN** the user clicks `[install](docs/install.md)` in `/home/a/README.md`
- **THEN** a Files pane shows `/home/a/docs` with `install.md` selected

#### Scenario: Refuse an unsafe link
- **WHEN** a document links to `file:///etc/passwd` or `javascript:alert(1)` and the user clicks it
- **THEN** nothing is launched and `preview.open_link` returns an error

#### Scenario: Toggle the source
- **WHEN** the user clicks the active Preview button on a Markdown preview and clicks it again
- **THEN** the Markdown text is shown, then the rendered document, with Find matching the visible view

### Requirement: Preview and Diff views
When the previewed file has a diff (see `git-diff`), the Preview toolbar SHALL show
**Preview** / **Diff** buttons that switch the view through `preview.diff.toggle {show}`. The
Diff view has a second bar with the revision button, "vs Working copy" and the diff options.

#### Scenario: Toggle
- **WHEN** the user clicks Preview while the diff is shown
- **THEN** the pane shows the normal preview and stays there for following files until Diff is chosen again

### Requirement: Markdown HTML subset
The Markdown viewer SHALL interpret a closed subset of HTML in HTML blocks and inline HTML: `p`, `div`, `center`, `h1`–`h6`, `br`, `a`, `img`, `picture`, `source`, `code`, `strong`, `b`, `em`, `i`, `table`, `tr`, `th`, `td`, plus `hr`, `ul`, `ol`, `li`, `pre` and `blockquote` as plain paragraph breaks. Elements with `align="center"` SHALL render centered. `img` MAY carry `width` in pixels. A `picture` SHALL show its `source` whose `media` is `(prefers-color-scheme: dark)` when the theme is dark and its `img` otherwise. Tables SHALL render with images and text in cells. Any other tag SHALL be ignored while keeping its text, and `script` and `style` content SHALL never be shown or acted on.

#### Scenario: Theme-aware picture
- **WHEN** Markdown contains `<picture><source media="(prefers-color-scheme: dark)" srcset="d.webp"><img src="l.webp"></picture>`
- **THEN** the dark theme shows `d.webp` and the light theme shows `l.webp`, never both

#### Scenario: Centered header and links
- **WHEN** Markdown contains `<h1 align="center">Name</h1>` and a centered `<p>` of links
- **THEN** they render centered and the links are clickable like Markdown links

#### Scenario: Image grid
- **WHEN** Markdown contains a `<table>` whose cells hold pictures
- **THEN** the cells render as a grid without a header row and images load as in Markdown

#### Scenario: Unknown markup
- **WHEN** Markdown contains `<script>alert(1)</script>` or `<blink>hi</blink>`
- **THEN** the script is not shown and `hi` is shown as plain text

### Requirement: Markdown images
Rendered Markdown SHALL show images written as `![alt](url "title")`. Images in paragraphs
SHALL be placed as block-level rows directly after the paragraph text, consecutive images
forming one wrapping row; an image inside a link SHALL keep that link. Images in headings
and table cells SHALL remain `[Image: alt]` placeholders. At most 200 images per document
SHALL be rendered; further ones stay placeholders.

A relative URL SHALL be resolved against the previewed file with the same rules as
Markdown links (percent-decoded, `?query` and `#fragment` ignored, `.` and `..` resolved,
refused above the root) and read through the file's own backend. Absolute paths, `file:`,
`data:`, `//` URLs and other schemes SHALL NOT be loaded. Supported formats are those
of image previews. SVG images are rasterized like SVG previews: text is drawn with installed system fonts, and embedded or external resources are not loaded.

`http` and `https` images SHALL be loaded only when Preferences > Preview > "Load remote
images in Markdown" is enabled; it SHALL default to off. While off, remote images SHALL
show their alt text with a "Remote image blocked" hover hint and SHALL cause no network
access.

Images SHALL load off the UI thread; until ready, and when blocked, missing, oversized,
undecodable or failed, the alt-text placeholder SHALL be shown and the document SHALL
remain usable. Images SHALL scale down to the document width, never up past natural size,
follow preview magnification, show alt text/title on hover, and render correctly in
dark and light themes. Copy SHALL treat an image as its alt text; Find does not search alt text.

#### Scenario: Local relative image
- **WHEN** `README.md` contains `![logo](img/logo.png)` and `img/logo.png` exists
- **THEN** the rendered view shows the image below its paragraph without blocking a frame

#### Scenario: Missing or escaping path
- **WHEN** an image points at a missing file or at `../../../etc/x.png`
- **THEN** the alt-text placeholder is shown, nothing outside the root is read, and the preview stays usable

#### Scenario: Remote images off by default
- **WHEN** Markdown contains `![badge](https://example.com/b.svg)` and the preference is off
- **THEN** no connection is made and the placeholder shows "Remote image blocked" on hover

#### Scenario: Remote images enabled
- **WHEN** the preference is enabled and the URL serves a valid image within limits
- **THEN** the image appears; toggling the preference re-renders open Markdown previews

#### Scenario: Linked badge
- **WHEN** an image is wrapped in a link and clicked
- **THEN** the link is followed by the normal Markdown link rules

### Requirement: SVG preview
An SVG file SHALL be rasterized on the preview worker, never the UI thread, at the larger of its declared size and 2048 px on the longest edge, within the existing image edge and pixel limits. The picture SHALL be drawn over a background chosen in the header: Checkered (default, from theme colours), White, Black or 50% grey; the choice SHALL be saved in preferences, apply to every SVG preview, and stay reachable when narrow toolbars fold into their menu. The header Preview toggle (`preview.toggle_source`) SHALL switch between the picture and the file's highlighted XML source, with Pretty and wrap available in the source view; selecting another file SHALL return to the picture. Files matching no filename pattern whose start contains an `<svg` element SHALL be previewed as SVG. Gzip-compressed SVG (`.svgz`, or gzip data matched by an SVG pattern) SHALL be decompressed to at most 16 MiB before parsing; larger output SHALL be refused with a reason, never decoded further. SVG and SVGZ files matched by the SVG type SHALL get thumbnails in folder thumbnail grids, drawn by the same renderer with the same rules, within the thumbnail edge and cached like image thumbnails.

Text SHALL be drawn with the installed system fonts, loaded once off the UI thread; fonts are never downloaded. Generic families (serif, sans-serif, monospace) SHALL map to installed fonts, text without a font-family SHALL be serif, and a named font that is not installed SHALL fall back to an installed font of the same kind (monospace for names such as Consolas or Courier, serif for names such as Times or Georgia, otherwise sans-serif, e.g. for Helvetica or Segoe UI).

Rendering SHALL NOT load external files, URLs or embedded resources (including `<image>` references to local paths and `data:` URLs), nor run scripts or animation. An SVG that cannot be parsed, has no usable size or exceeds limits SHALL show the reason and the source view.

#### Scenario: Background choice
- **WHEN** the user picks Black from the SVG background dropdown (or runs `preview.svg.background` with `{value: "black"}`)
- **THEN** transparent areas of every SVG picture show black, and the choice survives a restart

#### Scenario: Switch to source and back
- **WHEN** the user turns Preview off on a rendered SVG
- **THEN** the highlighted XML source is shown without reading the file again, and turning Preview on shows the picture

#### Scenario: Text in a diagram
- **WHEN** an SVG labels its boxes with `font-family="Helvetica"` on a system without Helvetica
- **THEN** the labels are drawn in an installed sans-serif font, keeping weight where the font has it

#### Scenario: Broken or hostile SVG
- **WHEN** the SVG is malformed, declares a size of `1e30`, or references `/etc/passwd` or an `http` URL
- **THEN** the preview shows an error with the source (or renders without the reference), no external resource is read, and the UI stays responsive

#### Scenario: Compressed SVG
- **WHEN** a `.svgz` file is previewed or shown in a thumbnail grid
- **THEN** it is drawn like the same `.svg`, and the source view shows the decompressed XML
- **WHEN** a `.svgz` expands beyond 16 MiB
- **THEN** decompression stops at the cap and the preview reports that the SVG is too large

#### Scenario: SVG thumbnails
- **WHEN** a folder with `.svg` files is shown as thumbnails
- **THEN** each SVG shows its drawing, and moving `*.svg` out of the SVG type removes them

### Requirement: Volume information in directory previews
The directory Preview SHALL show the closest containing listed volume's name,
mount path, filesystem, capacity and free space, plus used space when known.
A folder that is itself the root of a local volume (a Windows drive such as
`C:\`, `/`, or any mount point), listed under Devices or not, SHALL show the
same information for that volume. It SHALL use cached data only (the Devices
list and a cached list of every local volume refreshed in the background),
reflect cache changes, explicitly show unavailable capacity and preserve
unsupported-removal explanations. Thumbnail mode SHALL keep this information in
an expandable section.

#### Scenario: Preview inside a mounted volume
- **WHEN** a mounted directory or its descendant is previewed
- **THEN** volume information appears above its folder summary without extra sidebar lines
- **AND** a directory outside listed volumes that is not a volume root has no volume section

#### Scenario: Preview a system drive
- **WHEN** a Windows user selects `C:` in This PC
- **THEN** Preview shows its name, `C:\`, the filesystem (for example NTFS), capacity, free space and a used-space bar above the folder summary

### Requirement: Readable information summaries
Folder and file-information summaries SHALL follow the dialog style guide's
proportional text, neutral flat headings, muted labels and aligned selectable
wrapping values. Subfolder totals SHALL have their own section. Document, code,
hex and log typography SHALL remain unchanged.
Read-only rows SHALL use compact spacing rather than dialog-control heights.
Subfolders SHALL use a dense Name / Size / Files table with single-line truncated
names and full names on hover.

#### Scenario: Narrow preview
- **WHEN** a long filename or path appears in a narrow pane in either theme
- **THEN** values wrap inside the pane rather than extending beyond its edge

### Requirement: Audio playback in Preview
A Preview pane SHALL play one audio file at a time, showing cover art and tags with the same simple controls as video (play/pause, seek, volume, mute), without a queue. Playback SHALL stop when the selection changes, the pane kind switches or the pane closes. Audio SHALL NOT start automatically unless the "autoplay audio" preference is on (off by default) so browsing a music folder stays quiet.

#### Scenario: Preview an mp3
- **WHEN** an mp3 is previewed and the user presses play
- **THEN** it plays with sound and shows its cover art and tags

#### Scenario: Browsing is quiet
- **WHEN** the selection moves through several audio files with autoplay off
- **THEN** nothing plays

#### Scenario: Selection changes
- **WHEN** a previewed audio file is playing and another file is selected
- **THEN** the playback stops

### Requirement: Familiar single-file keyboard controls
When a Preview audio or video player owns keyboard input, it SHALL support the same basic shortcuts as the Media pane: Space/K for play/pause, J/L for backward/forward 10 seconds, Left/Right for backward/forward 5 seconds, Up/Down for volume changes of 5 percentage points, M for mute, 0–9 for 0%–90% seeking and Home/End for start/end. Percentage/end seeking requires a known duration. These shortcuts SHALL dispatch Preview commands and SHALL NOT affect the Media queue. They SHALL respect terminal, text, modal and interactive-widget input ownership and SHALL trigger at most one action per key event. Queue, fullscreen, speed, subtitle and frame-step shortcuts SHALL NOT add those features to simple Preview playback.

#### Scenario: Preview shortcut targets Preview
- **WHEN** a Preview player owns keyboard input and the user presses K, then L
- **THEN** Preview toggles play/pause and seeks forward ten seconds while the Media queue remains unchanged

### Requirement: One player at a time and hand-off to the Media pane
Preview playback and the Media pane SHALL share one audio output: starting playback in one SHALL pause playback in the other. Preview SHALL offer "Play in Media pane", "Play next" and "Add to queue" for audio and video. Play SHALL insert after the current Media item, stop the Preview decoder and immediately continue in Media from the Preview position. Play next SHALL insert after the current Media item without changing playback. Add to queue SHALL append at the end without changing playback. Play next and Add to queue SHALL NOT stop Preview or seek the current Media item; queued items start at the beginning when reached. All existing queue entries SHALL be retained until explicitly removed or cleared.

#### Scenario: Preview starts while the Media pane plays
- **WHEN** the Media pane is playing and the user starts a video in Preview
- **THEN** the Media pane pauses and the Preview video plays

#### Scenario: Hand-off
- **WHEN** the user chooses "Play in Media pane" on a previewed video at 1:30
- **THEN** the Preview stops and the Media pane plays it from about 1:30

#### Scenario: Queue without interrupting Preview
- **WHEN** an audio file plays in Preview and the user chooses Add to queue
- **THEN** it is appended to Media's queue while Preview continues from its current position

### Requirement: Pictures are shown upright and can be turned
Raster pictures SHALL be shown the way their EXIF orientation says (rotation and mirroring), in the Preview pane, folder thumbnails, images inside Markdown, media cover art and Copy Image, and the size shown SHALL be the upright size. A picture's Preview toolbar SHALL offer Rotate left and Rotate right (`preview.image.rotate {direction: "left"|"right", pane?}`), which turn the shown picture a quarter turn at a time without changing the file. The turn SHALL belong to the file shown: another file starts upright, and returning to the same file in that pane keeps its turn. Fit SHALL fit the turned picture.

#### Scenario: Phone photo stored sideways
- **WHEN** a JPEG stored 4032×2268 with EXIF orientation 6 is previewed or thumbnailed
- **THEN** it is shown upright as 2268×4032

#### Scenario: Turning a picture
- **WHEN** the user clicks Rotate right on a wide picture
- **THEN** the picture is drawn a quarter turn clockwise, tall and fitted to the pane, and the file on disk is unchanged

#### Scenario: Next file
- **WHEN** the user turned a picture and selects another picture
- **THEN** the other picture is shown upright

### Requirement: ffmpeg can be installed on demand
Where a verified build is pinned (Windows), Preferences > Media SHALL show where ffmpeg was found (system or app-managed), and offer Download, Update (when a newer pin ships in this app version) and Reinstall. The system install is always preferred; the app-managed copy is searched last. Downloading runs on a background thread with progress and never blocks the UI.

#### Scenario: System ffmpeg present
- **WHEN** ffmpeg and ffprobe are found next to the executable or on `PATH`
- **THEN** they are used, the section reports the location, and no download is started

#### Scenario: Missing ffmpeg
- **WHEN** no ffmpeg is found and a pin exists for the platform
- **THEN** the hint on a video card or error names Preferences > Media, and Download starts the verified installation; on success playback works without restarting

#### Scenario: Newer pin
- **WHEN** the installed manifest version differs from the app's pin
- **THEN** the section offers Update; the old copy keeps working until the new one is verified and renamed into place

#### Scenario: Failed install
- **WHEN** the download fails, is rejected, or the files cannot be replaced
- **THEN** the section shows the reason and the previous install, if any, stays usable

### Requirement: Camera RAW photos
Camera RAW photos with a TIFF structure (DNG, CR2, NEF, NRW, ARW, SR2, SRF, ORF, PEF, SRW, ERF, 3FR), recognised by their content, SHALL be previewed, thumbnailed and copied through the largest embedded baseline or progressive JPEG they contain, turned by the RAW file's orientation. Sensor data SHALL never be decoded. The preview SHALL read only the TIFF directories and the chosen JPEG, not the whole file, and SHALL bound the walk (directories visited, entries per directory, SubIFD depth, loops) and check every offset. The info line SHALL name the format ("DNG camera preview"). A TIFF file without such a JPEG SHALL show a card saying it has no embedded picture.

#### Scenario: Canon CR2
- **WHEN** a CR2 with a full-size 5184×3456 embedded JPEG and a lossless sensor strip is previewed
- **THEN** the 5184×3456 picture is shown (downscaled for the pane), labelled "Canon CR2 camera preview", and the sensor strip is not read

#### Scenario: Damaged RAW file
- **WHEN** a RAW file is truncated anywhere or has corrupted directory entries, offsets pointing outside the file or a directory that points at itself
- **THEN** the preview shows the picture or the no-picture card, never crashes and never hangs

### Requirement: Mermaid diagrams in Markdown
A rendered Markdown preview SHALL draw a fenced code block whose language (the first word of the info string, case-insensitive) is `mermaid` as a diagram when Preferences > Preview > "Draw Mermaid diagrams" is on; the preference SHALL default to on, and when it is off such blocks SHALL be ordinary code blocks. Supported diagram types SHALL be flowchart (`flowchart`, `graph`), sequence, class, state, entity-relationship, Gantt, pie, user journey, timeline, git graph and mind map, with subgraphs, node and edge labels (including `<br/>` line breaks), and solid, dotted, thick, undirected and two-way edges.

Diagrams SHALL use the app's current theme (dark or light, contrast and accent settings) for background, shapes, borders, text, lines, subgroup boxes and labels; theme settings in the document's own `%%{init}%%` directive SHALL NOT override them. A diagram SHALL scale down to the document width, never up past its natural size, and follow preview magnification, like Markdown images. While a diagram is being drawn, a placeholder SHALL reserve its space; the rest of the document SHALL stay usable.

A diagram SHALL keep the code block's header (language label and copy control, which copies the Mermaid source exactly) and add two icon controls, each with a tooltip and accessible name: "Copy diagram as image", which puts the drawn diagram on the clipboard as an opaque picture on its background colour with a margin, and "Show source", which shows that one block as its highlighted source code, with a "Show diagram" icon control to draw it again; the choice SHALL apply to the current document only, and a block shown as source SHALL be searched by Find like code. Whole-document copy SHALL contain the source as code text. The diagram SHALL be exposed to assistive technology as "Mermaid diagram" with its type, described by its source (first 1000 characters). The Markdown source view SHALL show the Mermaid source.

When a diagram cannot be drawn (unknown or unsupported type, syntax error, a limit exceeded, a render failure or timeout), the block SHALL be shown as an ordinary highlighted code block with one line above it: "Diagram not drawn: " followed by a short reason. At most 50 diagrams SHALL be drawn per document; later ones SHALL be code blocks with the reason "Too many diagrams in this document".

#### Scenario: Flowchart with subgroups
- **WHEN** a Markdown file contains a `mermaid` block with `flowchart TB`, four `subgraph` blocks, chained edges, labelled edges and a dotted edge
- **THEN** the rendered view shows one diagram with every node, subgroup title and edge label readable, in the current theme's colours, without blocking a frame

#### Scenario: Theme and zoom
- **WHEN** the user switches between dark and light, or zooms the Preview in
- **THEN** the diagram is redrawn in the new colours or at the new size and its labels stay inside their boxes

#### Scenario: Broken diagram
- **WHEN** a `mermaid` block contains `flowchart LR` followed by `A[[[ --> }}}`
- **THEN** the block is shown as code with "Diagram not drawn: " and the parse problem above it, and the rest of the document renders normally

#### Scenario: Copy the source
- **WHEN** the user activates the copy control on a drawn diagram
- **THEN** the clipboard contains exactly the Mermaid source of that block

#### Scenario: Show one diagram's source
- **WHEN** the user activates "Show source" on a drawn diagram
- **THEN** that block alone is shown as its highlighted Mermaid source with a "Show diagram" button, other diagrams stay drawn, and opening another file shows all of its diagrams drawn

#### Scenario: Copy a diagram as an image
- **WHEN** the user activates "Copy diagram as image" on a drawn diagram
- **THEN** the clipboard holds the diagram as shown, without transparent pixels, so it stays readable when pasted on any background

#### Scenario: Clear the diagram cache
- **WHEN** the user activates Preferences > Advanced > "Clear diagram cache" (or Preview > Clear Diagram Cache)
- **THEN** every drawn diagram and remembered failure is forgotten, queued and running drawing work is dropped, and open documents draw their diagrams again

#### Scenario: Preference off
- **WHEN** "Draw Mermaid diagrams" is turned off
- **THEN** every `mermaid` block is shown as an ordinary code block on the next frame, and Find searches its text

