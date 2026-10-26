## ADDED Requirements

### Requirement: SMB credentials and transport safety
SMB passwords SHALL live in memory only (never in `Prefs`, profiles, workspaces or logs) and SHALL NOT appear in any `FmError` display; a canary-secret test SHALL cover every SMB error path. Authentication SHALL be NTLM username/password or guest; other mechanisms SHALL fail with a message saying they are unsupported. A preference "Require signed or encrypted SMB" SHALL, when on, refuse sessions that negotiate neither. Remote size and entry caps apply as for SFTP.

#### Scenario: Canary password
- **WHEN** every SMB connect and I/O failure is formatted
- **THEN** none of the strings contains the canary password

#### Scenario: Require signing
- **WHEN** the preference is on and the server negotiates neither signing nor encryption
- **THEN** the connection is refused with an actionable message
