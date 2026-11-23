## ADDED Requirements

### Requirement: File-system images are untrusted and bounded
Image readers SHALL treat every structure as hostile: directory and extent trees depth-capped, NTFS MFT records capped at 10 million, JFFS2 nodes capped at 5 million, FAT cluster chains checked for loops, SquashFS tables bounded by the image size, and every reader run inside `catch_unwind`. Our own parsers (JFFS2, CramFS, the signature scanner) SHALL have fuzz tests in CI. Special files inside images SHALL never be opened, and images SHALL be read only; no image is ever written.

#### Scenario: Looping FAT chain
- **WHEN** a FAT image has a cluster chain that points back to itself
- **THEN** reading the file stops with "corrupt FAT chain" and the app keeps running

#### Scenario: Fuzzed JFFS2
- **WHEN** the JFFS2 fuzz corpus (truncated, bit-flipped, overlapping nodes) is mounted and listed
- **THEN** every case returns a listing or an error and none panics or exceeds the memory cap

### Requirement: Scans never execute or trust found content
"Open as…" SHALL only read the file; found items SHALL be mounted through the same bounded readers as normal archives and images, and firmware headers SHALL be parsed for offsets only.

#### Scenario: Executable found by a scan
- **WHEN** a scan finds a zip at an offset that contains `run.sh`
- **THEN** the zip can be browsed, but `run.sh` is not run, and opening it extracts a non-executable copy to the private temporary folder
