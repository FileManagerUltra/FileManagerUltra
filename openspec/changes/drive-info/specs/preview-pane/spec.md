## MODIFIED Requirements

### Requirement: Volume information in directory previews
The directory Preview SHALL show the closest containing listed volume's name,
mount path, filesystem, capacity and free space, plus used space when known.
A folder that is itself the root of a local volume (a Windows drive such as
`C:\`, `/`, or any mount point), listed under Devices or not, SHALL show the
same information for that volume. It SHALL use cached data only (the Devices
list and a cached list of every local volume refreshed in the background),
reflect cache changes, explicitly show unavailable capacity and preserve
unsupported-removal explanations. Thumbnail mode SHALL keep this information in
an expandable section.

#### Scenario: Preview inside a mounted volume
- **WHEN** a mounted directory or its descendant is previewed
- **THEN** volume information appears above its folder summary without extra sidebar lines
- **AND** a directory outside listed volumes that is not a volume root has no volume section

#### Scenario: Preview a system drive
- **WHEN** a Windows user selects `C:` in This PC
- **THEN** Preview shows its name, `C:\`, the filesystem (for example NTFS), capacity, free space and a used-space bar above the folder summary
