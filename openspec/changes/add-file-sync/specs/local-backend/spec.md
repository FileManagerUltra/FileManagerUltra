## ADDED Requirements

### Requirement: Local backend reads and writes full attributes
`fm-fs-local` SHALL implement `stat_full`, `set_attrs` and `attr_caps` with safe APIs only: mtime/atime (and creation time where the platform allows) through `std::fs::FileTimes`, mode, uid/gid and xattrs (including `system.posix_acl_*` as opaque blobs) through `rustix` on Unix. Symlinks SHALL be handled without following them. On Windows, owner, group and DACL SHALL be read and written through `fm-winsec` as an opaque, size-capped security descriptor, with failures from missing privileges reported per attribute. Platform gaps (Linux creation time; ACL/owner between Windows and Unix pairs) SHALL be reported in `attr_caps`, not as errors.

#### Scenario: Round trip on a temp tree
- **WHEN** a file's times, mode and an xattr are read with `stat_full` and applied to another file
- **THEN** the second file reports the same values, or the report lists the attribute as unsupported on a filesystem without it

#### Scenario: Symlink attributes
- **WHEN** `set_attrs` targets a symlink
- **THEN** the link's own times are set and its target is untouched

### Requirement: Windows security descriptors are isolated
`fm-winsec` SHALL be the only crate besides the approved exceptions that contains `unsafe` for security descriptors; it SHALL deny `unsafe_code` crate-wide and allow it only in one module, validate inputs in safe code and cap descriptor size.

#### Scenario: Oversized descriptor
- **WHEN** a descriptor larger than the cap is passed to `set_attrs`
- **THEN** it is rejected with an error before any API call

#### Scenario: Missing privilege
- **WHEN** setting the owner requires a privilege the process lacks
- **THEN** the report lists owner as `PermissionDenied` and the other attributes are applied
