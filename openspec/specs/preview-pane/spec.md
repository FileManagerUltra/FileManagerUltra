# preview-pane Specification

## Purpose
TBD - created by archiving change pane-preview. Update Purpose after archive.
## Requirements
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
The renderer SHALL be chosen from magic bytes and text validity first and the extension only as a hint; unknown or binary content SHALL fall back to a hex dump with a metadata card.

#### Scenario: Misleading extension
- **WHEN** a PNG file is named `notes.txt`
- **THEN** it is previewed as an image

### Requirement: Supported previews
The preview SHALL render: text and source with syntax highlighting; JSON, YAML, TOML and XML as raw highlighted text with an optional pretty-printed view; Markdown; png, jpeg, gif (animated), webp, bmp, ico and svg images; audio and PDF metadata; video with playback; a directory summary; and a hex dump for everything else.

#### Scenario: Structured files open raw
- **WHEN** a minified `.json` file is previewed
- **THEN** it is shown exactly as stored, and a Pretty button is available in the preview header

#### Scenario: Pretty on demand
- **WHEN** the user presses Pretty (or runs `preview.toggle_pretty`)
- **THEN** the file is shown re-indented, pressing it again returns to the raw text, and selecting another file resets to raw

#### Scenario: Invalid structured file
- **WHEN** a `.json` file is not valid JSON
- **THEN** it is shown raw, and Pretty is disabled with a "not valid JSON" tooltip

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

#### Scenario: Choosing a log time range
- **WHEN** the user opens Time in the preview's expandable filter row, including in a narrow pane
- **THEN** the time dialog is centered in the application viewport
- **THEN** a separate window offers optional From and To dates via calendar pickers and hour/minute/second controls, labelled as UTC
- **AND** opening it restores applied bounds or suggests dates from the loaded log records
- **AND** editing fields or choosing calendar dates does not close the window or apply a filter
- **AND** Apply dispatches the time-range command, Clear removes the bounds, and Cancel or closing the window leaves the applied filter unchanged
- **AND** a reversed range shows an inline error and keeps the window open

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
