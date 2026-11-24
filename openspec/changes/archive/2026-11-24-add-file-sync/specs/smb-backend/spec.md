## ADDED Requirements

### Requirement: SMB attributes
`fm-fs-smb` SHALL implement `stat_full`, `set_attrs` and `attr_caps` for creation, access and write time and DOS flags (readonly, hidden, system, archive) via the share's basic-information set, on the `smb` lane with all handles closed explicitly. Security descriptor (owner/ACL) support SHALL be reported only if the `smb` crate exposes it safely; otherwise it is unsupported. Mode and uid/gid SHALL be reported unsupported.

#### Scenario: Times on a share
- **WHEN** a file is copied to a share with times preserved
- **THEN** the share reports the same creation and write time within SMB resolution

#### Scenario: Docker unavailable
- **WHEN** the Samba integration container cannot start
- **THEN** the attribute integration tests skip, unit tests still run
