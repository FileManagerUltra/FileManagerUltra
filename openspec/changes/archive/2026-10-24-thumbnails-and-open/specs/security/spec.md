## ADDED Requirements

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
