## ADDED Requirements

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
"Compress to…" SHALL create a new archive from the selection in a dialog offering format (zip, tar.gz, tar.xz, 7z, and tar.zst when the encoder passes its quality check), compression level (fast, normal, best), an optional password for zip (AES-256) and 7z (AES-256 with encrypted names), and the target name and folder (default: next to the selection, named after the single item or the folder). It SHALL run as a file job with progress and cancel and SHALL never overwrite an existing file without the usual conflict prompt.

#### Scenario: Zip with a password
- **WHEN** the user compresses two folders to `share.zip` with a password
- **THEN** `share.zip` is created, its members are AES-256 encrypted, and opening it in the app asks for the password

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
`archive.extract { uri, dest? }` and `archive.compress { sources, target, format, level, password? }` SHALL be commands with ids, menu paths (File menu and the context menu) and bad-parameter tests; the password parameter SHALL never appear in logs, notices or errors.

#### Scenario: Malformed compress params
- **WHEN** `archive.compress` receives an unknown format, an empty source list or a level of 99
- **THEN** it returns `Err` and no file is created
