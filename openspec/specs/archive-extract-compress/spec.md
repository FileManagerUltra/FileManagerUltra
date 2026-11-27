# archive-extract-compress Specification

## Purpose
TBD - created by archiving change add-archive-vfs. Update Purpose after archive.
## Requirements
### Requirement: Extract here and Extract to
For a selected archive, "Extract here" SHALL extract it into a new folder named after the archive (without its extensions) next to it, and "Extract to…" SHALL extract it into a folder chosen with the path picker. Both SHALL run as file jobs with progress, cancel and the usual conflict handling, and SHALL work for every readable format, on every backend.

#### Scenario: Extract here
- **WHEN** the user chooses Extract here on `site.tar.gz` in `/home/me/Downloads`
- **THEN** a job creates `/home/me/Downloads/site/` with the archive's contents

#### Scenario: Name already taken
- **WHEN** `/home/me/Downloads/site` already exists and the user chooses Extract here on `site.tar.gz`
- **THEN** the job extracts into `site (2)` instead of merging into the existing folder

### Requirement: Copying out of an archive is extraction
Copying or moving entries from inside an archive to a location outside it (F5, F6, drag and drop, clipboard paste) SHALL use the normal copy pipeline. Moving out of a writable archive SHALL stage the removal from the archive; moving out of a read-only archive SHALL be offered as copy only.

#### Scenario: Drag a folder out of a 7z
- **WHEN** the user drags `tools.7z/bin` onto another Files pane
- **THEN** `bin` is copied there with its contents, with progress in the job panel

### Requirement: Compress to
"Compress to…" SHALL create a new archive from the selection in a dialog offering format (zip, tar.gz, tar.xz, 7z, and tar.zst when the encoder passes its quality check), compression level (fast, normal, best), the target name and folder (default: next to the selection, named after the single item or the folder) and splitting into numbered parts of a chosen size (100 MB, 700 MB, 4 GB, or a custom size of at least 64 KiB). It SHALL run as a file job with progress and cancel and SHALL never overwrite an existing file without the usual conflict prompt.

#### Scenario: Split into parts
- **WHEN** the user compresses a 950 MB folder to 7z with 100 MB parts
- **THEN** `name.7z.001` to `name.7z.010` are created, and entering `name.7z.001` shows the folder's contents

### Requirement: Compress with encryption
The Compress dialog SHALL offer a password, typed twice, for zip and 7z. Zip SHALL be encrypted with AES-256 by default, and with traditional ZipCrypto only when the user chooses it, with the choice marked as weak and for old tools only. 7z SHALL be encrypted with AES-256 and SHALL encrypt file names unless the user turns that off. For tar formats the password field SHALL be disabled with the reason. The password SHALL never be pre-filled, remembered, logged or shown in notices or errors.

#### Scenario: Zip with a password
- **WHEN** the user compresses two folders to `share.zip` with a password
- **THEN** `share.zip` is created, its members are AES-256 encrypted, and opening a file in it in the app asks for the password

#### Scenario: Encrypted 7z hides names
- **WHEN** the user compresses to `secret.7z` with a password and "Encrypt file names" on
- **THEN** another tool listing `secret.7z` without the password sees no file names

#### Scenario: Passwords do not match
- **WHEN** the two password fields differ
- **THEN** the Compress button is disabled and the dialog says the passwords do not match

### Requirement: A selection is added to an archive in one job
Compress to…, "Add to *name*.zip", "Add to *name*.7z", "Add to archive…", dropping onto an archive file and copying into a pane showing an archive SHALL take the pane's whole selection (Shift-click ranges, Ctrl-click picks, Select all, mixed files and folders) and add it in one job with one progress display, one conflict policy and one summary. Folders SHALL be added recursively, including empty folders; each selected item SHALL be stored under its own name relative to the pane's folder (or the search root for Find Files results); symlinks SHALL be stored as links, never followed. The Compress dialog SHALL show the number of files and folders and the total size of the selection.

