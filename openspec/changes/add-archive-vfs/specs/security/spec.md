## ADDED Requirements

### Requirement: Archives are untrusted and bounded
Archive indexes and members SHALL be treated as hostile input: at most 1 000 000 entries, 4 096-byte names and 64 path levels per archive; names normalised so that `..`, absolute paths, drive letters and backslashes cannot leave the archive root; control characters shown escaped; every decoder run inside `catch_unwind` with panics turned into errors; decompressed reads bounded by the declared size and the spool cap. Exceeding a limit SHALL fail the mount or the read with an actionable error, never crash or allocate without bound. Spooled data SHALL live only in the owner-only private temporary folder and be removed on unmount and quit.

#### Scenario: Fuzzed archives
- **WHEN** the fuzz corpus (truncated, bit-flipped and oversized-header zip, tar, 7z and ISO files) is mounted and listed
- **THEN** every case returns a listing or an error, none panics, and memory stays under the test's cap

#### Scenario: Million-entry archive
- **WHEN** a zip declares 5 000 000 entries
- **THEN** mounting stops with "too many entries (limit 1 000 000)" without reading them all

### Requirement: Split-archive parts are found by name, in one folder
The parts of a split archive SHALL be resolved only from the entered part's own folder on the same backend, by name pattern, capped at 10 000 parts; names or offsets read from archive content SHALL never point the app at other files or folders.

#### Scenario: Hostile volume name in a RAR header
- **WHEN** a RAR volume's header names its next volume `../../etc/passwd`
- **THEN** the app resolves parts only from the folder listing and reports a missing part instead of reading that path

### Requirement: Nothing inside an archive is executed
A program inside an archive SHALL NOT be run: `file.open` in run mode SHALL be refused for archive locations, and opening a member with its default application SHALL extract it to the owner-only private temporary folder as a non-executable file first.

#### Scenario: Run an exe from a zip
- **WHEN** `file.open` with `mode: "run"` and `confirm: true` is dispatched for `tools.zip/setup.exe`
- **THEN** it returns `Err` and no process starts

### Requirement: Archive passwords never persist
Archive passwords, for reading and for creating or re-encrypting archives, SHALL be held in memory only while needed (reading: while the archive is mounted; creating: for the job), zeroised on unmount, job end and quit, and never written to disk, logs, notices, `Display` output, command history, workspaces or the diagnostic log. New encryption SHALL default to AES-256; weaker ZipCrypto SHALL only be used when the user explicitly chooses it.

#### Scenario: Password not in the workspace
- **WHEN** a workspace is saved while an encrypted zip is open with its password entered
- **THEN** the workspace file contains the archive's location but no password, and reopening asks for the password again

### Requirement: The RAR helper runs a fixed program with an argument array
The RAR helper SHALL run only 7-Zip or unrar from the found or configured path, with a fixed argument array where the archive path and member name are single arguments after `--`, without a shell, with a timeout and a cap on output size. Member names from the tool's output SHALL be parsed as untrusted text with the archive limits. A password for an encrypted RAR SHALL be passed only to that child process (`-p`), which the password prompt SHALL state.

#### Scenario: Hostile member name
- **WHEN** a RAR contains a member named `a; rm -rf ~` or `-o+ x`
- **THEN** it is listed as a name, and reading it passes it as a single argument after `--` with no effect beyond reading that member
