## ADDED Requirements

### Requirement: What a copy cannot write is left out and listed
A copy or move SHALL leave out, not abort on, items inside the selected folders that cannot be written: special files (devices, FIFOs, sockets, hard links an archive cannot resolve) and names no folder can hold (control characters, separators, over-long names). Each left-out item SHALL count as skipped and be listed with its reason in the job's report (up to 1 000 by name). When the destination is a local folder on Windows, names Windows cannot store (`<>:"|?*`, trailing dots or spaces, device names such as `CON` or `nul.txt`) SHALL be written under a safe spelling (`a_b`, `CON_`, `nul_.txt`) instead of creating an alternate data stream or opening a device.

#### Scenario: FIFO inside a copied folder
- **WHEN** a copied folder contains a FIFO next to regular files
- **THEN** the files are copied, the FIFO is not opened, and the job report lists it as left out

#### Scenario: Colon in a name on Windows
- **WHEN** a file named `report:v2.txt` from an SFTP server or an archive is copied to a local folder on Windows
- **THEN** it is written as `report_v2.txt`, not as a stream of a file named `report`

### Requirement: A full destination stops the job
When a write fails because the destination is out of space (or over quota), the job SHALL stop instead of trying every remaining item, remove the partial temporary file, and fail with an actionable message naming the folder ("not enough free space for … free some space or choose another folder").

#### Scenario: Disk fills during a copy
- **WHEN** the destination fills up while the second of three files is written
- **THEN** the first file stays, no partial or temporary file is left, the third is not attempted, and the job fails with the free-space message
