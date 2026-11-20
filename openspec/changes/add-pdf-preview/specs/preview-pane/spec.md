## MODIFIED Requirements

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

### Requirement: Hostile files are bounded
Previews SHALL enforce size, line, dimension, frame and time limits, check image dimensions before allocating, never follow links or load external resources, and convert any decoder panic, error or timeout into an error card. PDFs SHALL additionally follow the limits in `pdf-view` ("Hostile PDFs are bounded").

#### Scenario: Decompression bomb
- **WHEN** an image header declares more than 100 megapixels
- **THEN** it is not decoded and the card says it is too large to preview

#### Scenario: Corrupt media
- **WHEN** a decoder fails or panics
- **THEN** the pane shows "the file could not be previewed" and the app keeps running

#### Scenario: Corrupt PDF
- **WHEN** a file starts with `%PDF` but its structure is unreadable
- **THEN** the pane shows that the PDF could not be read, with its file information, and the app keeps running

### Requirement: Preview commands
`pane.preview.source`, `preview.zoom`, `preview.toggle_wrap`, `preview.toggle_pretty` and `preview.video.play`, `preview.video.seek` and `preview.video.fit` SHALL be commands with ids, menu paths and bad-parameter tests; they are no-ops unless the focused pane is a Preview. The PDF view, form and signing commands are specified in `pdf-view`, `pdf-forms` and `pdf-signing`.

#### Scenario: Bad source
- **WHEN** `pane.preview.source` is given a pane that is not a Files pane
- **THEN** it returns an error and the source is unchanged
