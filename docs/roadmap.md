# Roadmap

Five OpenSpec changes, in order. Each change proposes, implements and archives itself; its deltas sync into `openspec/specs/` so the specs always describe what the product actually does.

| # | Change | Delivers | Status |
|---|---|---|---|
| 1 | `scaffold-workspace` | Cargo workspace, crate skeletons, `xtask layers` + `brand-check`, AGENTS.md/docs/devlog, branding module, empty themed window, **security baseline (fuzzed URIs/params, redacted errors, owner-only state files, no `unsafe`)** | done |
| 2 | `multi-pane-ui` | Recursive split panes, per-pane file tree + virtualized listing, selection, navigation/history, commander shortcuts (F5–F8, Tab), **dark/light/system theme from day one** (tokens only, no hard-coded colours); pane code works against any `VfsUri`, so remote backends plug in without UI rework | done, archived |
| 3 | `dnd-and-file-ops` | Cross-pane drag & drop (tree node + listing targets, Ctrl=copy/Shift=move), external file drop, `ops` pipelines, job queue with progress/cancel, conflict dialogs, trash + undo | done, archived |
| 4 | `sftp-backend` | **SFTP client that fills a pane** (`sftp://user@host/…` → tree + listing, connect dialog, in-memory passwords, TOFU host keys, profiles without secrets, cross-backend ops, free space in the status bar) | archived |
| 4b | `smb-backend` | SMB via the pure-Rust `smb` crate (NTLM, no system library, one code path on all platforms), reusing the connection model from change 4 | done (archived) |
| 5 | `live-watch-and-packaging` | `notify`-driven directory refresh; cross-platform watcher and release-package smoke checks | watcher implemented and Linux-tested; Windows/macOS checks remain |

Additional change, implemented: `git-status-and-diff` (git status markers, Diff view in the Preview pane) — see `docs/architecture.md` §12a; pending archive.

**Current focus:** close out `live-watch` with Windows/macOS smoke checks, including a mapped drive, then validate the existing release artifacts.

Session persistence is already delivered by `workspaces` (saved pane layouts and
startup behavior). Release packaging is also configured in `.github/workflows/release.yml`:
Linux `.deb`/AppImage, Windows NSIS `*-setup.exe`, and macOS `.dmg` for Apple silicon
only. It is not an `.msi` installer. The remaining milestone work is validation,
not implementing persistence or packaging from scratch.

**Security is a standing concern, not a change.** The `security` OpenSpec capability starts in change 1 (hostile input, secrets, state-file permissions, symlink policy, no `unsafe`) and each change that adds a behaviour area extends it in the same change: change 3 adds drag & drop / `ops` containment and overwrite rules; change 4 adds credentials, host keys and remote listing caps. A change that adds security-relevant behaviour without a spec + failing-test proof is not done.

Additional capabilities have landed alongside these milestones: previews, workspace
tabs, file/content search and terminal panes. Archives, plugins and CLI/MCP automation
remain future work (the command registry is already in place).

## Current focus

Recent interaction improvements include rich native Markdown previews (nested lists,
tasks, highlighted code and aligned tables), full-loaded-document selection/copy,
per-preview 50%-400% zoom, configurable terminal scrollback (5,000-1,000,000 rows,
applied to new/restarted terminals), and shared folder menus for listing backgrounds
and tree folders. Preferences clipboard routing and terminal-close notices are fixed.
The approved yellow-outline/blue-panel icon is applied across platform assets.
File timestamps now follow the OS timezone, including historical daylight-saving
rules, while log filters remain UTC. Markdown code panels have translucent copy
buttons that become opaque on hover/focus and copy only code; neither buttons nor
language labels enter text selections or document copies.
File and thumbnail menus now offer **Open in Preview Panel**, targeting the clicked
item and reusing an existing Preview (or creating one if absent), without changing
the Preview toggle shortcut.
Preview Find now provides Ctrl+F, highlighted next/previous matches, and
case/whole-word/regex options over loaded text, structured documents and rendered
Markdown; logs focus their existing record search. Matching is background work,
and unwrapped text remains virtualized.
Preview Find and zoom controls share a single source/title toolbar row, with
view options overflowing into a menu in narrow panes. Logs expose only Filter,
not a duplicate Find button; Ctrl+F still focuses their record search.
Automated coverage is in place; interactive confirmation of the latest preview and
context-menu changes and installed Windows/macOS icon smoke checks remain open.

