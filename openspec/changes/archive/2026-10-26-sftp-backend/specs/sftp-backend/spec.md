## ADDED Requirements

### Requirement: SFTP implements the whole Vfs contract
`fm-sftp` SHALL implement every `Vfs` method for `sftp://user@host[:port]/path` URIs: `read_dir`, `stat`, `open_read`, `open_read_at`, `open_write`, `create_dir`, `remove`, `rename`, `create_symlink`, `same_device` and `space`. Entries SHALL carry kind, size, modified time and symlink target, with the same semantics as the local backend; a method the server cannot do SHALL return `FmError::Unsupported` naming the operation, never panic.

#### Scenario: Round trip against a test server
- **WHEN** a file is written, listed, statted, read back, renamed and removed through the backend against an in-process SFTP server
- **THEN** each step returns the expected entries and bytes and the final listing is empty

#### Scenario: Resume offset
- **WHEN** `open_read_at(uri, 5)` is called on a 10-byte file
- **THEN** the reader yields the last 5 bytes

### Requirement: The runtime never runs on the caller's thread
`fm-sftp` SHALL own its tokio runtime and bridge each blocking `Vfs` call with `Handle::block_on` from a job-lane worker. Calling a `Vfs` method from within the runtime itself SHALL return an error instead of deadlocking, and no other crate SHALL depend on tokio.

#### Scenario: Slow server does not block local work
- **WHEN** an SFTP listing stalls
- **THEN** local-lane jobs and frames continue and only the `sftp` lane is occupied

### Requirement: Remote data is bounded
A directory listing SHALL obey the same entry cap as local reads and fail with an actionable error past it (never a silent truncate), enforced while packets arrive; entry names SHALL pass `validate_name` before use (a hostile name is dropped, not trusted); a reported size SHALL never be used to pre-allocate; reads and writes SHALL stream in fixed-size chunks.

#### Scenario: Hostile listing
- **WHEN** the server returns a million entries, a name containing `/` and an entry with a `u64::MAX` size
- **THEN** the listing fails at the cap, the bad name is skipped, and no allocation is sized from the reported size

### Requirement: Errors are actionable and redacted
Failures SHALL map to `FmError` variants (`NotFound`, `PermissionDenied`, `AlreadyExists`, `Backend`) with a message the user can act on; no variant's `Display` SHALL contain a password, passphrase or `user:password@` form.

#### Scenario: Permission denied
- **WHEN** the user lists a directory the server refuses
- **THEN** the pane shows a permission error naming the path and the session stays usable
