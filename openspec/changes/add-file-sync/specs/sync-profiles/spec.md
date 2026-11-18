## ADDED Requirements

### Requirement: Saved profiles
A sync profile SHALL store: name, left and right URIs, mode, comparison method and tolerance, filters, symlink policy, attribute policy, delete policy, `max_delete_percent`, conflict rule, watch settings. Profiles SHALL be created, saved, renamed, duplicated and deleted through `sync.profile.*` commands and persisted in preferences.

#### Scenario: Round trip
- **WHEN** a profile is saved and the app restarts
- **THEN** the profile loads with identical settings

#### Scenario: Unknown fields
- **WHEN** a profile file contains unknown or missing fields
- **THEN** it loads with defaults for the missing ones and does not crash

### Requirement: No secrets in profiles
Profiles SHALL store URIs without passwords or passphrases; remote credentials SHALL come from the existing connection store. Validation SHALL reject a URI containing `user:password@`.

#### Scenario: Password in URI
- **WHEN** a profile is saved with `sftp://u:secret@host/dir`
- **THEN** saving fails with an error that does not echo the password

### Requirement: Profile validation
A profile SHALL be rejected when left and right are the same folder, when one contains the other, when a policy is invalid for the target (Trash on remote), or when a URI exceeds the depth/length limits.

#### Scenario: Nested pair
- **WHEN** the right folder is inside the left folder
- **THEN** validation fails naming the overlap

### Requirement: Continuous sync while the app is open
A profile with watch enabled SHALL be monitored: local sides through debounced `notify` events, remote sides by polling at a profile interval (minimum 10 seconds). A detected change SHALL trigger a compare and, with auto-apply on, a run (keeping in sync applies changes automatically; there is no separate option). Turning Keep in sync on SHALL first compare, show what would be applied and start only after the user confirms (`sync.watch.confirm`); cancelling or editing the folders leaves nothing running or applied. Monitoring SHALL stop when the profile is stopped or the app closes; there is no scheduler and no background process.

#### Scenario: Local edit
- **WHEN** a file changes in a watched local folder with Keep in sync on and a clean plan
- **THEN** after the debounce the change is synced to the other side

#### Scenario: No overlapping runs
- **WHEN** changes arrive during a running sync
- **THEN** exactly one follow-up compare is queued and runs never overlap

### Requirement: Auto-apply is conservative
Auto-apply SHALL execute only a plan with no conflicts, no permanent deletes and no tripped deletion safeguard; otherwise it SHALL stop, keep the plan for review and show a notice.

#### Scenario: Conflict appears
- **WHEN** a watched change creates a conflict
- **THEN** nothing is auto-applied and the profile shows a pending-review state

### Requirement: Watch events never read inline
Watcher events SHALL only mark a profile dirty; they SHALL NOT call `read_dir` or any backend on the event thread.

#### Scenario: Event burst
- **WHEN** 10 000 events arrive in one second
- **THEN** one compare is scheduled after the debounce window