#### Scenario: Shift-click range into a new zip
- **WHEN** the user Shift-clicks from `a.txt` to `photos/` (5 files and 2 folders) in `/home/me/project` and chooses Compress to… zip
- **THEN** one job creates `project.zip` holding the 5 files and both folders with their contents at its root, and the dialog had shown "5 files, 2 folders"

#### Scenario: One-click add
- **WHEN** the user selects three files and chooses "Add to report.zip" (named after the first item when only one is selected, after the folder otherwise)
- **THEN** the zip is created next to them without a dialog

#### Scenario: Add to an existing archive
- **WHEN** the user selects two folders and a file and chooses "Add to archive…" and picks `backup.7z`
- **THEN** one job adds all three to `backup.7z`, asks once per name that already exists in the archive (with "apply to all"), saves the archive, and the original is replaced only after the new archive was verified

#### Scenario: Drop a selection onto an archive file
- **WHEN** the user drags a selection of 20 files onto `site.zip` in another pane's listing
- **THEN** the 20 files are added to `site.zip` and it is saved in one job

#### Scenario: Read-only target
- **WHEN** the user drops a selection onto `debian.iso`
- **THEN** the drop is refused with "ISO images are read-only" and nothing changes

### Requirement: Join parts
"Join parts" SHALL concatenate an archive's numbered parts into one file next to them (named without the part number), as a file job with progress, cancel and the conflict prompt; it SHALL refuse when a part is missing, naming it, and SHALL NOT be offered for zip spanned sets or RAR volumes.

#### Scenario: Join a split tar.gz
- **WHEN** the user chooses Join parts on `dump.tar.gz.002` with parts 001 to 004 present
- **THEN** `dump.tar.gz` is created with the parts' bytes in order, and the parts are kept

### Requirement: Extraction never escapes the destination
Extraction SHALL NOT write outside the chosen destination: entry names are normalised, links resolving outside the archive are not followed, hard links to entries outside the archive are skipped, device files and FIFOs are skipped, setuid and setgid bits are cleared, and names invalid on the destination OS are renamed by the existing rules. Skipped entries SHALL be listed in the job's summary.

#### Scenario: Path traversal entry
- **WHEN** a tar contains `../../.bashrc` and the user extracts it to `out/`
- **THEN** the file is written as `out/.bashrc` (clamped at the archive root) and nothing is written outside `out/`

#### Scenario: Device node in a tar
- **WHEN** a tar contains `/dev/sda` as a block device entry
- **THEN** extraction skips it and the summary lists it as skipped

### Requirement: Decompression bombs are stopped
Extraction of more than 1 GiB at a ratio above 1000:1 SHALL ask before continuing; a member whose decompressed output exceeds its declared size SHALL stop with an error; extraction SHALL stop with an actionable error when the destination runs out of space.

#### Scenario: Zip bomb
- **WHEN** the user extracts a 42 KB zip that declares 4 PB of content
- **THEN** the job asks before writing beyond 1 GiB, and stopping leaves no partial file

### Requirement: Extract and compress commands
`archive.extract { uri, dest? }`, `archive.compress { sources, target, format, level, password?, encryption?, encrypt_names?, split? }`, `archive.compress_quick { sources, format }`, `archive.add { sources, archive, dir? }` and `archive.join { uri, target? }` SHALL be commands with ids, menu paths (File menu and the context menu) and bad-parameter tests; the password parameter SHALL never appear in logs, notices or errors.

#### Scenario: Malformed compress params
- **WHEN** `archive.compress` receives an unknown format, an empty source list, a level of 99, a password for tar.gz or a part size below 64 KiB
- **THEN** it returns `Err` and no file is created

#### Scenario: Malformed add params
- **WHEN** `archive.add` receives an empty source list, a target that is not an archive, a read-only archive, or a source that is the target archive itself
- **THEN** it returns `Err` and the archive is unchanged

