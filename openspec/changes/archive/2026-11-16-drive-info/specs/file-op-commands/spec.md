## ADDED Requirements

### Requirement: Properties shows the drive
For a single item that is the root of a local volume (from the cached volume list), Properties SHALL show a Drive section with the volume's name, filesystem, capacity, used and free space and a used-space bar, and SHALL NOT walk the volume to count its contents; Size SHALL show the used space. For any other single item it SHALL show the free space of the drive or server the item is on, when the backend reports it (local volumes, SFTP, SMB), read on the Properties background thread.

#### Scenario: Properties of a drive
- **WHEN** the user opens Properties on `D:\` (labelled Data, NTFS, 2 TB, 1.2 TB free)
- **THEN** the dialog shows Data (D:), NTFS, capacity 2 TB, used 0.8 TB, free 1.2 TB with a bar, immediately, without counting files

#### Scenario: Properties of a file on a share
- **WHEN** the user opens Properties on a file on `smb://nas/media`
- **THEN** the dialog shows how much space is free on the share
