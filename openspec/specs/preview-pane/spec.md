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

### Requirement: Log mode
When the previewed file is a log (by name, location or content), the preview SHALL offer a Log mode that parses nginx/Apache access and error logs, Magento and Pimcore (Monolog) logs, Magento `var/report` JSON, PHP-FPM/PHP errors and generic text; colours records by level or HTTP status; supports level filter, text/regex search, time-range filter, folding of stack traces and a column view for access logs; reads `.gz` rotations; and offers Follow (live tail). Files of any size SHALL open with the end visible immediately, with older records loaded on demand in bounded chunks and memory capped.

#### Scenario: Huge log opens at the tail
- **WHEN** a multi-gigabyte log is previewed
- **THEN** the last records are shown without reading the whole file and the UI never blocks

#### Scenario: Follow survives rotation
- **WHEN** Follow is on and the file is truncated or replaced
- **THEN** the view reindexes and keeps following without error

#### Scenario: Hostile regex
- **WHEN** the user enters an invalid or oversized regex
- **THEN** the filter shows an error and the view is unchanged

#### Scenario: Multi-line record
- **WHEN** a stack trace follows an error line
- **THEN** it belongs to that record and can be folded

### Requirement: Narrow toolbar and clean text copy
When the preview toolbar options do not fit the pane width they SHALL move into a menu button at the right. In text previews the line-number gutter SHALL NOT be selectable, and copying selected text SHALL yield the text without line numbers or extra blank lines. Ctrl+C/X/V in Preview and Trash panes SHALL act on selected text, not on files.

#### Scenario: Copy text
- **WHEN** the user selects lines in a text preview and copies
- **THEN** the clipboard holds exactly those lines

