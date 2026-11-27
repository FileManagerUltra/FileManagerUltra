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

**Latest release:** 0.3.2 (2026-11-23): CSV/TSV table view and drag-to-pan pictures in the Preview pane, Ctrl+wheel and pinch zoom per pane, and fixes to Synchronize Folders, terminal start folders and sharp photos at 100%. 0.3.1 (2026-11-20) added PDF fill-in and signing. 0.3.0 (2026-11-19) brought Synchronize Folders and the first signed and notarized macOS download. 0.2.3 (2026-11-15) brought per-pane and per-terminal zoom, Ctrl+Tab workspace switching, a diagnostic log and Direct3D 12 drawing on Windows. 0.2.0 (2026-11-12) brought the Media pane, Find Files redesign and Activity panel. Archives (`add-archive-vfs`) are unreleased: see `[Unreleased]` in `CHANGELOG.md`. The Mac App Store build (sandboxed) is planned as its own change. Free Windows code signing through SignPath Foundation is prepared but not yet applied for (`docs/development.md`, "Windows code signing"). Release artifacts for all three platforms are built and published by `.github/workflows/release.yml`.

**Current focus:** finish `add-archive-vfs` (see "Next: archives, disk images and Docker as folders"), then release it; `add-disk-images` and `add-docker` follow. `add-text-editor` (Edit mode of the Preview pane: large files, tree-sitter colouring, autosave, per-kind internal/external choice; `docs/architecture.md` §12d2) and its follow-up `add-editor-folding-live-preview` (code folding, rainbow brackets, a live preview pane) are done and archived (2026-11-27), to ship in 0.4.0 with archives once the archive smoke test (add-archive-vfs 8.4) passes; scroll sync between editor and live preview is left for later. Proposed for after 0.4.0: `add-rtf-preview` (RTF read into the Markdown document view, with code pages, tables and embedded metafile pictures; `openspec/changes/add-rtf-preview/`).

Session persistence is already delivered by `workspaces` (saved pane layouts and
startup behavior). Release packaging is also configured in `.github/workflows/release.yml`:
Linux `.deb`/AppImage, Windows NSIS `*-setup.exe`, and macOS `.dmg` for Apple silicon
only. It is not an `.msi` installer. The remaining milestone work is validation,
not implementing persistence or packaging from scratch.

**Security is a standing concern, not a change.** The `security` OpenSpec capability starts in change 1 (hostile input, secrets, state-file permissions, symlink policy, no `unsafe`) and each change that adds a behaviour area extends it in the same change: change 3 adds drag & drop / `ops` containment and overwrite rules; change 4 adds credentials, host keys and remote listing caps. A change that adds security-relevant behaviour without a spec + failing-test proof is not done.

Additional capabilities have landed alongside these milestones: previews, workspace
tabs, file/content search and terminal panes. Plugins and CLI/MCP automation
remain future work (the command registry is already in place).

## Recently shipped

0.3.x: `add-file-sync` and `add-path-picker` (Synchronize Folders, Browse… on every path
field; archived 2026-11-24), `add-pdf-preview` (PDF view, form-fill and signing; 0.3.1),
`add-table-preview` (CSV/TSV table view; 0.3.2) and `add-quality-gate`. What is in
progress is under "Next: archives, disk images and Docker as folders" below; everything
older follows.

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

### Next: archives, disk images and Docker as folders

Three OpenSpec changes, in order (proposals written 2026-11-23). `add-archive-vfs` is
nearly done: browsing (zip, tar family, 7z, nested, remote, encrypted, ISO, RAR through an
installed tool), split archives (open, and save back as parts of the original size with a
rollback-able swap and recovery after an interrupted save), tree expansion, restoring open
archives after a restart, editing with Save / Save as copy / Discard, password
set/change/remove, Extract and Compress (also in parts) and adding a selection to an archive (menu or drop), Join Parts, move-out rules and the archive preview card have landed. Still open (see its
`tasks.md`): the tab marker for unsaved archives, the RAR and nesting-limit pane states and the manual smoke on Linux and
Windows. The other two are not started:

