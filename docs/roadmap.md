# Roadmap

Five OpenSpec changes, in order. Each change proposes, implements and archives itself; its deltas sync into `openspec/specs/` so the specs always describe what the product actually does.

| # | Change | Delivers | Status |
|---|---|---|---|
| 1 | `scaffold-workspace` | Cargo workspace, crate skeletons, `xtask layers` + `brand-check`, AGENTS.md/docs/devlog, branding module, empty themed window, **security baseline (fuzzed URIs/params, redacted errors, owner-only state files, no `unsafe`)** | done |
| 2 | `multi-pane-ui` | Recursive split panes, per-pane file tree + virtualized listing, selection, navigation/history, commander shortcuts (F5–F8, Tab), **dark/light/system theme from day one** (tokens only, no hard-coded colours); pane code works against any `VfsUri`, so remote backends plug in without UI rework | done, archived |
| 3 | `dnd-and-file-ops` | Cross-pane drag & drop (tree node + listing targets, Ctrl=copy/Shift=move), external file drop, `ops` pipelines, job queue with progress/cancel, conflict dialogs, trash + undo | done, archived |
| 4 | `sftp-backend` | **SFTP client that fills a pane** (`sftp://user@host/…` → tree + listing, connect dialog, in-memory passwords, TOFU host keys, profiles without secrets, cross-backend ops, free space in the status bar) | archived |
| 4b | `smb-backend` | SMB via the pure-Rust `smb` crate (NTLM, no system library, one code path on all platforms), reusing the connection model from change 4 | done (archived) |
| 5 | `live-watch-and-packaging` | `notify`-driven directory refresh; cross-platform watcher and release-package smoke checks | watcher done and archived as `live-watch` (Linux, Windows, macOS checked); packaging in `.github/workflows/release.yml` |

Additional change, done and archived: `git-status-and-diff` (git status markers, Diff view in the Preview pane) — see `docs/architecture.md` §12a.

**Latest release:** 0.3.0 (2026-11-19): Synchronize Folders (guided setup and Advanced view, Update/Mirror/Two-way, Keep in sync), a Browse… chooser for every path field, and the first signed and notarized macOS download. 0.2.3 (2026-11-15) brought per-pane and per-terminal zoom, Ctrl+Tab workspace switching, a diagnostic log and Direct3D 12 drawing on Windows. 0.2.0 (2026-11-12) brought the Media pane, Find Files redesign and Activity panel. The Mac App Store build (sandboxed) is planned as its own change. Free Windows code signing through SignPath Foundation is prepared but not yet applied for (`docs/development.md`, "Windows code signing"). Release artifacts for all three platforms are built and published by `.github/workflows/release.yml`.

**Current focus:** see "Current focus" below: the `windows-wgpu` redraw fix (task 3.4), then release 0.2.4.

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

`add-media-pane` shipped in 0.2.0 and is archived (2026-11-15): shared native audio pipeline
with Preview sound, the Media pane (persisted queue with lockable playlist folders,
playlists, drop targets on player and playlist, Files/Preview entry points with
position hand-off, double-click to play, player right-click menu), local and remote
(spooled) video with speed control, audio/subtitle track selection, styled text
subtitles, DVD folders and `.iso` images, fullscreen, YouTube-style keyboard
controls, status-bar transport, gapless playback, equalizer, A-B loop, sleep timer,
audiobooks (chapters, resume, bookmarks, skip), and an ffmpeg fallback for audio
formats Symphonia cannot read. ffmpeg is not bundled: Windows offers a pinned,
verified download (`managed-ffmpeg`). `media-open-and-play-ux` is archived.

Picture (PGS/VobSub) subtitles and hardware decoding with a software fallback have
landed too. `live-watch` (Windows/macOS checks done), `status-bar-time` and
`image-orientation` are archived as well.

All open changes except `windows-wgpu` passed their manual checks and were archived
on 2026-11-16 (`native-paths`, `drive-info`, `smb-search`, `raw-photos`,
`remote-bookmark-panes`, `terminal-start-folder`, `terminal-zoom`, `diagnostics-log`,
`managed-ffmpeg`, `app-folder-names`). `windows-wgpu` followed the same day, after the
playback redraw fix (playback no longer counts as busy; it wakes the UI four times a
second) passed on Windows. The audio decoder runs on Symphonia 0.6.

