## ADDED Requirements

### Requirement: SMB credentials and transport safety
SMB passwords SHALL live in memory only (never in `Prefs`, profiles, workspaces or logs) and SHALL NOT appear in any `FmError` display; a canary-secret test SHALL cover every SMB error path. Authentication SHALL be NTLM username/password or guest; other mechanisms SHALL fail with a message saying they are unsupported. A preference "Require encryption" (`smb_require_encryption`) SHALL, when on, make every new SMB connection demand encryption and refuse servers that cannot provide it; the connect popup SHALL start with that checkbox set from the preference. Plain sessions are always signed or labelled "NOT signed". Remote size and entry caps apply as for SFTP.

#### Scenario: Canary password
- **WHEN** every SMB connect and I/O failure is formatted
- **THEN** none of the strings contains the canary password

#### Scenario: Require encryption
- **WHEN** the preference is on and the server cannot encrypt
- **THEN** the connection is refused with an actionable message

### Requirement: Temporary opened files and vendored crates
A remote file opened with its default application SHALL be downloaded to an owner-only (0700) per-process folder as a non-executable file; remote `run` mode and downloads over 1 GiB SHALL be refused. The app SHALL delete its own folders on quit and folders older than 24 hours at start. Vendored crates (`vendor/sspi`, `vendor/smb-transport`) SHALL be listed with their deviations in `vendor/FM-VENDORED.md`.

#### Scenario: Open a remote file
- **WHEN** the user opens a file on a remote location
- **THEN** a job with progress downloads it to the private folder, then the default app opens it, and the folder is removed when the app quits
