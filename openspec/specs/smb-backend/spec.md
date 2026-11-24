# smb-backend Specification

## Purpose
TBD - created by archiving change smb-backend. Update Purpose after archive.
## Requirements
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

### Requirement: Server root lists shares
`smb://host/` SHALL list the server's disk shares (no `$` admin shares) as folders, falling back to the shares connected so far when the server will not enumerate. Write operations on the server root SHALL fail with a typed error that tells the user to open a share first.

#### Scenario: Bare host
- **WHEN** the user navigates to `smb://nas/`
- **THEN** the pane lists the shares of `nas`, and creating a folder there reports "open a share first, e.g. smb://nas/<share>"

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

### Requirement: SMB attributes
`fm-fs-smb` SHALL implement `stat_full`, `set_attrs` and `attr_caps` for creation, access and write time and DOS flags (readonly, hidden, system, archive) via the share's basic-information set, on the `smb` lane with all handles closed explicitly. Security descriptor (owner/ACL) support SHALL be reported only if the `smb` crate exposes it safely; otherwise it is unsupported. Mode and uid/gid SHALL be reported unsupported.

#### Scenario: Times on a share
- **WHEN** a file is copied to a share with times preserved
- **THEN** the share reports the same creation and write time within SMB resolution

#### Scenario: Docker unavailable
- **WHEN** the Samba integration container cannot start
- **THEN** the attribute integration tests skip, unit tests still run

