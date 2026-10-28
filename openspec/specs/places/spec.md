# places Specification

## Purpose
Quick access to the user's standard folders and bookmarks from every Files pane and the Go menu.

## Requirements
### Requirement: Places lists standard folders and bookmarks
Every Files pane SHALL show two collapsible sections above the folder tree — Places (the user's existing standard folders: Home, Desktop, Documents, Downloads, Music, Pictures, Videos, each with its own icon) and Bookmarks (with a `+` button that bookmarks the current folder, and a hint when empty). The collapse state of each section SHALL be stored in the pane's layout state and persisted with the workspace; Go → Places SHALL list the same entries. Clicking an entry opens it in that pane (`nav.open`).

#### Scenario: Open a bookmark
- **WHEN** the user clicks a bookmark in Places
- **THEN** the pane navigates to it and the entry is highlighted while the pane shows it

### Requirement: Bookmarks are commands and are persisted safely
`bookmark.add {uri?, name?}` (Ctrl+D, default: the focused folder), `bookmark.remove {uri}`, `bookmark.rename {uri, name}` and `bookmark.move {uri, by | to}` (by ±1 or to an index; dragging a bookmark row reorders it) SHALL edit `Prefs.bookmarks`, which is saved by the owner-only prefs writer. At most 200 bookmarks of at most 100-character names and 4096-byte uris are kept; invalid entries in a hand-edited prefs file are dropped on load. Adding an existing bookmark is a no-op with a status message.

#### Scenario: Junk in the prefs file
- **WHEN** the prefs file contains a bookmark with an empty name or an unparsable uri
- **THEN** that bookmark is dropped on load and the others are kept

### Requirement: Places are drop targets
Dropping files on a Places or Bookmarks row SHALL copy/move them there, exactly like dropping on a folder-tree row.

#### Scenario: Collapse survives a restart
- **WHEN** the user collapses Bookmarks in a pane and restarts the app
- **THEN** that pane's Bookmarks section is still collapsed and the other panes are unaffected

### Requirement: Devices lists removable and user-mounted volumes
Files panes SHALL show a collapsible Devices section above Places only when eligible
volumes are mounted. Entries SHALL include removable drives, user data volumes,
mounted network volumes and mounted virtual volumes such as VeraCrypt, regardless
of the removable flag; system, pseudo and implementation-only mounts are excluded.
Linux document-portal (`fuse.portal`) and GVFS bridge (`fuse.gvfsd-fuse`) mounts
SHALL be excluded, while user-mounted FUSE filesystems remain eligible.
Rows SHALL show only the volume label and removal action, open through `nav.open`,
and act as drop targets. Mount location and capacity SHALL remain available in the
hover tooltip and the directory Preview, not as extra sidebar lines.
Go → Devices SHALL mirror the list.
Collapse state SHALL persist independently per pane with the workspace.

#### Scenario: An encrypted volume mounts
- **WHEN** VeraCrypt mounts a volume while the app is focused
- **THEN** it appears within 5 seconds even if reported as non-removable
- **AND** locked containers are not listed as mounted devices

### Requirement: Non-blocking mount detection
Discovery SHALL poll off the UI and file-operation lanes on a single-flight worker
every 2 seconds focused / 10 seconds unfocused, refreshing on focus regain and with
`devices.refresh`. Failed or over-one-second scans SHALL preserve the previous list,
report a diagnostic, and never accumulate replacement workers while hung.

#### Scenario: A volume disappears
- **WHEN** a listed volume is disconnected while the app is focused
- **THEN** its row disappears within 5 seconds and connection status is reported
- **AND** affected panes show unavailable instead of an empty backing directory
- **AND** new application operations below its old mount point are refused until reconnection

### Requirement: Capability-aware volume removal
`devices.eject {uri}` and `devices.dismount {uri}` SHALL accept only an exact listed
mount with the requested capability, revalidate its OS identity, refuse active file
transfers before side effects, and move affected open/parked panes Home. Removal
SHALL be non-forced, run on the local lane, surface errors and verify disappearance
before reporting success. Eject MAY power off only a verified external disk without
other mounted volumes; Dismount SHALL never power off the host disk.

#### Scenario: Unsupported encrypted volume
- **WHEN** the volume manager or safe dismount method cannot be verified
- **THEN** browsing remains available but Dismount is disabled with an explanation
- **AND** users are directed to VeraCrypt/OS tools rather than generic hardware eject

#### Scenario: Invalid or busy removal
- **WHEN** removal params are malformed, unlisted, stale, unsupported or target an active transfer
- **THEN** the command reports an error without executing a removal tool
