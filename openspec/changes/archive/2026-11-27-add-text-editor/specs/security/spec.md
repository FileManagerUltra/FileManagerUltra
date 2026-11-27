## ADDED Requirements

### Requirement: Editor and parsers handle hostile input safely
Syntax parsers (tree-sitter) and the editor buffer SHALL treat file content as untrusted. Parsing SHALL run off the UI thread with a size threshold, a time budget and cancellation; exceeding any of them SHALL fall back to plain text, never crash and never block a frame. Grammar crates SHALL be limited to the curated, feature-gated set, and the workspace's own code SHALL remain free of `unsafe`. Buffer operations SHALL bound-check positions, cap undo memory, cap regular-expression size and run time, and cap allocation from file-declared sizes. Credentials SHALL never be written into buffers, temp files or messages; temporary files created for saving SHALL be owner-only and removed on failure. Editing SHALL NOT follow symlinks to write outside the file the user opened without the same symlink rules as `file-operations`.

#### Scenario: Pathological nesting
- **WHEN** a JSON file with 1 million nested brackets is opened in Edit mode
- **THEN** the editor stays responsive, colouring falls back to plain text if the budget is exceeded, and nothing panics

#### Scenario: Failed save cleans up
- **WHEN** a save fails after the temporary sibling was created
- **THEN** the temporary file is removed and the original file is unchanged

#### Scenario: Temp file permissions
- **WHEN** a local file is saved through a temporary sibling
- **THEN** the temporary file is created owner-only and the final file keeps the original permissions