`devices-section` is implemented with conditional Devices navigation, background
mount detection, volume information and guarded removal. Real USB/VeraCrypt smoke
checks on Linux, macOS and Windows remain open in its task list; the change is not
archived.

`pane-preview`, `pane-arrange`, `thumbnails-and-open`, `current-folder-row` and `workspaces` are implemented, smoke-tested in a real window and archived (their specs live in `openspec/specs/`). Change 4 (`sftp-backend`) is implemented and archived; 4b (`smb-backend`, with Network discovery) is implemented and archived. Change 5's watcher implementation is complete; its remaining platform checks are listed above.

## Ideas for future changes

Captured so they are not lost; each becomes an OpenSpec proposal (`/opsx-propose`) when picked up.

- **Preferences menu + pane-layout restore policy** (extends existing workspace startup behavior): a Preferences menu item holding app settings from `prefs.toml`, with *Restore previous pane layout* as `yes` / `no` / `ask`. On `ask`, a popup poses the question at startup before the saved layout is applied, and the popup repeats the same yes/no/ask control so the setting can be changed right there (the answer persists the preference). Stays a command (`prefs.*` id) and a `Prefs` field, so the security rules (no secrets, owner-only file) hold unchanged.

### File synchronization

A substantial future feature inspired by FreeFileSync, not just a folder-comparison
action. Folder comparison belongs to this feature as the basis for planning a sync.
Candidate scope for its OpenSpec proposal:

- Compare two folder trees by relative path, size and timestamps, with optional
  content comparison.
- Show a reviewable plan of copies, updates, deletions and conflicts before any
  changes; make direction and destructive actions explicit.
- Consider one-way update, mirror and two-way synchronization, with include/exclude
  filters and reusable folder-pair profiles.
- Build on the VFS and existing background file-operation lanes, progress and
  cancellation rather than introducing a separate transfer engine.

The proposal must settle supported backends, comparison rules, two-way change
tracking, deletion/conflict policy, symlink handling and recovery after partial
failure or cancellation. Scheduling and unattended operation are separate scope
decisions, not commitments. No implementation or delivery date is planned yet.

### Batch rename

A future multi-item rename tool with a before/after preview. Candidate scope for
its OpenSpec proposal:

- Numbering, text replacement, case conversion and filename/extension handling.
- Preview every proposed name and flag invalid names, unchanged items and
  collisions before applying.
- Handle swaps and rename cycles safely; report partial failures explicitly and
  evaluate undo through the existing operation model.
- Dispatch through commands and run filesystem work on the existing job lanes.

The proposal must settle rule ordering, regex support, backend-specific naming and
case-sensitivity rules, temporary-name handling, cancellation and undo guarantees.
This is a planned feature idea, not approved implementation scope.

## Working agreement

- Nothing is "done" until `cargo test`, `cargo clippy -D warnings`, `cargo xtask layers`, `cargo xtask brand-check` and `cargo fmt --check` are green, tests are added, and `log/devlog.md` has an entry.
- Scope cuts happen in the proposal, not mid-change: if a task turns out wrong, edit the change's `tasks.md`/`design.md` before continuing.

## PRIORITY 1: remove the `unsafe` in `fm-wayland` (winit 0.31 `start_drag`)
- Decision (2026-11-05): native-Wayland drag-out and drops ship in `fm-wayland` with one documented `unsafe` exception (`crates/wayland/src/foreign.rs`: adopts winit's `wl_display`/`wl_surface`, because only the connection that got the button press can start a drag).
- Goal: delete `fm-wayland` and use winit 0.31's safe `ActiveEventLoop::start_drag` (rust-windowing/winit#4571) once eframe moves to winit 0.31 (egui#7731; community 0.30 patch egui#8805). Unverified by us; check before starting.
- Background: https://notes.nickdiego.dev/chromium/wayland-events-during-drag (GNOME: origin loses pointer events during a drag; drop is signalled via data_source events).
- Status: unit tests only; verify by hand against Chrome/Brave on native Wayland (GNOME/mutter).
