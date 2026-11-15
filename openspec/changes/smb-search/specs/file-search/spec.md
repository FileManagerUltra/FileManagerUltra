## MODIFIED Requirements

### Requirement: Search is bounded and safe
The walker SHALL NOT follow symlinks, SHALL stop at 10 000 results or depth 64, SHALL skip files whose first 8 KiB contain a NUL when matching content, and SHALL read at most 64 MiB per file. Local folders, SFTP servers and SMB shares SHALL be searchable; SFTP and SMB use one walker thread of their own, never a job lane, so a hung server blocks only that search. The Network and Servers folders and an SMB server root (which lists shares, not files) SHALL be refused with a message saying to open a share or server first. Local searches SHALL use at most 8 threads and SHALL NOT read metadata of entries that do not match.

#### Scenario: Symlink loop and binary files
- **WHEN** a tree contains a symlink cycle and binary files and a content search runs
- **THEN** the symlink is not followed and binary files are skipped
- **AND** the search ends once 10 000 results are found

#### Scenario: Search an SMB share
- **WHEN** the user searches `smb://nas/media` for `*.mkv`
- **THEN** matching files in the share and its subfolders are listed, as for an SFTP folder

#### Scenario: Nothing to search
- **WHEN** the user starts a search in the Network folder or at `smb://nas/`
- **THEN** no search starts and the message says to open a share (or server) to search it
