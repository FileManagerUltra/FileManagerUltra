## MODIFIED Requirements

### Requirement: Content is identified by sniffing
The renderer SHALL use the first matching configured filename association, with text validity protecting text-based renderers from binary input. Unmatched files SHALL use magic bytes and content detection (including the `{\rtf` header of Rich Text Format, see `rtf-preview`) without restoring removed filename associations; unknown binary content SHALL fall back to a hex dump with a metadata card.

#### Scenario: Misleading extension
- **WHEN** a PNG file is named `notes.txt`
- **THEN** it is previewed as an image

#### Scenario: RTF recognised by content
- **WHEN** a file named `letter.doc` starts with `{\rtf1`
- **THEN** it is previewed as an RTF document

### Requirement: Configurable preview filename filters
Preferences > Preview SHALL show, for each of Logs, Images, Video, Audio, PDF, Markdown, RTF, SVG, JSON, YAML, TOML, XML, HTML, CSV and Text, an entry in a type list (with its pattern count) beside a list of the selected type's persisted filename patterns, each list with an icon toolbar (types: move up/down; patterns: add, remove, edit). Each pattern is one case-insensitive full-filename glob (`*`, `?`, character classes or an exact name); a pattern is edited in place (toolbar, a second click or F2; Enter or leaving the field saves, Escape cancels without closing Preferences), Delete removes the focused one, Up and Down move the selection in either list while it has keyboard focus, Tab, Shift+Tab, Left and Right move the keyboard between the two lists (Tab continues to the next control when the selected type has no patterns), and clicking a row gives it keyboard focus. While Preferences (or another dialog that owns the keyboard) is open, arrow keys SHALL NOT move the cursor of the Files listing, thumbnail grid, preview or playlist behind it. A short explanation of the pattern syntax is shown below the lists. Groups SHALL be checked from top to bottom and the first match SHALL win; users SHALL be able to move types up and down to change priority. A group with no rows disables filename matching for that type, not content detection. HTML SHALL use existing text rendering and never execute markup; CSV SHALL open in the table view (see `table-preview`).

Defaults SHALL cover supported extensions and common system logs (`syslog`, `messages`, `secure`, `maillog`, `cron`, `debug`, `dmesg`), including gzip and numeric/date rotations. Rotation patterns SHALL match a numeric-starting suffix using globs, not an exact rotation-name parser. SVG defaults to `*.svg` and `*.svgz` and comes before XML. RTF defaults to `*.rtf` and comes after Markdown. Settings are stored as an ordered list of `{type, patterns}` groups; the older one-string-per-type form SHALL migrate automatically and missing or unreadable groups SHALL use defaults. Stored settings without an SVG group SHALL gain one directly above XML, and the default `*.svg` pattern SHALL be removed from XML; other user patterns SHALL be kept. Stored settings without an RTF group SHALL gain one directly below Markdown (or above Text when there is no Markdown group). A restore action SHALL reset all groups and order.

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

#### Scenario: Older settings gain the RTF type
- **WHEN** preferences saved before the RTF type existed are loaded
- **THEN** RTF appears directly below Markdown with `*.rtf`, and every other pattern and the order of the other types are unchanged

### Requirement: Supported previews
The preview SHALL render: text and source with syntax highlighting; JSON, YAML, TOML and XML as raw highlighted text with an optional pretty-printed view; Markdown; Rich Text Format documents as formatted text with their pictures (see `rtf-preview`); png, jpeg, gif (animated), webp, bmp and ico images; SVG drawings rendered as a picture with a switch to their source; PDF documents as rendered pages (see `pdf-view`), with form filling and signing (see `pdf-forms`, `pdf-signing`); CSV and TSV files as a sortable, filterable table (see `table-preview`); audio metadata; video with playback; a directory summary; and a hex dump for everything else.

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

#### Scenario: RTF opens formatted
- **WHEN** an `.rtf` file is previewed
- **THEN** it is shown as formatted text, not as a hex dump or raw control words, and its RTF text is one toggle away