`markdown-mermaid` is archived (2026-11-16, checked on Linux and Windows): Mermaid code
blocks in Markdown previews are drawn as diagrams (eleven types, ELK layout, app theme
colours, offline and bounded) with Show source, Copy source, Copy as image and a cache
reset in Preferences ▸ Advanced. Help ▸ About lists third-party licences (ELK is EPL-2.0).
A follow-up could preview standalone `.mmd` / `.mermaid` files with the same service.

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
mount detection, volume information and guarded removal. It was smoke-tested with
USB media and VeraCrypt on Linux and Windows (macOS untested) and is archived.

`pane-preview`, `pane-arrange`, `thumbnails-and-open`, `current-folder-row` and `workspaces` are implemented, smoke-tested in a real window and archived (their specs live in `openspec/specs/`). Change 4 (`sftp-backend`) is implemented and archived; 4b (`smb-backend`, with Network discovery) is implemented and archived. Change 5's watcher implementation is complete; its remaining platform checks are listed above.

## Ideas for future changes

Captured so they are not lost; each becomes an OpenSpec proposal (`/opsx-propose`) when picked up.

- **Preferences menu + pane-layout restore policy** (extends existing workspace startup behavior): a Preferences menu item holding app settings from `prefs.toml`, with *Restore previous pane layout* as `yes` / `no` / `ask`. On `ask`, a popup poses the question at startup before the saved layout is applied, and the popup repeats the same yes/no/ask control so the setting can be changed right there (the answer persists the preference). Stays a command (`prefs.*` id) and a `Prefs` field, so the security rules (no secrets, owner-only file) hold unchanged.

### Near-future: more viewers in the Preview pane

Planned next; each becomes its own OpenSpec proposal. All render off the UI thread, are
bounded for hostile input like the existing previews, and follow the Preview pane's
zoom, Find and theme conventions.

- **PDF view with form-fill and signing**: shipped in 0.3.1 (OpenSpec change
  `add-pdf-preview`, archived 2026-11-23; specs `pdf-view`, `pdf-forms`, `pdf-signing`):
  hayro renderer, lopdf editing, visual and PAdES digital signatures, signature
  verification; see `docs/architecture.md` §12c.
- **CSV / TSV table view**: delimiter detection, header row, virtualized rows and
  columns for large files, column sorting and Find. Read-only first.
- **draw.io diagram view**: render `.drawio` / `.dio` files (including compressed
  diagrams and multiple pages) as an SVG-style preview through the shared SVG service,
  with Copy as image like Mermaid.
- **Dia diagram view**: render `.dia` files in the Preview pane. Open question for the
  proposal: view only, or basic editing (the request called it an "editor view").

### Near-future: quick feature walkthrough
- **Guided tour**: a short walkthrough (Help menu and first run) that shows off several
  features with balloons pointing at the real UI, e.g. panes, tree, preview, Find, sync.
  It runs on a built-in sandbox of example files (in-memory/temp, never the user's
  folders), so nothing it demonstrates can change, copy or delete real data. Skippable,
  restartable, every step dispatches normal commands against the sample tree.

### File synchronization

Implemented as OpenSpec change `add-file-sync` (`fm-sync`, `sync.*` commands, File >
Synchronize Folders): Update, Mirror and Two-way modes, reviewable plan, filters,
saved profiles, "Keep in sync" watching, attribute carry-over. See
`docs/architecture.md` §12b. Open follow-ups: grid sorting by size/time, more
conflict rules, a shortcut, manual checks on SFTP/SMB/Windows.

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

## PRIORITY 1: remove the `unsafe` in `fm-wayland` and `fm-winout` (winit 0.31 `start_drag`)
- Decision (2026-11-05): native-Wayland drag-out and drops ship in `fm-wayland` with one documented `unsafe` exception (`crates/wayland/src/foreign.rs`: adopts winit's `wl_display`/`wl_surface`, because only the connection that got the button press can start a drag).
- Decision (2026-11-07): Windows drag-out ships in `fm-winout` (OLE `DoDragDrop`; `unsafe` confined to `crates/winout/src/ole.rs`), same removal plan.
- Goal: delete `fm-wayland` and `fm-winout` and use winit 0.31's safe `ActiveEventLoop::start_drag` (rust-windowing/winit#4571) once eframe moves to winit 0.31 (egui#7731; community 0.30 patch egui#8805). Unverified by us; check before starting.
- Background: https://notes.nickdiego.dev/chromium/wayland-events-during-drag (GNOME: origin loses pointer events during a drag; drop is signalled via data_source events).
- Status: unit tests only; verify by hand against Chrome/Brave on native Wayland (GNOME/mutter).
