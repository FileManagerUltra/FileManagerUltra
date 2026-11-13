## ADDED Requirements

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
