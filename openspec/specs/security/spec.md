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
- **AND** log preview configuration stores only non-secret numeric `log_poll_ms` and `log_tail_mib`; zero tail size does not bypass preview safety caps, and hand-edited numeric values are clamped before scheduling or allocating
- **WHEN** the `Prefs` type and its serialized form are inspected by test
- **THEN** no field can hold a password/passphrase/token (the type is closed: theme + window fields only)
- **AND** saved window geometry contains only bounded numeric size, position and scale-factor fields, using the existing owner-only atomic preferences writer
- **AND** preview filters contain only non-secret filename patterns in 14 closed type groups, bounded to 256 bytes per pattern, 64 patterns and 1024 bytes per group, with no paths, list separators or control characters

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
Our crates SHALL contain no `unsafe` (workspace-wide forbid), and every dependency added later that performs FFI or network I/O MUST be justified in `docs/architecture.md` and isolated in its L0 backend crate. The only exceptions are the user-approved platform drag-and-drop crates `fm-wayland` and `fm-winout`: each sets its own lints (`unsafe_code = "deny"`), allows `unsafe` in exactly one named module, documents the invariants of every `unsafe` call, and keeps all other code in the crate safe. The Windows drag source SHALL offer only the dragged local paths as a file-drop list, never read or modify the files, and SHALL validate the path list (absolute, no NUL, count-capped) before any OS call.

#### Scenario: Unsafe is structurally impossible
- **WHEN** any workspace crate other than `fm-wayland` and `fm-winout` enables `unsafe`
- **THEN** the build fails (`unsafe_code = "forbid"` in workspace lints)

#### Scenario: Dependency additions are auditable
- **WHEN** a change adds a dependency that does network or FFI work
- **THEN** the change updates the dependency table in `docs/architecture.md` (review checklist item; enforced by change review, not tooling)

#### Scenario: Hostile drag-out paths
- **WHEN** a drag-out payload contains an empty list, relative paths, paths with NUL, or more than the cap
- **THEN** the drag is refused with an error before any OS call and the application does not crash

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
File content shown in a Preview SHALL be treated as hostile: bounded reads, header checks before allocation, no network access except opt-in remote Markdown images as defined below, no execution of embedded content, decoder panics contained, and decoder child processes (video) started without a shell and with fixed arguments, the file passed as a single argument after `--`/via `file:` protocol guard.

Markdown images SHALL be limited to 8 MiB of source and 100 megapixels, at most 200 per document. Local images SHALL resolve only inside the previewed file's root. Remote images SHALL load only with the explicit, default-off preference, over `http`/`https` with URLs of at most 2048 characters and no embedded credentials, no cookies, at most 3 redirects, a 10 second timeout and the byte cap enforced on both `Content-Length` and streamed data. Every resolved address, including after each redirect, SHALL be public; loopback, private, link-local, unique-local, CGNAT, multicast, unspecified and IPv4-mapped equivalents SHALL be refused, and the connection SHALL use the validated address. The preference SHALL hold no secret and a malformed value SHALL mean off.

SVG files and SVG images in Markdown SHALL be parsed from at most 16 MiB and drawn within the image pixel limits. Rendering SHALL NOT follow any `<image>` reference (local path, relative path, URL or `data:` payload), load stylesheets, scripts or remote fonts, or run animation; the only files read besides the SVG are the installed system fonts. Font files SHALL be read, not memory-mapped.

#### Scenario: Rich Markdown remains inert and bounded
- **WHEN** Markdown contains remote images/links, HTML/script tags, deeply nested
  lists/quotes or excessive table cells and inline nodes
- **THEN** links are not fetched, markup is not executed, remote images are not fetched unless enabled, document depth and
  allocation counts remain bounded, and exceeded caps produce a truncation notice
- **AND** magnifying a preview cannot increase source-read or decoder limits

#### Scenario: Hostile filename to the decoder
- **WHEN** a video file is named `-i evil.mp4` or contains shell metacharacters
- **THEN** the decoder receives it as one argument and no other file or command is touched

#### Scenario: SVG cannot read other files
- **WHEN** a previewed SVG (or an SVG image in Markdown) contains `<image href="/home/u/secret.svg">` or a `data:` image
- **THEN** nothing is read or drawn for that element, and the rest of the drawing renders

#### Scenario: Remote image targets an internal host
- **WHEN** remote images are enabled and an image URL (or a redirect) resolves to `127.0.0.1`, `192.168.x.x`, `169.254.169.254` or `::1`
- **THEN** no request is sent to that address, the placeholder is shown and an error is recorded without any URL credentials

