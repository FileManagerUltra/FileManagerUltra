## ADDED Requirements

### Requirement: Sync modes
`fm-sync` SHALL support three modes between a left and a right folder on any `Vfs` pair: **mirror** (target made identical to source, extras deleted), **update** (source copied to target when new or newer, nothing deleted) and **two-way** (changes propagate in both directions).

#### Scenario: Mirror
- **WHEN** the source has files A, B and the target has B (older), C and a mirror runs
- **THEN** A is copied, B is overwritten, C is deleted per the delete policy

#### Scenario: Update leaves extras and newer files
- **WHEN** the target has a file C not in the source and a file newer than the source copy
- **THEN** neither is deleted or overwritten and the newer one is flagged

### Requirement: Compare before change
A sync SHALL build a plan (list of items with a status and an action) without writing anything; execution SHALL only apply a plan the user (or auto-apply rules) accepted. Compare SHALL be cancellable and SHALL NOT read file content unless checksum comparison is selected.

#### Scenario: Dry run
- **WHEN** compare finishes
- **THEN** no file on either side has changed and the plan lists counts and byte totals per action

### Requirement: Comparison method
Files SHALL be compared by size and modification time with a configurable tolerance (default 2 seconds), optionally by streamed checksum when sizes match. Directories compare by existence and, when attribute sync is on, by attributes.

#### Scenario: FAT rounding
- **WHEN** two copies differ by 1 second in mtime and have the same size
- **THEN** they compare equal under the default tolerance

### Requirement: Two-way uses a snapshot
Two-way sync SHALL classify each path on each side as unchanged, new, changed or deleted relative to a persisted snapshot of the last sync. A one-sided change SHALL propagate. A change on both sides, or a change on one and a deletion on the other, SHALL be a conflict.

#### Scenario: Deletion propagates
- **WHEN** a file was synced, is then deleted on the left and unchanged on the right
- **THEN** the plan deletes it on the right per the delete policy

#### Scenario: Missing snapshot
- **WHEN** a two-way profile runs with no valid snapshot
- **THEN** missing files are copied both ways, differing files are conflicts and nothing is deleted

### Requirement: Conflicts default to skip
A conflict SHALL be planned as `Skip` and shown as a warning until the user chooses a direction for that item. The profile SHALL carry a `ConflictRule` whose only value in this release is `Skip`. Skipped conflicts SHALL remain conflicts in the snapshot so they are reported again.

#### Scenario: Both sides edited
- **WHEN** a file changed on both sides since the last sync
- **THEN** it is skipped, counted as a conflict, and a later run reports it again until resolved

### Requirement: Snapshot integrity
The snapshot SHALL be written atomically after a run and SHALL contain only paths that were synced or verified equal. A corrupt or unknown-version snapshot SHALL be treated as missing and reported.

#### Scenario: Crash mid-run
- **WHEN** a run is interrupted after half the actions
- **THEN** the next compare still sees the unfinished paths as pending

### Requirement: Delete policy
Each profile SHALL select `Permanent`, `Trash` (local targets only) or `Versioning(folder, retention)`. Versioning SHALL move the file to `folder/<timestamp>/<relative path>` and prune by age, count or total size after a run. Defaults SHALL be `Trash` for local targets and `Permanent` for SMB/SFTP targets. Deletion SHALL go through `ops::trash` for Trash, never `remove_dir_all`.

#### Scenario: Versioning retention
- **WHEN** retention is 2 versions and a third version of a file is archived
- **THEN** the oldest archived version is removed

#### Scenario: Trash on remote
- **WHEN** a profile with `Trash` targets an SFTP folder
- **THEN** the profile is rejected as invalid with a message naming the supported policies

### Requirement: Deletion safeguards
Execution SHALL require explicit confirmation when the plan contains permanent deletes, when deletes exceed `max_delete_percent` (default 50) of the target's items, or when the source scan is empty and the target is not. Auto-apply SHALL never bypass these checks.

A target folder that does not exist SHALL be treated as empty by compare and SHALL be created (with its parents) by the first run that copies into it. A missing source folder (Update, Mirror) or both folders missing SHALL be an error, and the source SHALL never be created.

