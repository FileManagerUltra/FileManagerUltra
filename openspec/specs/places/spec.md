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
Windows SHALL exclude fixed drive letters backed by internal physical disks.
Removable drives, network mappings, USB-backed fixed drives and virtual drive
letters without physical partition associations SHALL remain eligible. Missing
classification metadata SHALL NOT implicitly include a fixed drive.
Windows removable drive-letter slots without capacity or a volume serial SHALL
not count as mounted media and SHALL disappear from Devices after unplugging.
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
`devices.refresh`. Pending or failed scans SHALL preserve the previous list.
Successful results SHALL be applied even when slow; a scan still pending after
10 seconds SHALL report one diagnostic without spawning replacement workers.
Automatic polling SHALL wait its interval after completion before scanning again.

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

#### Scenario: Two USB drives are connected
- **WHEN** Eject is selected for one volume
- **THEN** only that volume is targeted and its row shows Ejecting while pending
- **AND** other removal buttons are temporarily disabled with an explanation
- **AND** Windows keeps its Shell request alive until the target disappears or reports failure, never reporting success merely for submitting the request

### Requirement: Devices lists removable media and user-mounted volumes
Every Files pane SHALL show a collapsible Devices section above Places listing mounted removable media and user-mounted volumes, including data disks, network mounts and mounted VeraCrypt volumes, regardless of their removable flag. System volumes, pseudo filesystems and implementation-only mounts SHALL be excluded. Each entry SHALL show only its label and removal action; mount location and total/free capacity (or explicitly unavailable capacity) SHALL move to the directory Preview and remain in its hover tooltip. The section SHALL be hidden entirely while no eligible volume is mounted. Clicking an entry opens it in that pane (`nav.open`); entries are drop targets like other Places; Go → Devices SHALL list the same entries. The collapse state is stored in the pane layout and persisted with the workspace.

#### Scenario: Desktop infrastructure is not a device
- **WHEN** Linux document-portal (`fuse.portal`) or GVFS bridge (`fuse.gvfsd-fuse`) mounts exist
- **THEN** they are absent from Devices and Go > Devices, while user-mounted FUSE filesystems remain eligible

#### Scenario: No device
- **WHEN** no eligible volume is mounted
- **THEN** no Devices heading or placeholder is shown

#### Scenario: Windows internal data drives
- **WHEN** Windows has internal fixed disks alongside USB, network and virtual volumes
- **THEN** internal physical disk drive letters are excluded
- **AND** removable drives, network mappings, USB-backed fixed drives and virtual drive letters without partition associations remain eligible
- **AND** missing classification metadata does not implicitly include fixed drives

#### Scenario: Plugging in a stick
- **WHEN** a USB stick is mounted while the app is focused
- **THEN** it appears in Devices within 5 seconds without user action and a status notice names it

#### Scenario: Mounting a VeraCrypt volume
- **WHEN** VeraCrypt mounts a volume while the app is focused, even if the OS reports it as non-removable
- **THEN** it appears within 5 seconds with its mount location and capacity
- **AND** a locked, unmounted container is not listed as a mounted device

#### Scenario: Unplugging
- **WHEN** a listed device disappears while the app is focused
- **THEN** its row is removed within 5 seconds and a pane that was showing it reports the folder as unavailable instead of crashing
- **AND** an empty Windows removable drive-letter slot without capacity or volume serial is not retained as a device

### Requirement: Device detection never blocks
Device scanning SHALL run on a dedicated single-flight worker isolated from file-operation lanes, SHALL be rate-limited (2 s focused, 10 s unfocused, immediate on focus regain), and SHALL keep the previous list while a scan is pending or fails. Successful results SHALL be applied regardless of duration. A scan still pending after 10 seconds SHALL report one diagnostic; completed successful scans SHALL NOT be reported as failures. Automatic polling SHALL wait its interval after completion before scanning again. The UI thread SHALL only read the cached list. A hung scan SHALL NOT cause replacement scan workers to accumulate.

#### Scenario: Discovery takes longer than one second
- **WHEN** a background scan successfully completes after several seconds
- **THEN** its mounted volumes update Devices without a timeout error
- **AND** the next automatic scan waits the polling interval after completion

#### Scenario: A mount hangs
- **WHEN** reading one device stalls
- **THEN** the UI stays responsive and the last known list remains

### Requirement: Eject
`devices.eject {uri}` SHALL unmount and, where the OS supports it, power off the device mounted at `uri`. The uri MUST equal a currently listed device mount, otherwise the command returns an error. Panes showing the device SHALL first navigate to Home. Failure (for example a busy device) SHALL surface the OS tool's message as a notice and leave the device listed. Eject SHALL be available from the row's button and context menu and from the command registry.

#### Scenario: Eject a busy stick
- **WHEN** a copy is writing to the device
- **THEN** eject is refused with an actionable message and the copy continues

#### Scenario: Two sticks are connected
- **WHEN** Eject is clicked on one
- **THEN** only the selected mount is targeted and its row shows Ejecting
- **AND** other removal actions are disabled with an explanation until completion
- **AND** Windows retains its Shell request while waiting for confirmed removal or explicit failure

#### Scenario: Malformed params
- **WHEN** `devices.eject` receives a missing, non-string or unlisted uri
- **THEN** it returns an error and changes nothing

### Requirement: Dismount is distinct from hardware eject
`devices.dismount {uri}` SHALL detach only a currently listed volume with a supported dismount capability, without powering off its host disk. Verified VeraCrypt-owned mounts SHALL use a VeraCrypt-specific adapter; generic hardware eject SHALL NOT be used as a substitute. When ownership or a safe adapter cannot be established, browsing SHALL remain available and Dismount SHALL be disabled with an explanation. Dismount SHALL never force removal or request credentials. Active application transfers SHALL prevent removal before panes are retargeted or tools run; tool failures SHALL be reported explicitly.

#### Scenario: Unsupported VeraCrypt dismount
- **WHEN** a mounted VeraCrypt volume is browseable but no safe dismount adapter is available
- **THEN** it remains visible and the UI directs the user to dismount it in VeraCrypt
- **AND** no hardware power-off or generic eject is attempted

#### Scenario: Dismount a supported volume
- **WHEN** the user chooses Dismount on a supported volume with no active application transfers
- **THEN** the engine uses its verified non-forced dismount adapter and rescans after completion
- **AND** it reports failure rather than success if the operation cannot be confirmed