#### Scenario: Oversized or slow remote image
- **WHEN** a remote image declares or streams more than 8 MiB, or stalls beyond 10 seconds
- **THEN** the transfer is aborted and the placeholder is shown without growing memory past the cap

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
Find Files and Properties walks SHALL NOT follow symlinks and SHALL be capped (see `file-search`; sizes stop at 5 M entries or depth 256). Wildcard patterns SHALL be capped at 1024 bytes and 64 alternatives and matched without backtracking blow-up. Open With SHALL parse `.desktop` `Exec` lines without a shell, drop field codes other than `%f %F %u %U %%`, cap arguments at 256 and launch detached; chmod SHALL mask modes to `0o7777` and refuse to change a symlink.

#### Scenario: Oversized wildcard pattern
- **WHEN** a search pattern exceeds 1024 bytes or 64 alternatives
- **THEN** it is rejected before any walk starts

### Requirement: Mounted-volume removal is identity-checked and non-forced
Eject/Dismount SHALL derive arguments from a currently listed OS mount, revalidate
its identity before acting and refuse malformed, stale or unsupported targets.
Tools SHALL use fixed argument arrays; required PowerShell source SHALL be fixed
with separately validated arguments, never interpolated mount names. Tool output
and runtime SHALL be bounded. Display labels SHALL be capped, stripped of control
characters and redacted; raw operation identities SHALL remain separate.

#### Scenario: A mount point is reused
- **WHEN** another volume replaces the one selected for removal
- **THEN** removal is refused without executing its tool

### Requirement: Encrypted volume discovery requires no secrets
Discovery/removal SHALL NOT unlock containers, request or store credentials,
inspect locked contents or log raw operation identities. Unknown encrypted/virtual
mounts SHALL stay browseable with an explanatory disabled Dismount action.
Dismount SHALL never power off the underlying disk or sibling volumes.

#### Scenario: VeraCrypt container on a USB disk
- **WHEN** no verified volume-manager adapter exists
- **THEN** the app directs the user to VeraCrypt instead of attempting hardware eject

### Requirement: Hostile repositories are inert
Opening a repository for status or diff SHALL never execute hooks, filter drivers, fsmonitor,
credential helpers, textconv or any external program, and SHALL ignore configuration that would
reference them. Repository paths, ref names, file names and blob contents are untrusted:
discovery depth, entry counts, blob sizes, ref lookups and diff input sizes are capped, and
symlinks are never followed out of the working tree.

#### Scenario: Malicious config
- **WHEN** a repo's config sets `core.fsmonitor` or a filter driver to a command
- **THEN** status and diff complete without running it

#### Scenario: Filter driver
- **WHEN** a repo's config defines `filter.<name>.clean`/`process` and `.gitattributes` applies it
- **THEN** repository-local `filter.*` sections are removed from the in-memory config and nothing runs

#### Scenario: Oversized or corrupt data
- **WHEN** a repo has a corrupt object, a 10 GiB blob or 1 000 000 untracked files
- **THEN** the feature degrades with a visible notice, bounded memory and no panic

### Requirement: Markdown HTML is inert and bounded
HTML in Markdown SHALL never execute, load styles, scripts, frames or any resource other than images under the Markdown image rules. HTML blocks SHALL be limited to 256 KiB each and nesting depth to 32; attribute values SHALL be length-bounded; exceeded limits SHALL truncate with the existing notice rather than fail.

#### Scenario: Hostile HTML
- **WHEN** Markdown contains deeply nested tags, huge attributes, unterminated tags or entity bombs
- **THEN** the preview stays responsive, memory stays within the caps and nothing is fetched except permitted images

### Requirement: Release checks do not execute remote code
Update checks SHALL only fetch bounded release metadata over verified HTTPS.
Downloads SHALL be initiated by the user in a browser at the official repository.
No release-provided URL, command, executable or markup SHALL be executed.

#### Scenario: Release metadata contains an unrelated download URL
- **WHEN** metadata supplies an external or non-HTTPS URL
- **THEN** the application ignores it and uses the locally constructed official release page

### Requirement: Removal runs fixed tools with listed arguments only
Eject and Dismount SHALL execute fixed per-OS adapters with argument arrays and SHALL derive the device identity from the scanned list, not from command parameters. No untrusted text may be interpolated into shell or PowerShell source; any required script SHALL be fixed with separately validated arguments. Device labels and mount paths are untrusted: display copies are length-capped and stripped of control characters, while operation identities are preserved separately and never interpreted as commands or markup. Removal SHALL verify the current mount identity before acting so a reused mount point cannot target a replacement volume.

#### Scenario: Hostile volume label
- **WHEN** a device label contains control characters, a newline or shell metacharacters
- **THEN** it is shown sanitised and has no effect on any executed command

