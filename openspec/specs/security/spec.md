# security

## Purpose

Security baseline: hostile input, no secrets on disk, owner-only state files, no unsafe, symlink containment. Later changes extend this capability.
## Requirements
### Requirement: Hostile input never crashes or escapes
All parsers and entry points that touch attacker-controlled data — `VfsUri` parsing, command parameters, directory listings (local and, from change 4, remote), and drop payloads — SHALL reject malformed input with `Err`, never panic, never allocate unbounded memory, and never treat input as a trusted path. Directory reads are capped at a bounded entry count; names and metadata from listings are rendered as data, never interpreted as markup or paths.

#### Scenario: Adversarial URIs
- **WHEN** `VfsUri` parsing is fuzzed/pro property-tested with inputs containing `..` segments, NUL bytes, overlong components (≥ 4 KiB), non-UTF-8 bytes, missing schemes, `file://` with a host, repeated `//`, and 10 000-character paths
- **THEN** every case returns `Err(InvalidUri)` or a normalized safe `Ok`, and no case panics or takes more than the bounded time/size budget

#### Scenario: Absolute paths stay absolute and contained
- **WHEN** a parsed `VfsUri::parent()` or `join()` chain is applied to any hostile path
- **THEN** the result is either `None` or a URI that still parses and is still absolute (no `..` in the normalized form)

#### Scenario: Command parameters are untrusted
- **WHEN** the command fuzz suite feeds every registered command wrong-typed, oversized (≥ 1 MiB string), empty and deeply-nested parameters
- **THEN** each call returns `Ok`/`Err`, the process survives, and no single call allocates more than the configured cap

#### Scenario: Hostile directory content
- **WHEN** a directory fixture contains more than the configured entry cap, 4 KiB component names, control characters, non-UTF-8 bytes and special files (fifo/device/socket)
- **THEN** reads either return a bounded capped `Err` or a bounded `Ok`, tree and listing render every entry as inert data without panic or unbounded allocation, and special files are never opened as regular files

#### Scenario: Capped reads, explicit failures
- **WHEN** a local or in-memory backend is asked to read a directory with more than `MAX_DIR_ENTRIES` entries
- **THEN** the read fails with an actionable error naming the cap — the user is never silently shown a truncated listing

### Requirement: Secrets are never written to disk
The scaffolded preferences file SHALL contain only non-secret settings (theme, window state). Passwords, passphrases, private keys and session tokens SHALL NOT be serializable into it, and error/reporting paths SHALL redact credentials.

#### Scenario: Prefs schema has no secret slots
- **WHEN** the `Prefs` type and its serialized form are inspected by test
- **THEN** no field can hold a password/passphrase/token (the type is closed: theme + window fields only)
- **AND** saved window geometry contains only bounded numeric size, position and scale-factor fields, using the existing owner-only atomic preferences writer

#### Scenario: Redaction in errors
- **WHEN** a backend error message is constructed from a URI or connection string containing `user:password@host`
- **THEN** the resulting `Display` output contains no password (unit test over `FmError::Backend` formatting)

