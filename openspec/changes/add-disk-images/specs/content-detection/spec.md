## ADDED Requirements

### Requirement: Open as archive or image
Every file SHALL offer "Open as archive or image…" (context menu and File menu), which scans the file for known signatures of archives, compressed streams, file systems, partition tables and firmware headers at any offset, and lists each hit with offset (hexadecimal and decimal), format, size when known and a short detail. Choosing a hit SHALL open it as a folder from that offset, as a location that survives history, tabs and restarts.

#### Scenario: SquashFS inside a firmware blob
- **WHEN** the user runs Open as… on `firmware.bin` that holds a uImage header, a kernel and a SquashFS at 0x1A0000
- **THEN** the list shows the uImage header, the compressed kernel and `SquashFS 4.0 · xz · 0x1A0000`, and opening the SquashFS shows its root

### Requirement: Scans are bounded and cancellable
The scan SHALL run as a job off the UI thread with progress and cancel, SHALL stop after 64 GiB or 10 000 hits, SHALL validate each signature's header before reporting it, and SHALL hide hits inside an already found item unless "Show nested hits" is ticked. Scanning a remote file larger than 4 GiB SHALL ask first.

#### Scenario: Cancel a long scan
- **WHEN** the user cancels a scan of a 20 GB disk image halfway
- **THEN** the hits found so far stay listed and can be opened

#### Scenario: False positive rejected
- **WHEN** a text file contains the bytes `hsqs` without a valid SquashFS superblock after them
- **THEN** no SquashFS hit is listed

### Requirement: Forcing a format
The dialog SHALL let the user open the file as a chosen format at a given offset (typed in hex or decimal) without scanning; an invalid choice SHALL show an actionable error naming the format.

#### Scenario: Force FAT at an offset
- **WHEN** the user opens `dump.bin` as FAT at offset `0x8000` where a FAT boot sector is
- **THEN** the pane lists the FAT root

### Requirement: Content detection commands
`archive.scan { uri }` and `archive.open_as { uri, format, offset }` SHALL be commands with ids, menu paths and bad-parameter tests.

#### Scenario: Malformed open_as
- **WHEN** `archive.open_as` receives an unknown format, an offset past the end of the file or a negative offset
- **THEN** it returns `Err` and nothing is mounted
