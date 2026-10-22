# security

Baseline security requirements for the scaffold. Later changes extend this spec: change 3 adds the drag & drop and `ops` rules (symlink escapes, overwrite policy), change 4 adds credential and host-key handling for SMB/SFTP.

## ADDED Requirements

### Requirement: Hostile input never crashes or escapes
All parsers and entry points that touch attacker-controlled data — `VfsUri` parsing, command parameters, and (later) remote directory listings and drop payloads — SHALL reject malformed input with `Err`, never panic, never allocate unbounded memory, and never treat input as a trusted path.

#### Scenario: Adversarial URIs
- **WHEN** `VfsUri` parsing is fuzzed/pro property-tested with inputs containing `..` segments, NUL bytes, overlong components (≥ 4 KiB), non-UTF-8 bytes, missing schemes, `file://` with a host, repeated `//`, and 10 000-character paths
- **THEN** every case returns `Err(InvalidUri)` or a normalized safe `Ok`, and no case panics or takes more than the bounded time/size budget

#### Scenario: Absolute paths stay absolute and contained
- **WHEN** a parsed `VfsUri::parent()` or `join()` chain is applied to any hostile path
- **THEN** the result is either `None` or a URI that still parses and is still absolute (no `..` in the normalized form)

#### Scenario: Command parameters are untrusted
- **WHEN** the command fuzz suite feeds every registered command wrong-typed, oversized (≥ 1 MiB string), empty and deeply-nested parameters
- **THEN** each call returns `Ok`/`Err`, the process survives, and no single call allocates more than the configured cap

### Requirement: Secrets are never written to disk
The scaffolded preferences file SHALL contain only non-secret settings (theme, window state). Passwords, passphrases, private keys and session tokens SHALL NOT be serializable into it, and error/reporting paths SHALL redact credentials.

#### Scenario: Prefs schema has no secret slots
- **WHEN** the `Prefs` type and its serialized form are inspected by test
- **THEN** no field can hold a password/passphrase/token (the type is closed: theme + window fields only)

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
