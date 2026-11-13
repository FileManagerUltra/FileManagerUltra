## ADDED Requirements

### Requirement: Volume information in directory previews
The directory Preview SHALL show the closest containing listed volume's name,
mount path, filesystem, capacity and free space, plus used space when known.
It SHALL use cached device data only, reflect cache changes, explicitly show
unavailable capacity and preserve unsupported-removal explanations. Thumbnail
mode SHALL keep this information in an expandable section.

#### Scenario: Preview inside a mounted volume
- **WHEN** a mounted directory or its descendant is previewed
- **THEN** volume information appears above its folder summary without extra sidebar lines
- **AND** a directory outside listed volumes has no volume section

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
