## MODIFIED Requirements

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