#### Scenario: Unmounted source
- **WHEN** a mirror source lists zero entries and the target has 1000
- **THEN** the run stops with a confirmation-required result and deletes nothing

### Requirement: Filters
A profile SHALL support include and exclude globs on root-relative, `/`-normalized paths. Excludes win. Excluded paths SHALL be ignored by compare and SHALL NOT be deleted by mirror.

#### Scenario: Excluded extra
- **WHEN** the target holds `build/x.o` and the profile excludes `build/**`
- **THEN** mirror does not delete it

### Requirement: Symlink policy
A profile SHALL select `Preserve` (default; recreate the link, never follow), `Follow` (copy the target, cycle-guarded) or `Skip`. A link SHALL never be dereferenced outside the sync root under `Preserve`.

#### Scenario: Symlink loop
- **WHEN** `Follow` meets a link pointing at its own ancestor
- **THEN** the cycle is reported and not descended into

### Requirement: Attribute policy
A profile SHALL select the attribute groups to preserve and what to do on failure: warn (default), error (skip the file) or ignore. Unpreserved attributes SHALL be listed per file in the run log.

#### Scenario: Warn policy
- **WHEN** owner cannot be set on 3 files and the policy is warn
- **THEN** the files are copied, the run succeeds with 3 warnings

### Requirement: Bounded and non-blocking
Scans SHALL cap entry count and depth, reject names failing `validate_name`, and run on the job lanes of their scheme; the UI thread SHALL never perform sync I/O. Execution SHALL report throttled progress and be cancellable, and SHALL reuse the copy pipeline (temp file then rename, resume offsets, conflict channel not needed because conflicts are planned).

#### Scenario: Cancel during compare
- **WHEN** the user cancels while a 100 000 entry scan is running
- **THEN** workers stop, no plan is produced and no file changes

### Requirement: Commands
Sync behaviour SHALL be exposed as commands (`sync.open`, `sync.compare`, `sync.run`, `sync.cancel`, `sync.set_action`) with documented params; malformed params SHALL return `Err`, never panic.

#### Scenario: Bad params
- **WHEN** `sync.run` receives a profile id that does not exist
- **THEN** it returns an error and state is unchanged

### Requirement: Background presence
The sync window SHALL be minimizable (`sync.minimize`, `sync.restore`). A minimized, watching or running sync SHALL appear in the Activity panel with its status and Show/Cancel/Stop actions, and the status bar SHALL show a one-line status (progress, conflicts needing a decision, or "Syncing <name>" while keeping in sync). Closing the window of a profile that keeps in sync SHALL minimize it instead of hiding it silently. The comparison grid SHALL be sortable (`sync.sort`) by folder order, name, size, modified time and action, in both directions.

#### Scenario: Keep in sync while minimized
- **WHEN** a profile with "Keep in sync" is minimized
- **THEN** it keeps comparing, and the Activity panel and status bar show its state until it is stopped

### Requirement: Two workflows
The sync window SHALL offer a guided workflow and an advanced workflow, switchable at any time while no sync is running via `sync.view`. A new sync SHALL open in the guided workflow; a saved profile SHALL open in the advanced one. Every button in both workflows SHALL say what it does, and both SHALL state that nothing is changed until the user presses the start button.

#### Scenario: Guided questions
- **WHEN** a new sync is opened
- **THEN** the wizard asks, one screen per step: folders, goal (update, mirror, two-way and the direction), how to delete, how to compare, what to leave out, and whether to keep in sync (with the profile name)
- **AND** each answer is a profile edit and each move is `sync.wizard.go`; moving forward requires two valid folders

#### Scenario: Review before starting
- **WHEN** the wizard reaches the review step
- **THEN** the folders are compared with the Changes filter and the list of changes is shown, with the text that nothing has been changed yet
- **AND** a one-time sync is started with "Start: synchronize now", while keep-in-sync waits for "Start and keep in sync" (the compare-then-confirm rule applies)
- **AND** going back abandons a pending keep-in-sync start

#### Scenario: Advanced workflow
- **WHEN** the user switches to Advanced
- **THEN** all options are shown on one screen with one-line explanations of Compare, Synchronize and Keep in sync

