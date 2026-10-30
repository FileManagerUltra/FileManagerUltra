# preview-pane Specification

## Purpose
TBD - created by archiving change pane-preview. Update Purpose after archive.
## Requirements
### Requirement: Markdown code blocks have copy controls
Every fenced or indented code block SHALL have a top-right copy icon, including
blocks nested in lists/quotes and Markdown multi-file cards. The control SHALL
be 50% opaque at rest and fully opaque on hover or keyboard focus, scale with
preview zoom, and reserve space so it does not cover code. Activating
`preview.code.copy {pane?, block, item?}` SHALL copy only that loaded block's
plain code text, without language labels, Markdown fences or UI chrome.

#### Scenario: Copy code without polluting selection
- **WHEN** a user clicks a code block's copy control
- **THEN** the clipboard contains exactly that block's loaded code, independent of wrapping or syntax highlighting
- **AND** the copy control and language label are not selectable text and never appear in drag-selection or Ctrl+A/Ctrl+C document copies
- **AND** existing whole-document selection is not replaced by the control's label

#### Scenario: Invalid block target
- **WHEN** the command receives malformed parameters, an unavailable preview or an out-of-range block/grid item
- **THEN** it returns an error without changing the clipboard or performing filesystem I/O

### Requirement: Preview follows a source Files pane
A Preview pane SHALL display the entry focused in its source Files pane. The source SHALL default to the most recently focused Files pane and MAY be pinned to a specific Files pane from the pane header; a pinned source that closes SHALL revert to following focus. A Preview pane is never its own source.

#### Scenario: Follow focus
- **WHEN** two Files panes and a Preview pane exist and the user focuses the second Files pane and moves its cursor
- **THEN** the preview shows the entry under that cursor

#### Scenario: Pinned source closes
- **WHEN** the pinned source pane is closed
- **THEN** the preview follows the most recently focused Files pane again

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
Preferences > Preview SHALL show, for each of Logs, Images, Video, Audio, PDF, Markdown, JSON, YAML, TOML, XML, HTML, CSV and Text, a collapsible group with a table of persisted filename patterns. Each pattern row is one case-insensitive full-filename glob (`*`, `?`, character classes or an exact name) that users can edit, add and remove. Groups SHALL be checked from top to bottom and the first match SHALL win; users SHALL be able to move groups up and down to change priority. A group with no rows disables filename matching for that type, not content detection. HTML and CSV SHALL use existing text rendering, not execute markup or introduce table rendering.

Defaults SHALL cover supported extensions and common system logs (`syslog`, `messages`, `secure`, `maillog`, `cron`, `debug`, `dmesg`), including gzip and numeric/date rotations. Rotation patterns SHALL match a numeric-starting suffix using globs, not an exact rotation-name parser. Settings are stored as an ordered list of `{type, patterns}` groups; the older one-string-per-type form SHALL migrate automatically and missing or unreadable groups SHALL use defaults. A restore action SHALL reset all groups and order.

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

### Requirement: Supported previews
The preview SHALL render: text and source with syntax highlighting; JSON, YAML, TOML and XML as raw highlighted text with an optional pretty-printed view; Markdown; png, jpeg, gif (animated), webp, bmp, ico and svg images; audio and PDF metadata; video with playback; a directory summary; and a hex dump for everything else.

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

HTML SHALL remain literal inert text and images SHALL remain labelled alt-text
placeholders. No link/image resource SHALL be fetched or opened.
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
The toolbar SHALL expose minus, percentage/reset and plus controls in every view
type, including text, Markdown, structured text, logs, images, hex, metadata,
folder summaries, thumbnails, multi-file cards and video.
Only that pane's content SHALL scale; pane chrome, other panes and the application
zoom preference SHALL remain unchanged. Scale SHALL survive changes of previewed
file within the same pane. Images SHALL retain Fit/Actual modes; choosing either
SHALL reset additional magnification. Zoom SHALL NOT increase file/decode limits.

A focused Preview SHALL accept Ctrl/Cmd+= (or +), Ctrl/Cmd+- and Ctrl/Cmd+0.
Existing Ctrl/Cmd+Shift zoom shortcuts SHALL target preview magnification when
a Preview owns the keyboard; elsewhere they retain application-wide behavior.
Dialogs, popups and focused text fields SHALL retain keyboard ownership.
Commands SHALL reject malformed parameters and invalid explicit panes.

#### Scenario: Zoom without changing the app
- **WHEN** a focused Preview is zoomed in, out or reset
- **THEN** its body changes size while the toolbar, Files panes, other previews,
  global zoom preference and egui zoom factor remain unchanged
- **AND** minimum/maximum scales are enforced and bad parameters return an error

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
Previews SHALL enforce size, line, dimension, frame and time limits, check image dimensions before allocating, never follow links or load external resources, and convert any decoder panic, error or timeout into an error card.

#### Scenario: Decompression bomb
- **WHEN** an image header declares more than 100 megapixels
- **THEN** it is not decoded and the card says it is too large to preview

#### Scenario: Corrupt media
- **WHEN** a decoder fails or panics
- **THEN** the pane shows "the file could not be previewed" and the app keeps running

### Requirement: Video playback is owned by the pane
A Preview pane SHALL play at most one video (silently; audio output is out of scope for now), starting automatically, controllable with play/pause and seek, decoded at a size fitted to the pane; leaving the file, switching the pane kind or closing the pane SHALL stop it and end the decoder process. Without a decoder available the pane SHALL show metadata and a hint instead.

#### Scenario: Pane closed while playing
- **WHEN** a Preview pane is closed during playback
- **THEN** the decoder child process no longer exists

### Requirement: Preview commands
`pane.preview.source`, `preview.zoom`, `preview.toggle_wrap`, `preview.toggle_pretty` and `preview.video.play`, `preview.video.seek` and `preview.video.fit` SHALL be commands with ids, menu paths and bad-parameter tests; they are no-ops unless the focused pane is a Preview.

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
