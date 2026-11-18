## ADDED Requirements

### Requirement: Full attribute model
`fm-vfs` SHALL define `Attrs` (creation, modification and access time, mode, uid/gid with names, ACL blob, xattrs, DOS flags; every field optional) and `AttrCaps` describing which of them a backend can read and write. `Vfs` SHALL expose `stat_full`, `set_attrs` and `attr_caps` with default implementations that report no capabilities, so a backend that does not implement them still compiles and returns `FmError::Unsupported` naming the operation.

#### Scenario: Backend without attribute support
- **WHEN** `set_attrs` is called on a backend using the default implementation
- **THEN** it returns `Unsupported` and no panic occurs

#### Scenario: Capabilities are reported
- **WHEN** `attr_caps` is called on the local backend on Linux
- **THEN** mtime, atime, mode, owner, group, ACL and xattrs are reported writable and creation time is reported unsupported

### Requirement: Best-effort preservation with a report
`set_attrs` SHALL apply every requested attribute it can and return an `AttrReport` listing, per attribute, whether it was applied, unsupported or failed (with an actionable reason such as permission denied). One failing attribute SHALL NOT prevent the others from being applied or the file content from being kept.

#### Scenario: chown without privilege
- **WHEN** a non-root user copies a file with owner preservation requested
- **THEN** times and mode are applied, the report lists owner and group as `PermissionDenied`, and the file exists with its content

### Requirement: Apply order
Attributes SHALL be applied after content is written, ownership before mode, and directory attributes after all of their children have been written.

#### Scenario: Directory mtime survives
- **WHEN** a directory with mtime T is copied with files inside it
- **THEN** the destination directory's mtime equals T afterwards

### Requirement: Attribute selection
A caller SHALL choose which attribute groups to apply (times, mode, owner/group, ACL, xattrs, flags). Groups not selected SHALL be left untouched on the destination.

#### Scenario: Times only
- **WHEN** only the times group is selected
- **THEN** the destination keeps its own mode and owner

### Requirement: Hostile attribute data
Attribute values read from a backend are untrusted: xattr names and counts, blob sizes and owner names SHALL be capped, names containing NUL or invalid UTF-8 SHALL be skipped with a warning, and a size field SHALL never size an allocation.

#### Scenario: Oversized xattr
- **WHEN** a file reports an xattr larger than the cap
- **THEN** that xattr is skipped, reported, and the rest of the attributes are processed