#### Scenario: Nothing sensitive in logs
- **WHEN** the app logs or prints an error anywhere in the scaffold
- **THEN** no credential-shaped string is emitted (checked by the same redaction test on every `FmError` variant's `Display`)

### Requirement: Application state files are owner-only
Any file the application writes for state (`prefs.toml` now; host-key and profile files later) SHALL be created with owner-only read/write permissions on Unix (mode `0600`, directory `0700`) and the platform-equivalent protection on Windows.

#### Scenario: Prefs file permissions
- **WHEN** `engine::prefs::save` creates or overwrites `prefs.toml` on Unix
- **THEN** the file's mode is `0600` and the containing directory is `0700` (test creates a temp config dir and asserts modes)

#### Scenario: Corrupt or hostile file content
- **WHEN** `prefs.toml` contains invalid TOML, absurdly large values (≥ 10 MiB), or a billion-field document
- **THEN** load returns defaults, bounded time/memory, never panics and never truncates/appends to the file

### Requirement: No unsafe code, no unchecked trust in dependencies
Our crates SHALL contain no `unsafe` (workspace-wide forbid), and every dependency added later that performs FFI or network I/O MUST be justified in `docs/architecture.md` and isolated in its L0 backend crate.

#### Scenario: Unsafe is structurally impossible
- **WHEN** any workspace crate enables `unsafe` outside the forbid-exception process
- **THEN** the build fails (`unsafe_code = "forbid"` in workspace lints)

#### Scenario: Dependency additions are auditable
- **WHEN** a change adds a dependency that does network or FFI work
- **THEN** the change updates the dependency table in `docs/architecture.md` (review checklist item; enforced by change review, not tooling)

### Requirement: Symlinks cannot be used to escape a destination
Operations that write into a directory (copy, move, mkdir) SHALL resolve symlinks explicitly and refuse to write through a symlink that leaves the intended destination tree — the rule is specified here and exercised by `ops` tests from change 3 onward.

#### Scenario: Symlinked destination directory
- **WHEN** a copy targets a directory that is a symlink pointing outside the parent tree
- **THEN** the operation fails with `Err(FmError::…)` rather than following the link (implemented with tests in change 3; the policy is fixed now)

#### Scenario: Symlinked file inside the tree
- **WHEN** a source entry is a symlink to a file inside the same tree
- **THEN** the copy copies the symlink's target contents (or the link itself, per documented policy) and never silently follows it outside the root

### Requirement: Operations never escape or silently destroy
File operations and drop payloads SHALL be treated as hostile: sources and destinations are validated `VfsUri`s; operations never write outside the chosen destination directory, never overwrite without an explicit decision, never delete a source before a verified copy, and never downgrade trash to permanent delete silently.

#### Scenario: Traversal in a dropped name
- **WHEN** a payload entry name contains `..` or a separator
- **THEN** the item is rejected and nothing is written outside the destination

#### Scenario: Cancelled write leaves no partial file
- **WHEN** a copy is cancelled or fails mid-file
- **THEN** no destination file with partial content remains

#### Scenario: Errors stay redacted
- **WHEN** an operation fails on a URI carrying credentials
- **THEN** the displayed error contains no secret

### Requirement: Terminal child hygiene
The terminal child SHALL inherit the user's environment minus app-internal variables (`FM_*`, `CARGO_*`, `RUST_BACKTRACE`), run as the current user only, and be terminated on pane close and app exit. Terminal output and input MUST NOT be written to logs, devlog or state files, and `Prefs` stores no terminal content (only the `terminal_shell` choice: `default`/`powershell`/`cmd`). The shell executable SHALL be resolved only from absolute locations (`%SystemRoot%`, `%COMSPEC%` when absolute, absolute `PATH` entries), never the working directory, and paths typed into a shell SHALL be quoted for that shell's dialect so a file name cannot inject commands.

#### Scenario: No content on disk
- **WHEN** a terminal session prints text and the app saves state
- **THEN** the state files contain none of that text

#### Scenario: Injection via directory name
- **WHEN** `term.cd_here` targets a directory named `x; rm -rf ~`
- **THEN** the name is quoted so it is a single literal argument

### Requirement: Previews are untrusted-input safe
File content shown in a Preview SHALL be treated as hostile: bounded reads, header checks before allocation, no network access, no execution of embedded content, decoder panics contained, and decoder child processes (video) started without a shell and with fixed arguments, the file passed as a single argument after `--`/via `file:` protocol guard.

#### Scenario: Hostile filename to the decoder
- **WHEN** a video file is named `-i evil.mp4` or contains shell metacharacters
- **THEN** the decoder receives it as one argument and no other file or command is touched

### Requirement: Starting programs is explicit and shell-free
Activating a program SHALL require an explicit user choice and explicit `confirm: true` on `file.open`; the spawn SHALL use the OS process API with the file as one absolute path argument (no shell, no argument string interpolation), inherit no pipes, run in its own process group and be detached from the file manager's lifetime. Activation of a remote uri SHALL be refused rather than downloaded and executed. The UI SHALL never classify an entry as a document and then silently run it.

#### Scenario: Program cannot start by accident
- **WHEN** any code path dispatches `file.open` for a program without both `mode: "run"` and `confirm: true`
- **THEN** it returns an error and no process starts (typed tests cover the refusal)

#### Scenario: Remote uri is refused
- **WHEN** `file.open` is dispatched for a non-local uri
- **THEN** it returns `Err(Unsupported)` and nothing is spawned or downloaded

### Requirement: Thumbnail cache files are owner-only
The on-disk thumbnail cache SHALL be created in the platform cache directory with owner-only permissions on Unix (files `0600`, directory `0700`, written via a temp file + atomic rename) and SHALL be treated as regenerable: corrupt, foreign-versioned or oversized cache files are ignored, never loaded into the UI, never executed. The cache SHALL NOT contain anything beyond decoded image pixels and derived file identity (uri, mtime, size).

#### Scenario: Cache permissions
- **WHEN** the thumbnail cache writes its first file on Unix
- **THEN** the directory mode is `0700` and the PNG mode is `0600` (test asserts both)

#### Scenario: Hostile cache entry
- **WHEN** a cache file is truncated, has the wrong version, or is larger than a plausible thumbnail
- **THEN** it is ignored and the thumbnail is regenerated from the source file

### Requirement: Saved workspaces are private and bounded
`workspaces.json` SHALL hold only pane trees, locations and per-pane view settings — never credentials, terminal scrollback or command history. It SHALL be written atomically with owner-only permissions (`0600`, directory `0700`) through the same writer as `prefs.toml`, and SHALL be size-capped before it is read.

#### Scenario: Oversized or hostile file
- **WHEN** `workspaces.json` exceeds the size cap, is malformed, or holds duplicate or oversize pane trees
- **THEN** unreadable entries are skipped with a notice and the app starts with a built-in single pane, never panicking

### Requirement: Remote credentials and host keys are handled safely
SFTP passwords and key passphrases SHALL be held in zeroizing secret types for the lifetime of the connection only, SHALL NOT implement `Debug`/`Display` of their value, and SHALL never be written to disk, logs, the profile store or an error message (a test SHALL format every `FmError` variant built from a failing connect with a known secret and assert it is absent). The known-hosts file SHALL be created owner-only (`0600`, directory `0700`) and bounded in size and entries; a corrupt line SHALL be skipped, not trusted. Remote listings and transfers SHALL obey the entry, depth and size caps before allocation.

#### Scenario: Secret never surfaces
- **WHEN** a connect fails with the password `hunter2`
- **THEN** no error string, job label, notice or state file contains it

#### Scenario: Known-hosts permissions
- **WHEN** a host key is first saved
- **THEN** the file mode is `0600` and unreadable to other users

### Requirement: SMB credentials and transport safety
SMB passwords SHALL live in memory only (never in `Prefs`, profiles, workspaces or logs) and SHALL NOT appear in any `FmError` display; a canary-secret test SHALL cover every SMB error path. Authentication SHALL be NTLM username/password or guest; other mechanisms SHALL fail with a message saying they are unsupported. A preference "Require encryption" (`smb_require_encryption`) SHALL, when on, make every new SMB connection demand encryption and refuse servers that cannot provide it; the connect popup SHALL start with that checkbox set from the preference. Plain sessions are always signed or labelled "NOT signed". Remote size and entry caps apply as for SFTP.

#### Scenario: Canary password
- **WHEN** every SMB connect and I/O failure is formatted
- **THEN** none of the strings contains the canary password

#### Scenario: Require encryption
- **WHEN** the preference is on and the server cannot encrypt
- **THEN** the connection is refused with an actionable message

### Requirement: Temporary opened files and vendored crates
A remote file opened with its default application SHALL be downloaded to an owner-only (0700) per-process folder as a non-executable file; remote `run` mode and downloads over 1 GiB SHALL be refused. The app SHALL delete its own folders on quit and folders older than 24 hours at start. Vendored crates (`vendor/sspi`, `vendor/smb-transport`) SHALL be listed with their deviations in `vendor/FM-VENDORED.md`.

#### Scenario: Open a remote file
- **WHEN** the user opens a file on a remote location
- **THEN** a job with progress downloads it to the private folder, then the default app opens it, and the folder is removed when the app quits

### Requirement: Search, sizes and Open With are bounded
Find Files and Properties walks SHALL NOT follow symlinks and SHALL be capped (see `file-search`; sizes stop at 5 M entries or depth 256). Wildcard patterns are capped at 1024 bytes and 64 alternatives and matched without backtracking blow-up. Open With SHALL parse `.desktop` `Exec` lines without a shell, drop field codes other than `%f %F %u %U %%`, cap arguments at 256 and launch detached; chmod masks modes to `0o7777` and refuses to change a symlink.
