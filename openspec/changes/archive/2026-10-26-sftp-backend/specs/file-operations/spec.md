## ADDED Requirements

### Requirement: Operations work across backends
Copy and move SHALL work for local ↔ SFTP and SFTP ↔ SFTP by streaming `open_read`/`open_write` with the usual progress, cancel and conflict handling. A move within one authority SHALL use `Vfs::rename`; a move across backends SHALL copy and then remove the source only after the copy fully succeeded. A failed or cancelled transfer SHALL NOT leave a partial destination file behind.

#### Scenario: Cancel mid-upload
- **WHEN** a local-to-SFTP copy of a large file is cancelled
- **THEN** the partial remote file is removed and the source is untouched

#### Scenario: Cross-backend move failure
- **WHEN** the copy half of a move fails
- **THEN** the source is not removed and the failure is reported

#### Scenario: Same-authority move
- **WHEN** files are moved between two directories on one server
- **THEN** a rename is used and no bytes are transferred
