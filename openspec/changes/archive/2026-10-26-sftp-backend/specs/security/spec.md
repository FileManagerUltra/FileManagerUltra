## ADDED Requirements

### Requirement: Remote credentials and host keys are handled safely
SFTP passwords and key passphrases SHALL be held in zeroizing secret types for the lifetime of the connection only, SHALL NOT implement `Debug`/`Display` of their value, and SHALL never be written to disk, logs, the profile store or an error message (a test SHALL format every `FmError` variant built from a failing connect with a known secret and assert it is absent). The known-hosts file SHALL be created owner-only (`0600`, directory `0700`) and bounded in size and entries; a corrupt line SHALL be skipped, not trusted. Remote listings and transfers SHALL obey the entry, depth and size caps before allocation.

#### Scenario: Secret never surfaces
- **WHEN** a connect fails with the password `hunter2`
- **THEN** no error string, job label, notice or state file contains it

#### Scenario: Known-hosts permissions
- **WHEN** a host key is first saved
- **THEN** the file mode is `0600` and unreadable to other users
