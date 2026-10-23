## ADDED Requirements

### Requirement: Operations never escape or silently destroy
File operations and drop payloads SHALL be treated as hostile: sources and destinations are validated `VfsUri`s; operations never write outside the chosen destination directory, never overwrite without an explicit decision, never delete a source before a verified copy, and never downgrade trash to permanent delete silently.

#### Scenario: Traversal in a dropped name
- **WHEN** a payload entry name contains `..` or a separator
- **THEN** the item is rejected and nothing is written outside the destination

#### Scenario: Cancelled write leaves no partial file
- **WHEN** a copy is cancelled or fails mid-file
- **THEN** no destination file with partial content remains

#### Scenario: Errors stay redacted
- **WHEN** an operation fails on a URI carrying credentials
- **THEN** the displayed error contains no secret
