## ADDED Requirements

### Requirement: SFTP attributes
`fm-fs-sftp` SHALL implement `stat_full`, `set_attrs` and `attr_caps` using SFTP attribute get/set for mode, access and modification time and uid/gid. Creation time, ACLs and xattrs SHALL be reported unsupported. A server refusing `setstat` SHALL yield a per-attribute failure in the report, not a failed transfer. Remote values SHALL be bounded like every other remote data.

#### Scenario: Times and mode preserved
- **WHEN** a file is copied to the in-process SFTP server with times and mode preserved
- **THEN** `stat_full` on the remote file returns the same mtime (to second resolution) and mode

#### Scenario: Setstat refused
- **WHEN** the server rejects the attribute change
- **THEN** the file stays uploaded and the report lists the refused attributes
