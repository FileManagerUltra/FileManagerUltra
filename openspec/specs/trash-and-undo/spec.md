# trash-and-undo Specification

## Purpose
TBD - created by archiving change dnd-and-file-ops. Update Purpose after archive.
## Requirements
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

### Requirement: Panes leave a deleted folder
When a trash or permanent delete finishes and a Files pane shows a removed folder (or a folder inside one), the pane SHALL move to the folder its tree now highlights, if that is not removed too, and otherwise to the nearest parent that still exists; it SHALL NOT keep showing a "not found" error. A delete that failed or was cancelled part way SHALL leave the pane where it is. The move SHALL be decided from the job's own sources, without filesystem calls on the UI thread.

#### Scenario: Delete the shown folder from the tree
- **WHEN** the folder shown in the listing is moved to the trash from the tree, and the tree highlight moves to the next folder
- **THEN** the listing shows that next folder

#### Scenario: Delete a parent of the shown folder
- **WHEN** a folder above the shown one is deleted and the tree highlights nothing else
- **THEN** the pane shows the deleted folder's parent

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

### Requirement: Browse trashed folders
The Trash pane SHALL let the user view the contents of a trashed folder, either expanded inline under its row or by opening it (Back returns). Contents are read-only; restore and delete act on top-level entries.

#### Scenario: Expand a trashed folder
- **WHEN** the user expands a trashed folder
- **THEN** its files and sub-folders are listed beneath it

The user SHALL be able to select individual items inside a trashed folder, at any depth, and restore them to their original location; existing files are never overwritten.

