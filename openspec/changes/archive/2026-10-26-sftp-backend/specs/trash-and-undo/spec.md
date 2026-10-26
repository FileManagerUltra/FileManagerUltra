## ADDED Requirements

### Requirement: Trash manager
File ▸ Manage Trash (`trash.open`) SHALL turn the focused pane into a Trash pane (pane type `trash`) listing the platform trash (name, original folder, deletion time, newest first) without blocking the UI. The user SHALL be able to restore selected items (`trash.restore`), delete selected items permanently (`trash.purge`) and empty the trash (`trash.empty`). Permanent removal SHALL require `confirm: true` and the pane SHALL ask for confirmation first. Listings SHALL be capped, with a truncation notice. On platforms whose trash cannot be enumerated (macOS) the commands SHALL return an `Unsupported` error.

#### Scenario: Emptying the trash
- **WHEN** the user chooses Empty Trash and confirms
- **THEN** `trash.empty {"confirm":true}` runs on a job lane and the listing refreshes when it finishes

#### Scenario: Unconfirmed purge
- **WHEN** `trash.purge` or `trash.empty` runs without `confirm: true`
- **THEN** it returns `InvalidParams` and removes nothing

### Requirement: Remote delete is permanent
Deleting on a remote (SFTP) location SHALL always be a permanent delete behind the confirmation dialog; `file.delete` on a remote selection SHALL return an error and never attempt to trash.

#### Scenario: Delete on SFTP
- **WHEN** `file.delete` is triggered with a remote selection
- **THEN** the permanent-delete confirmation opens instead of moving to the trash

## ADDED Requirements

### Requirement: Browse trashed folders
The Trash pane SHALL let the user view the contents of a trashed folder, either expanded inline under its row or by opening it (Back returns). Contents are read-only; restore and delete act on top-level entries.

#### Scenario: Expand a trashed folder
- **WHEN** the user expands a trashed folder
- **THEN** its files and sub-folders are listed beneath it

The user SHALL be able to select individual items inside a trashed folder, at any depth, and restore them to their original location; existing files are never overwritten.
