## ADDED Requirements

### Requirement: SMB implements the whole Vfs contract
`fm-smb` SHALL implement every `Vfs` method for `smb://[user@]host[:port]/share/path` URIs using the pure-Rust `smb` crate: `read_dir`, `stat`, `open_read`, `open_read_at`, `open_write`, `create_dir`, `remove`, `rename`, `same_device` and `space`. The first path segment is the share. Entries SHALL carry kind, size and modified time with the same semantics as the local backend. A method the server or protocol cannot do (symlink creation) SHALL return `FmError::Unsupported` naming the operation, never panic.

#### Scenario: Round trip against Samba
- **WHEN** a file is written, listed, statted, read back, renamed and removed through the backend against a Samba share
- **THEN** each step returns the expected entries and bytes and the final listing is empty

#### Scenario: Resume offset
- **WHEN** `open_read_at(uri, 5)` is called on a 10-byte file
- **THEN** the reader yields the last 5 bytes

### Requirement: One code path on every platform
The SMB backend SHALL need no system library and SHALL behave identically on Linux, macOS and Windows; it SHALL NOT use native UNC paths or `libsmbclient`.

#### Scenario: Clean build
- **WHEN** the workspace is built on a machine without Samba libraries
- **THEN** it builds and the SMB backend works

### Requirement: A share is required
Operations on `smb://host/` without a share SHALL return a typed error with a hint to enter a share name; share enumeration is not part of this capability.

#### Scenario: Bare host
- **WHEN** the user navigates to `smb://nas/`
- **THEN** the pane shows "Enter a share name, e.g. smb://nas/media" and no request is sent as a listing

### Requirement: Remote data is bounded
A directory listing SHALL obey the local entry cap and fail with an actionable error past it, enforced while directory chunks arrive; entry names SHALL pass `validate_name` (a hostile name is dropped); a reported size SHALL never be used to pre-allocate; reads and writes SHALL stream in fixed-size chunks with an idle timeout and cancellation between chunks.

#### Scenario: Hostile listing
- **WHEN** the server returns more entries than the cap, a name containing a path separator and an entry with a `u64::MAX` size
- **THEN** the listing fails at the cap, the bad name is skipped, and no allocation is sized from the reported size

### Requirement: Errors are actionable and redacted
Failures SHALL map to `FmError` (`NotFound`, `PermissionDenied`, `AlreadyExists`, an authentication error, "share not found", transport errors) with messages built from fixed text plus host, share and path; no error SHALL contain a password or `user:password@` form.

#### Scenario: Wrong password
- **WHEN** the password is wrong
- **THEN** the form shows "Authentication failed", keeps its fields, and no error string contains the password

### Requirement: Signing and encryption are reported
After connecting, the backend SHALL expose whether the session is signed and/or encrypted. The UI SHALL show it in the connection tooltip, and an unsigned, unencrypted session SHALL be labelled as such.

#### Scenario: Unsigned session
- **WHEN** the server negotiates neither signing nor encryption
- **THEN** the tooltip reads "NOT signed"