- **`add-archive-vfs`**: zip, tar (gz/bz2/xz/zst), 7z, ISO and RAR (read-only, through an
  installed 7-Zip or unrar) open like folders with Enter, on every backend and nested,
  in a distinct archive colour. Split archives (numbered parts, zip spanned sets, RAR
  volumes) open as one archive from any part. Encrypted zip/7z/RAR open with a
  password; Compress to… and Set password… encrypt (AES-256). Edits inside writable
  archives are staged and written by an explicit, atomic Save (Discard and quit
  prompts like PDF Fill & Sign). Extract here / to, Compress to (optionally in parts),
  Join parts.
- **`add-disk-images`**: read-only SquashFS, ext2/3/4, FAT, NTFS, JFFS2 and CramFS images,
  MBR/GPT partitions as folders, and "Open as archive or image…" that scans any file for
  signatures at any offset (binwalk-style).
- **`add-docker`**: local Docker/Podman engine in Places; containers (copy in/out, edit,
  new folder), images (read-only) and volumes as folders through the Engine API, plus
  `docker save`/OCI image tars as archives.

### Near-future: more viewers in the Preview pane

Planned next; each becomes its own OpenSpec proposal. All render off the UI thread, are
bounded for hostile input like the existing previews, and follow the Preview pane's
zoom, Find and theme conventions. Already shipped: the PDF view with form-fill and
signing (0.3.1, `add-pdf-preview`; `docs/architecture.md` §12c) and the CSV / TSV table
view (0.3.2, `add-table-preview`).

- **draw.io diagram view**: render `.drawio` / `.dio` files (including compressed
  diagrams and multiple pages) as an SVG-style preview through the shared SVG service,
  with Copy as image like Mermaid.
- **Dia diagram view**: render `.dia` files in the Preview pane. Open question for the
  proposal: view only, or basic editing (the request called it an "editor view").
- **Ebook view**: read EPUB (zipped XHTML), MOBI/AZW (PalmDoc and HUFF/CDIC compressed
  text) and CHM (compiled HTML Help, LZX) books in the Preview pane: the table of contents
  as a sidebar, chapters rendered through the same document view as Markdown and RTF
  (headings, lists, tables, links between chapters, pictures through the bounded image
  pool), Find across the book and the last read position remembered per file. Books also
  browse as folders where the format is an archive (EPUB, CHM), like `add-archive-vfs`.
  Scripts, external resources and DRM-protected books are never run or fetched; a
  protected book says so. Open questions for the proposal: pure-Rust LZX and HUFF/CDIC
  decoders (or a vetted crate), and whether KF8/AZW3 is in the first change.

### Near-future: quick feature walkthrough
- **Guided tour**: a short walkthrough (Help menu and first run) that shows off several
  features with balloons pointing at the real UI, e.g. panes, tree, preview, Find, sync.
  It runs on a built-in sandbox of example files (in-memory/temp, never the user's
  folders), so nothing it demonstrates can change, copy or delete real data. Skippable,
  restartable, every step dispatches normal commands against the sample tree.

### Near-future: select with a rectangle

- **Rubber-band selection**: drag a rectangle over empty space in a Files pane to
  select the files/items it touches, in the list, grid and thumbnail views. Ctrl adds
  to the selection, Shift/Ctrl-drag toggles; the rectangle scrolls the pane near its
  edges. Open questions for the proposal: virtualized rows (hit-test by row index, not
  by drawn widgets), interaction with drag & drop starting on an item, and a command
  id so it stays dispatchable (`selection.*`). This is a planned feature idea, not
  approved implementation scope.

### Near-future: better Git handling

`git-status-and-diff` gives read-only status marks and a Diff view. A follow-up change
would make version control a first-class workflow:

- **Changed-files overview**: one view per repository listing modified, added, deleted,
  renamed and untracked files (staged and unstaged), with the diff of the selected file
  beside it and jumps to the file in a Files pane.
- **Branch switching**: show the current branch in the pane header or status bar, list
  local and remote branches, and switch (refusing or asking when the working tree has
  changes that would be overwritten).
- The text editor is now its own change (`add-text-editor`, Edit mode of the Preview
  pane); Git integration can build on it later (e.g. stage from the editor).
- Open questions for the proposal: `fm-git` is read-only `gix` today, so writing
  (checkout, stage, commit?) needs a scope decision and its own safety rules; how far
  to go beyond switching (stage/commit/stash).

### File synchronization

Implemented as OpenSpec change `add-file-sync`, archived 2026-11-24 together with
`add-path-picker` (`fm-sync`, `sync.*` commands, File >
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
