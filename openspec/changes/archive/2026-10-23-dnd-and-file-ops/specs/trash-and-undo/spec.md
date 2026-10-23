## ADDED Requirements

### Requirement: Delete goes to the trash
`file.delete` SHALL move local items to the platform trash through `ops::trash`; it SHALL NOT call recursive removal, and when the trash is unavailable it SHALL fail with an actionable error rather than deleting permanently.

#### Scenario: Trash success
- **WHEN** a file is deleted
- **THEN** it leaves its directory and is recoverable from the platform trash

#### Scenario: No silent downgrade
- **WHEN** trashing fails or the location has no trash
- **THEN** the item is untouched and the error offers permanent delete via an explicit confirm

### Requirement: Permanent delete needs confirmation
`file.delete_permanent` SHALL require a confirm dialog stating the item count and that it is irreversible, and SHALL walk the tree bottom-up through `Vfs::remove`.

#### Scenario: Declined confirm
- **WHEN** the user cancels the confirm
- **THEN** nothing is removed

### Requirement: Undo stack
The session SHALL keep a bounded (100) stack of inverse records for trash, move, rename, mkdir and copy; `edit.undo` SHALL replay the newest, undoing a copy or mkdir by moving the created items to the trash.

#### Scenario: Undo rename
- **WHEN** a file is renamed and `edit.undo` runs
- **THEN** the original name is restored

#### Scenario: Stale record
- **WHEN** the target of an undo record changed or vanished
- **THEN** `edit.undo` returns an actionable error, drops the record and changes nothing

#### Scenario: Permanent delete is not undoable
- **WHEN** a permanent delete completes
- **THEN** no undo record is pushed
