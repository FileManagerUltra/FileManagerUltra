## ADDED Requirements

### Requirement: Sync state is owner-only and secret-free
Sync snapshots, profile files and sync logs SHALL be created owner-only (`0600` files, `0700` directories) and SHALL NOT contain passwords, passphrases or `user:password@` URIs; no `FmError::Display` or log line produced by sync SHALL include a credential.

#### Scenario: Snapshot permissions
- **WHEN** a snapshot is written on Unix
- **THEN** its mode is `0600` and its directory is `0700`

#### Scenario: Redaction
- **WHEN** a sync against a remote with a wrong password fails
- **THEN** the message and log contain no part of the password

### Requirement: Destructive sync actions are guarded
Mass deletion, permanent deletion and mirroring from an empty source SHALL require explicit user confirmation, and auto-apply SHALL NOT bypass it. Symlinks under the `Preserve` policy SHALL NOT be followed outside the sync root, and sync-created paths SHALL be validated so a hostile remote name cannot escape the target root.

#### Scenario: Traversal name
- **WHEN** a remote listing contains an entry named `../../etc/x`
- **THEN** it is dropped and never written or deleted
