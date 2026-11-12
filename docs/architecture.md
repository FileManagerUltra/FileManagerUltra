# Architecture

**FileManagerUltra** is a cross-platform (Linux, macOS, Windows), multi-pane file manager written in **Rust only**: native `eframe`/egui UI, no webview, no Electron, no Tauri. Panes split arbitrarily; every pane owns a file tree and a file listing; files drag between panes (and onto tree nodes). Local volumes, mounted network shares, in-app SMB and SSH/SFTP all sit behind one virtual filesystem trait.

This document is the source of truth for the crate map, layering, data model, concurrency model and the drag-and-drop contract. Design decisions that change any of those get a section here in the same change.

---

## 1. Goals and non-goals

**Goals (v1)**

- Unlimited split panes (horizontal/vertical, recursive), each with a file tree + listing.
- **Any pane can browse a remote.** Local drives, mounted shares, in-app SMB and an **SFTP client**: point a pane at `sftp://user@host/...` (or `smb://server/share`) and its file tree and listing fill from that backend, exactly as for local files. One `Vfs` trait underneath — the pane never knows the difference.
- Drag & drop between panes: drop targets are *tree nodes*, *tree background*, *listing rows* and *listing background*; modifier semantics Ctrl = copy, Shift = move.
- File operations (copy/move/rename/mkdir/delete) as background jobs with progress and cancel.
- Delete goes to the platform trash, with undo.
- Live directory updates via filesystem notifications.
- Commander-style shortcuts (F5 copy, F6 move, F7 mkdir, F8 delete, Shift+F7 new file, Ctrl+, preferences, Ctrl+L edit path, Tab = swap panes; symlinks are recreated, never followed, with optional re-targeting).

**Non-goals (v1)**

- Archive browse/extract, dual-pane-only mode, plugins. (Image preview, workspace tabs and Find Files with content search have since landed.)
- CLI, JSON control channel and MCP — *the command registry exists from day one* (the UI dispatches through it), but the automation crates are a later change. Nothing else may bypass the registry.

---

## 2. Golden rules

These apply to every crate. They are checked in review and by CI (`cargo xtask layers`, `cargo clippy`).

1. **Never crash.** Non-test code contains no `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `unreachable!` — errors propagate as `FmError` with `?`. A malformed filename, a vanished file mid-copy, an unreachable SMB server, a full disk or a 0-byte write must produce a user-visible error, never a panic. Unimplemented backends return `FmError::Unsupported`.
2. **No `unsafe` in our code.** `unsafe_code = "forbid"` workspace-wide. No FFI: the SMB backend is the pure-Rust `smb` crate. Exceptions: `fm-wayland` (two `unsafe` calls adopting winit's Wayland display; roadmap priority 1 to remove) and `fm-winout` (the Windows OLE drag source, `unsafe` confined to `ole.rs`).
3. **Everything user-visible is a command.** New behaviour = a `CommandSpec` in `crates/engine/src/*_cmds.rs`, registered in `commands.rs`, with id, label, menu path, shortcut, params doc, `enabled` and `run`, plus tests. The UI never performs file operations directly.
4. **The UI is thin.** `crates/ui` holds widgets and view state only. It reads `Session` state and dispatches commands; no `std::fs` calls, no job spawning.
5. **The UI thread never blocks on I/O.** Every filesystem call runs on a worker (see §7). A 40 GiB copy across a slow SMB link must not drop a frame.
6. **Tests are the gate.** Every change ships tests. Backends get tempdir integration tests; `engine` gets adversarial-input tests over every command; `ui` gets `egui_kittest` snapshots for pane/tree/listing states.
7. **Input is hostile.** Filenames, remote directory listings, drop payloads and command params are attacker-controlled: bounds-check, cap allocation sizes, never index without `get()`.

---

## 3. Workspace and layers

```text
crates/
  brand        L0  the single place where the product name/ids live (rename story, §5)
  vfs          L0  VfsUri, Entry, Vfs trait, VfsError, VfsRegistry (pure data, no I/O impl)
  fs-local     L0  local + mounted-volume backend (std::fs, notify hooks)
  fs-smb       L0  in-app SMB backend (pure-Rust `smb` crate)
  discovery    L0  LAN discovery (mDNS, WS-Discovery) + the virtual `network:///` folder
  fs-sftp      L0  SSH/SFTP backend (russh + russh-sftp, owns its tokio runtime)
  git          L0  read-only git status, blobs and the line/word diff model (gix + similar, no git binary)
  ops          L1  copy/move/delete/mkdir/rename pipelines: streaming, conflicts, progress
  media        L1  shared audio decode, bounded PCM pipeline, clock and native output
  layout       L2  pane tree (recursive splits), per-pane history/bookmarks/selection
  engine       L3  Session, command registry, job scheduler, watchers, dir cache
  ui           L4  egui widgets: file tree, virtualized listing, pane chrome, dialogs
  testkit      L0  in-memory Vfs, temp-tree builders, command fuzzing helpers
apps/
  fm-app       L5  eframe shell: window, menus, shortcut routing, theming
xtask/                layers | brand-check | ci | bench
docs/  AGENTS.md  openspec/  log/devlog.md
```

**Layering is enforced by `cargo xtask layers`.** A crate may depend only on equal-or-lower layers, and only through the workspace path dependencies declared in the root `Cargo.toml`. New crates must be registered in `xtask/src/layers.rs`. Nothing above L3 may contain filesystem logic; nothing below L4 may name an egui type.

Dependency direction in one line: **`vfs` knows nothing, backends know `vfs`, `ops` knows backends only through `Vfs`, `engine` orchestrates, `ui` observes `engine`, `fm-app` wires them.**

### Shared audio foundation (`add-media-pane`, in progress)

`fm-media` (L1) decodes local audio with Symphonia on a dedicated worker and
outputs through cpal's safe API. Native FFI inside cpal and ALSA development
headers on Linux are an approved dependency exception; workspace code still
forbids unsafe code. File probing and device initialization never run in a frame.
The worker maps channels and resamples once with Rubato, then writes interleaved
f32 PCM to a lock-free, bounded single-producer/single-consumer ring. Device
callbacks neither allocate nor take locks: they apply preamp/volume/mute and a
soft limiter, emit silence on underrun, and update a sample-driven clock.

Preview audio metadata and bounded cover art are read on its existing job lane.
Its `AudioState` owns a non-blocking `AudioPlayer` handle: selection changes,
pane closure and workspace changes cancel playback. Audio is quiet by default;
`preview.audio.*` commands and `prefs.set`'s `preview_audio_autoplay` setting drive
the controls. Starting Preview audio or video pauses the other Preview players.
Video autoplay remains on by default and can be disabled in Preferences.
The Media and Preview panes share their transport widgets (icon buttons, seek bar,
volume) in `fm-ui`'s `player_controls`; Preview shows them as a framed card above
the cover art. The Media equalizer is a persistent window (not a menu) so choosing
a preset never dismisses it.

The Preview video coordinator owns both ffmpeg children off-thread: fixed-cadence
RGBA frames and native-rate s16le PCM (converted to f32 before the same Rubato
pipeline). Only two blocks per pipe, one pending block and one displayed frame
are retained. It holds future frames and replaces late ones against the sink's
sample clock, independently of UI polling; silent videos use a monotonic clock.
Delayed soundtracks retain their leading silence, including after seeks. A
missing output device produces a silent-video hint; failed or stalled decoders
produce errors (10 s no-progress budget). Pause/seek/selection cancellation and
child reaping run on the coordinator, never a rendering frame. Normal app exit
cancels and awaits all current/replaced video workers with a bounded shutdown
wait.

Media audio extras live in `fm-media` (`eq.rs`, gapless hand-over in `player.rs`)
and `engine/src/media_audio_cmds.rs`. The equalizer and fade run in the output
callback through the shared `Gain` (so video soundtracks get them too); the player
worker pre-opens the armed next item and continues on the same sink, reporting the
change with `PlaybackStatus.token`, which the session turns into a queue advance.
A-B loop and sleep timer are tick-driven session state.

The session-owned `MediaSession` now plays local audio independently of Files
selection, with a bounded `fm-media::Queue`, transport/queue commands, metadata,
background cover-art decoding and a virtualized queue view. `AudioOwner` pauses
Media when Preview audio/video starts and vice versa; a late completion from a
paused owner cannot restart it. Finished items auto-advance through shuffle/repeat;
failed items bypass repeat-one and stop after one wholly failed queue cycle.
Closing the pane retains the queue and, by default, audio playback, with a
status-bar Stop control. `media_background_audio` disables that behavior.
The local audio queue, current index, volume, shuffle and repeat are restored
paused from bounded owner-only state. File validation and atomic saves use the
local job lane; startup edits take precedence over a delayed restore, and missing
items are skipped with a notice. Unsupported remote entries are not persisted.
`media_spool` downloads remote (SFTP/SMB) items through their job lane into the
owner-only temp folder (at most 3 kept, size cap, progress, cancel, cleanup) before
playback; remote items are never persisted. `media_track_cmds` selects audio and
subtitle tracks; `media_fullscreen_cmds` owns pane fullscreen (layout untouched) and
the independent `window.fullscreen` flag, which `fm-app` forwards to the viewport
without overwriting saved geometry. `fm-ui::media_keys` maps bare keys to commands.
`media_playlist_cmds` loads bounded local M3U/M3U8/PLS on the local lane, reporting
skipped URLs, missing items and unsupported media without exposing credentials.
Imports append without starting playback; export snapshots the queue to a new
owner-only M3U8 through an atomic no-clobber temporary file.
`media_source_cmds` expands local folder children in natural name order without
recursion. Media drops enqueue in payload order, never copy/move files, and start
the first new item when idle. Delayed imports refuse to replace concurrent queue
edits; command and input caps also bound folder scans and queue growth.
`media_entry_cmds` resolves Files selections or explicit Preview panes for local
audio/video actions. Add appends; Play next inserts after current with priority over
shuffle/history/repeat-one; Play inserts there and starts immediately. Queue
edits remap playback/history/pending and UI selection indices, retaining all
existing items until explicit removal. Preview Play transfers its position and
stops its decoder; Add/Play next leave playback unchanged. Folder context actions
retain their explicit target; Preview Play folder uses the media file's parent.
Media video reuses `VideoState` and the shared ffmpeg player/clock. Cancellable
ffprobe jobs on the local lane load metadata without decoding a poster; dropping
a pending request cancels its probe and discards stale results. A shared texture
helper retains just the latest displayed frame per view. Fit commands bucket
decode sizes; paused frame steps decode one still without opening audio output.
The existing transport, gain, ownership and bounded error/auto-advance paths
serve both media types. Closing/switching the pane or changing to a workspace
without Media cancels video regardless of the background-audio preference.
`media.speed` adjusts video playback from 0.5× to 3× in 0.05× steps. The shared
player applies ffmpeg's pitch-preserving `atempo` chain to the soundtrack and
scales frame deadlines and reported position against its output clock; silent
video uses the same speed-scaled monotonic clock. Audio-only speed, track
selection, fullscreen, YouTube-style shortcuts and advanced playback remain in
the change's task list.

---

## 4. Core data model

### 4.1 `VfsUri`

One path type for every backend, so a drop payload never cares where a file lives.

```rust
pub enum Scheme { File, Smb, Sftp }

pub struct VfsUri {           // serializable, hashable, ordered
    scheme: Scheme,
    authority: Authority,      // host[:port], share, user — empty for File
    path: UriPath,             // always absolute, `/`-normalized, no `.`/`..`
}
```

- `VfsUri::join`, `.parent()`, `.file_name()`, `.is_ancestor_of()` — all pure and total (return `Option`/`Result`, never panic).
- Display form doubles as the address bar: `file:///home/x`, `smb://server/share/dir`, `sftp://user@host:22/abs/path`.
- Windows drive letters normalize to `file:///C:/...` internally and display as `C:\` in the UI; UNC paths are `file:///server/share/...`. `VfsUri::local_path()` (`fm_vfs::native_path`) is the only uri → OS-path conversion. On Windows `file:///` is **This PC**: `fs-local` lists the existing drive roots there (A:–Z:), it has no OS path, and Up from a drive root returns to it.

### 4.2 `Entry`

```rust
pub struct Entry {
    pub uri: VfsUri,
    pub name: String,
    pub kind: EntryKind,        // File | Dir | Symlink { target } | Other
    pub size: Option<u64>,      // None when the backend cannot answer cheaply
    pub modified: Option<SystemTime>,
    pub permissions: Option<Permissions>,
}
```

Names come from the remote/local filesystem verbatim (invalid UTF-8 becomes `OsString`-lossy for display with the raw bytes preserved for operations — see `Entry::raw_name`).

### 4.3 The `Vfs` trait

Blocking by contract: implementations are called **only** from engine workers.

```rust
pub trait Vfs: Send + Sync {
    fn scheme(&self) -> Scheme;
    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError>;
    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError>;
    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError>;
    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError>;
    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError>;
    fn remove(&self, uri: &VfsUri) -> Result<(), FmError>;
    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError>;   // same-backend move
    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool;                  // drives move-vs-copy default
    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError>;
}
```

- **Copy across backends** lives in `ops`: stream `open_read` → `open_write` in chunks with progress ticks. Same-scheme moves call `rename` (fast path); cross-scheme moves stream then delete.

  Move eligibility compares the source device with the existing destination
  parent, since the destination name may not exist yet. Eligible directory
  moves use a shallow plan and rename without scanning descendants. If rename
  fails, the bounded recursive copy plan is expanded before copying that tree;
  progress totals grow accordingly. A renamed tree counts as one item and keeps
  its symlink targets unchanged. Existing destinations retain conflict/merge
  handling, and cross-device moves retain copy/verify/delete behavior.
- `VfsRegistry` maps `Scheme -> Arc<dyn Vfs>`; a connect (SMB/SFTP) creates a session-backed instance and registers it.
- Backends are dumb: no caching, no sorting, no watching. That is `engine`'s job.

### 4.4 Backend choices

| Backend | Implementation | Notes |
|---|---|---|
| `fs-local` | `std::fs` + `notify` | Linux `/`, `/media/$USER`, `/mnt`; macOS `/`, `/Volumes`; Windows `A:`–`Z:` + UNC |
| `discovery` | `mdns-sd` (pure Rust, no async runtime); WS-Discovery is a hand-written UDP probe | `NetworkFs` lists hosts as `<name> (File Sharing)` → `smb://addr/` and `(Remote Login)` → `sftp://addr/`; discovery starts on the first listing and stops when the folder is dropped. An unconnected `smb://host/` raises the sign-in popup (`network.login`); logins live in memory only |
| `fs-smb` | `smb` 0.12 (pure Rust, blocking mode, NTLM; signed always, encryption optional or required) | one `SmbFs` per host+user serves every share (tree-connect on demand); `smb://host/` lists the shares connected so far |
| `fs-sftp` | `russh` + `russh-sftp` | one tokio runtime inside the crate; `Handle::block_on` satisfies the blocking `Vfs` contract |

SMB jobs get their own scheduler lane (§7) — a slow share can never starve local work.

### 4.5 Remote backends in a pane: the SFTP client

**Requirement:** an SFTP location is a first-class pane. Connecting fills the focused pane's file tree and listing from the remote host, and every pane feature (split, drag & drop, copy/move, shortcuts, selection, history) works across the local↔remote boundary without special cases.

**Connect flow** (all steps are commands; nothing blocks the UI thread):

1. An `Sftp` pane without a connection shows the **Saved servers** folder (`servers:///`, `ServersFs` over the SFTP profiles). `remote.connect.dialog` (File ▸ Connect to Server…, the pane's Connect… button) or opening a saved server / typed `sftp://` address raises the connect popup (host, port, user, *password* | *key file*, start folder, save-as, remember); `servers.edit` prefills it from a profile. `PaneKind::Sftp` is a Files pane on a server (`PaneKind::is_files`).
2. `engine` opens the connection on the **`sftp` lane** (`russh` handshake + `sftp` subsystem, host-key check), emitting `JobEvent`s; the dialog shows a spinner and stays responsive.
3. On success the backend is registered in `VfsRegistry` under its authority (`user@host:port`) and the focused pane's `location` becomes `sftp://user@host/<start-dir>`; tree root and listing fill through the normal `ReadDir` → `DirCache` path. On failure: `FmError::Backend` → dialog shows the reason + Retry; the pane is untouched.

Silent saved-server attempts using remembered passwords retain a non-secret
retry prompt. If unreachable or refused, they open that prompt with the error
instead of reporting only in the status bar. Network failures suggest checking
the address, VPN/network and firewall; cancelling an explicit popup does not
reopen it when its in-flight attempt fails. Form errors also stay in the popup.

**Pane behaviour while remote — identical to local, with three differences:**

| Aspect | Behaviour |
|---|---|
| Reads | `read_dir`/`stat` go to the `sftp` lane; results land in the same `DirCache`, so panes/tabs sharing a directory share the cache. |
| Watching | No `notify` for SFTP — a pane refreshes on focus regain, on a manual `F5`, and on a 5 s poll while visible (§7). |
| Connection state | Per-authority state machine `Disconnected → Connecting → Ready → Failed(err)`, shown as a badge in the pane chrome. On disconnect/error the last listing stays visible with a *stale* badge and all mutating actions are disabled for that pane (retry via `remote.reconnect`), never a blank window and never a panic. |

**Credentials (v1):** passwords and passphrases live **in memory only** for the lifetime of the connection (prompted on connect, zeroed/dropped on disconnect, never written to disk). Connection *profiles* — host, port, user, key file path, start directory — persist in `prefs.toml`. Known-hosts is trust-on-first-use with an explicit fingerprint confirm dialog; a changed key refuses the connection with an actionable error.

**Operations across the boundary:** `ops` streams `open_read`/`open_write` for every local↔SFTP pair (progress + cancel work the same); same-authority moves use `Vfs::rename`. Dragging files from a local pane onto the SFTP pane's tree or listing is exactly the §8 flow — the payload carries `VfsUri`s, so nothing in the UI knows which side is remote.

**Implementation notes (change 4):** connections are keyed by authority (`user@host[:port]`) in `RemoteHub` (`engine/src/remote.rs`); `VfsRegistry` registers one `fm-fs-sftp` backend per authority, so two servers coexist. Host keys live in owner-only `known_hosts.json` (TOFU: an unknown key parks in a pending confirm dialog, a changed key is a hard refusal plus a `host_key_alert` card; there is no in-app replace path yet). Profiles are `Prefs.profiles` (no secrets). Transport loss flips the connection to `Failed`, panes keep their last listing (stale banner, mutating commands disabled), and `remote.reconnect` reuses the in-memory credentials. A 5 s poll (`DirCache::refresh`, no blanking) and window-focus regain refresh visible remote panes; free space is cached 30 s per location.

**Sequencing:** `fs-sftp` lands in change 4 (`sftp-backend`), but pane code is written against the `Vfs` trait from change 2 on — tests use `fm-testkit`'s in-memory remote backend, so "remote fills a pane" is provable before the real SSH code exists.

---

## 5. Branding / rename story

The product name must be changeable in one commit. Therefore:

- All user-visible identity lives in **`crates/brand`**: `APP_NAME`, `APP_SLUG`, `APP_ID`, `ORG_NAME`, `EXECUTABLE`, `DESKTOP_ID`. Nothing else hard-codes the name — window title, About, installers, desktop entry, file associations read these constants.
- Crate and directory names use the neutral `fm-*` prefix (`fm-vfs`, `fm-app`), so a product rename never touches crate names.
- `cargo xtask rename "NewName"` rewrites `crates/brand`, the `[[bin]]` name, desktop/NSIS/DMG identifiers, and runs `cargo xtask brand-check`.
- `cargo xtask brand-check` fails if the previous name appears anywhere outside `openspec/` and `log/`.

The application mark is a yellow folder with a shallow gold tab and an inset blue
right panel enclosed by a continuous yellow frame. `fm-brand::ICON_PNG` selects the platform's 256 px artwork: flat
silhouette on Linux, shaded silhouette on Windows, rounded light tile on macOS.
SVG masters and the regeneration script live in `crates/brand/assets`; generated
packaging assets use those same treatments (Linux PNG/SVG, Windows ICO, macOS ICNS).

---

## 6. Engine: session, commands, state

### Dialog presentation

`fm-ui::dialog_style` owns common floating-window chrome and locally scoped
typography; the app's About dialog uses it too. Windows start centered (Jobs starts
bottom-right), are draggable rather than anchored, and always expose a title-bar
close button. Each caller maps closing to its existing cancel/dismiss command,
never to an affirmative action. See [the dialog style guide](style-guide.md).
The main interface retains its existing typography.

### Release checks

`engine::Updates` owns one bounded-result worker for startup/manual GitHub release
checks. It is separate from the filesystem lanes so an offline update server
cannot delay directory reads. Requests use verified HTTPS, no redirects, a 15 s
timeout and a 1 MiB metadata limit. `Session::poll` drains results without waiting.
Startup checks default on (`Prefs.check_updates`); only available updates open
the Activity panel, with a dismissible download CTA alongside any file operations.
Checks in progress and no-update results never open the panel. Errors use the
status bar; manual checks also report no newer release there.
The download command opens the official release page on a worker; no code is
downloaded or installed by the app. Signed in-app installation is a later change.

Windows executables embed the branded ICO via the app's build script. This is
separate from the runtime window icon and supplies Explorer/shortcut branding;
the packager also uses that ICO for its NSIS installer.

On Linux, the runtime application ID uses `WINDOW_CLASS` (the slug), matching
the packaged `<slug>.desktop` filename and `StartupWMClass`. Development runs
can register the same identity and a per-user icon with
`cargo xtask desktop-install`; the standalone tool reads identity from the
brand source, like the packager config generator.

`fm-engine` owns all mutable application state; the UI borrows it per frame.

```rust
pub struct Session {
    pub registry: Registry,         // command id -> CommandSpec
    pub prefs: Prefs,
    pub jobs: JobQueue,
    pub layout: Layout,             // pane tree + per-pane state (from fm-layout)
    pub running: bool,              // app.quit flips it; the shell closes the window
    pub about_open: bool,           // app.about raises it; the shell shows the dialog
    pub dirs: DirCache,             // uri -> DirState (Absent|Loading|Ready|Failed), revisioned
    pub backends: VfsRegistry,      // scheme -> Arc<dyn Vfs>; LocalFs registered by default
    // private: inbox (staged reads), reads/inflight (dedup), indexes (per-pane sort views)
}

pub struct CommandSpec {
    pub id: &'static str,           // e.g. "pane.swap", "nav.open", "view.sort.size"
    pub label: &'static str,
    pub menu: &'static [&'static str],  // non-empty; drives the app menu catalogue
    pub shortcut: Option<&'static str>, // display form ("Ctrl+R"); the egui binding table must agree
    pub params: &'static str,       // "none" | human doc; UI tests dispatch Null only when "none"
    pub enabled: fn(&Session) -> bool,
    pub run: fn(&mut Session, &Params) -> Result<(), FmError>,  // Params = serde_json::Value
}
```

- `Session::dispatch(id, params)` is the **only** entry point for behaviour — menus, shortcuts and (later) the control channel all call it. Dispatch caps params (size/depth) before a command sees them and does **not** check `enabled` — a command must defensively no-op with `Ok(())` when disabled.
- Directory reads never block the UI: `Session::ensure_read/submit_read` enqueue on the scheme's lane (in-flight dedup), workers only *stage* results into an inbox, and `Session::poll()` applies them into `DirCache` (loading → ready | failed, strictly monotonic per-uri revision), prunes vanished selections and rebuilds per-pane sort indexes.
- Per-pane sorted views are engine-side: `Session::view_index/pane_view` return an `Arc` of entry indices ordered by the pane's `SortSpec`, cached per (pane, revision, sort) and invalidated on sort changes — sorting never re-reads the disk.
- Pane/tree/listing view state (scroll, expansion, sort, selection by **name**, history, tree width) lives in `fm-layout`'s per-pane state so it survives pane splits and can be serialized later.
- New commands live in their own module (`engine/src/<area>_cmds.rs`) and are appended to `commands::specs()`; the UI's id catalogue is cross-checked by `cargo xtask brand-check`.

### Layout model

```rust
pub enum PaneNode {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, a: Box<PaneNode>, b: Box<PaneNode> },
}

pub struct PaneState {
    pub location: VfsUri,          // current directory of the listing
    pub tree: TreeState,           // root (always expanded), expanded set, scroll
    pub listing: ListingState,     // SortSpec, selection (names), focused row, scroll
    pub history: History,          // back/forward stacks (current() -> Option)
    pub tree_width: f32,           // drag-resized tree column
}
```

Splits are pure data: `Layout::split(pane, axis)` / `close(pane)` / `swap(a, b)` / `set_ratio(handle, r)` (handle = left/top leaf id), plus focus bookkeeping (`focus_pane`, `swap_focused_with_previous` for Tab, `focus_next`) — unit-tested for ratio clamping, single-pane degenerate cases and dangling-focus regressions. Arrangement (change `pane-arrange`): `Layout::move_pane(pane, target, edge)` detaches a leaf and re-inserts it beside the target (state and terminals are keyed by `PaneId`, so they travel along); `equalize` (weighted by leaf count), `rotate`, `resize(pane, axis, delta)` and `toggle_zoom` (`Layout.zoomed`, view state dropped by `fix_zoom` when focus changes or the pane closes). Engine commands live in `arrange_cmds.rs` (`pane.move/equalize/zoom/rotate/resize.*`). Dragging a pane by its outer border (a 5 px band inside the frame; it highlights on hover) carries `fm_ui::arrange::PaneDrag` (separate from the file `DragPayload`); `slot_for` maps the pointer to an edge band or centre (swap) and the drop dispatches `pane.move`. Dividers (`fm-app`) highlight on hover and reset to the equal split on double-click.

`PaneState::up_target()` returns `None` at the pane's tree root so `nav.up` is a no-op there.

---

## 7. Concurrency model

```text
UI thread (eframe)                 engine workers                backends
┌────────────────────┐   commands   ┌──────────────────┐
│ read Session state │─────────────▶│ local lane (4 t) │──▶ std::fs
│ dispatch(command)  │              │ smb   lane (1 t) │──▶ smb (blocking, pure Rust)
│ poll JobQueue ─────│◀── events ───│ sftp  lane (2 t) │──▶ russh (tokio inside)
│ request_repaint()  │              └──────────────────┘
└────────────────────┘
```

- **Lanes, not a shared pool:** `local` (4 threads), `smb` (2 threads), `sftp` (2 threads). A hung share blocks only the `smb` lane.
- **Channels:** `crossbeam-channel` for job submission/events. The UI drains job events once per frame (`Session::poll()`), applies them to `Session`, then repaints if anything changed. Workers never touch egui.
- **Read lane in use today:** directory reads (`ensure_read/submit_read/refresh_read`) run on the scheme's lane; the worker calls `Vfs::read_dir` and *stages* a `ReadOutcome` (entries or error) into the session inbox — only `poll()` writes the `DirCache`. One in-flight read per uri (dedup), `refresh_read` invalidates then re-reads.
- **File jobs (landed, change 3):** `engine::fileops` holds a jobs table (`OpJob { id, label, state, progress, conflict, summary }`). `Session::submit_op(OpRequest)` queues a copy/move/trash/delete/mkdir/rename/undo job on the source's lane; the worker throttles progress (30 ms) into shared state and `poll()` copies it into the table — the UI never waits. Cancel is an `Arc<AtomicBool>` checked between chunks.
- **Conflicts** are resolved *during* the job: the worker parks on a decision channel and the UI shows the dialog from `FileOps::pending_conflict()`, answering with `job.resolve_conflict {job, choice, apply_to_all?}`. Cancel (or a dropped channel) resolves as Cancel, so a parked worker is never leaked.
- **Undo/trash:** delete (F8) moves local entries to the platform trash and records an `UndoRecord` (created / moved / trashed / renamed); `edit.undo` replays it with an actionable error when stale. Remote URIs never fall back to a permanent delete; `file.delete_permanent` needs `confirm: true`. macOS cannot restore from trash (documented).
- **UI contract:** dialogs and the Activity panel (`fm-ui::dialogs`, `fm-ui::jobs`) only dispatch command ids; dispatch failures set `Session::notice`, which `fm-ui::jobs::show` moves into the bounded `Session::messages` log (`engine::messages`, command `messages.clear`). A new message opens the Activity panel for 5 seconds (kept open while hovered); the status-bar icon (`jobs::icon_button`) reopens it. Short confirmations still use `Session::info` in the status bar.
- **Thumbnails (`thumbnails-and-open`):** `engine::thumbs::ThumbService` owns its own pool (≤ 4 workers, *not* a job lane — a folder of images must not queue behind file copies) and a per-pane generation counter. The grid reports the visible window (`preview.thumbs.visible`); the engine queues visible entries, then one screenful after and one before, and drops results whose generation is stale. Decoded RGBA lives in a 192 MiB LRU; PNGs go to `<cache>/thumbs` (cache parent configurable via `thumb_cache_dir`; 256 MiB default cap, configurable; `0600`/`0700`, temp + rename). Only `file://` entries are thumbnailed.
- **Markdown images (`markdown-images`):** `engine::md_images::MdImageService` has two small pools (local, net; 2 workers each, not job lanes) and a 256 MiB / 256-entry LRU keyed by (document, mtime, url). The Markdown preview reports the image URLs it needs (`preview.md_images.want`); relative paths resolve like `preview.open_link` and load through the document's `Vfs`. Remote http(s) loads only with `preview_remote_images` and uses a DNS guard that refuses non-public addresses, ≤ 3 revalidated redirects, a 10 s timeout and an 8 MiB cap. SVG is rasterized by `fm-preview::svg` (resvg, no text, no external resources).
- **Markdown HTML subset (`markdown-html-subset`):** `fm-preview::markdown::html` is a tolerant tokenizer/builder (allow-listed tags and attributes, 256 KiB per block, depth 32, `script`/`style` dropped) that lowers HTML blocks and inline HTML into the normal model: `MdBlock::Centered`, `MdImage.dark_url`/`width`, headerless `Table`s. `<picture>` keeps both URLs; the UI picks `url_for(tokens.dark)`, so only the visible image is requested.

- **Walkers outside the lanes:** Properties folder sizes (`props_cmds`, thread `fm-props`), Find Files (`search_cmds`, `fm-search`), the Open With application scan (`openwith_cmds`, `fm-apps`) and Media subtitle discovery/extraction (`media_subtitle_cmds`, `fm-subtitles`; extracting an embedded track reads the whole container) run on their own short-lived threads so a long walk never queues behind file jobs. Each publishes into a `Mutex` the UI reads every frame and stops on an `AtomicBool` set by its owner's `Drop` (closing the window cancels the walk; minimising a search to the Activity panel keeps it running). On a native local filesystem Find Files walks with up to 8 threads over `std::fs::read_dir`, reads file types from the directory entry and stats only matches; content search uses `memchr::memmem`. Other backends keep the single-threaded `Vfs` walker. A search of `/` skips pseudo-filesystems (`/proc`, `/sys`, `/dev`, `/run`; macOS system volumes). Walks never follow symlinks, are capped (5 M entries / depth 256 for sizes; 10 k results, depth 64 and 64 MiB read per file for search) and skip SMB, which must stay on its lane.

### Mounted devices and volumes

`Session::volumes` (`engine::devices`) caches mounted removable and user volumes,
separately from `Session::devices` (the drag/drop same-device cache). The shell
calls `devices_tick(focused)` and schedules repaint without input. A single-flight
`devices-scan` worker polls every 2 s focused / 10 s unfocused and on focus regain;
`poll()` applies snapshots and connection notices. Pending/failed scans retain the
last list; successful scans are applied even when slow. A scan still pending after
10 s reports one warning. A hung worker is not replaced until it exits, so it cannot
starve the local job lane or accumulate threads. Automatic polling waits its
interval after completion, avoiding continuous rescans on slower systems.

Linux reads bounded mountinfo plus sysinfo capacity and volume labels; macOS uses
sysinfo and diskutil plist metadata; Windows uses fixed PowerShell/CIM queries.
Windows fixed drives are filtered through logical-disk/partition/physical-disk
associations: internal physical disks are hidden, USB-backed disks and virtual
drive letters without a partition association remain eligible. Removable drives
and network mappings remain eligible independently of physical backing.
Windows eject revalidates the selected drive's serial-backed identity inside an
STA PowerShell process and keeps Shell COM references alive until the drive
disappears or the wait fails. Only the selected row shows pending removal;
other removal actions are temporarily disabled with an explanatory tooltip.
System/pseudo mounts are excluded, but virtual/encrypted mounts are not excluded
solely for being non-removable. Unavailable capacity is explicit.

Files panes and Go show Devices only when nonempty. `devices.refresh`,
`devices.eject {uri}` and `devices.dismount {uri}` own behaviour. Removal runs on
the local lane, validates the mount identity again, refuses active file transfers,
moves affected open/parked workspace panes Home, and confirms disappearance.
Linux only powers off a verified external disk when no sibling mount uses it;
macOS uses volume-scoped unmount, and Windows eject is limited to identified
removable drives. Unknown encrypted/virtual volumes remain browseable with a
disabled Dismount action directing users to VeraCrypt/OS tools. No unlocking,
force removal, passwords or generic hardware eject of encrypted mappings.

Disappearance invalidates stale/in-flight listings and blocks new file operations
below the old mount point until reconnection, rather than copying into its now
unmounted backing directory.

### Watching

Oversized OS events (more than four paths) request a debounced rescan of all
active watches rather than dropping uninspected paths. Failed watcher startup
and individual watches retry with exponential backoff from 1 second to a
60-second cap while polling continues. Successful retries clear backoff;
leaving a folder drops its retry. Known network folders remain polling-only,
and folders over the watch cap are reconsidered when capacity is freed.

`DirCache::invalidate(uri)` (revision bump) is the only mutation path — the cache is invalidated, never patched. `engine::watch::Watcher` (the `notify` crate: inotify / FSEvents / `ReadDirectoryChangesW`) holds one non-recursive watch per *visible* local folder (reconciled every `Session::poll` from the active tab's Files panes and their expanded tree nodes, pane folders first so the cap never starves a listing; shared folders share one watch and hidden tabs hold none). The OS callback queues nothing: it only marks an already-watched folder dirty in a set bounded by `MAX_WATCHES` (64) and wakes the UI, so event floods cost no memory. `drain` debounces (100 ms) and the session re-reads through `poll_location` on the `local` lane, swapping entries in without a loading blank; selection prunes by name. Backend errors/overflow mark every watch dirty. A vanished folder shows the read error. Folders on network filesystems/UNC paths, over the watch cap, or whose watch fails fall back to a 2 s poll; remotes keep focus-regain + the 5 s poll. The OS watcher is created lazily on first use.

---

## 8. Drag & drop contract

**Payload** (`fm_ui::dnd::DragPayload { sources: Vec<VfsUri>, origin: PaneId }`, set with `Response::dnd_set_drag_payload`): the whole selection when the dragged row is selected, otherwise just that row; tree nodes drag one uri. Rows, tree nodes, the `..` row and each pane register *drop zones* per frame; `dnd::finish` picks the most specific zone under the pointer, asks `Session::preview_drop` (engine `resolve_drop`) for the action, paints outline + badge, and on release dispatches `dnd.drop`. The UI never decides Copy vs Move.

**Resolution at release** (modifiers read from `ctx.input(|i| i.modifiers)` on the release frame):

| Modifier | Action |
|---|---|
| none | move if `same_device(src, dst)`, else copy |
| Ctrl | always copy |
| Shift | always move |
| Ctrl+Shift | link (symlink) — later change, rejected in v1 |
| Escape / drop on self / drop on ancestor of self | cancel |

**Drop targets and computed destination:**

| Target | Destination |
|---|---|
| tree node (directory) | that directory |
| tree node (file) | the file's parent |
| tree background / tree root label | the tree's current root |
| listing row (directory) | that directory (highlight the row) |
| listing row (file) | the current directory (i.e. beside the file) |
| listing background / `..` row | the pane's current directory (the `.` row is not a drop target) |
| another pane's border/chrome | rejected (no drop) |

Guards enforced by `ops` regardless of UI: never drop a directory into itself or a descendant; never overwrite without an explicit conflict decision; never copy a file onto itself (same uri) — silently ignored.

**Drag out** (files to other apps): `fm-dragout` (L0) is a pure-Rust XDND v5 source over `x11rb`. `fm-app/src/dragout.rs` watches an in-app drag; when the pointer leaves the window it starts an OS drag carrying the payload as `text/uri-list` (RFC 2483, capped). The source polls the real pointer, finds the `XdndAware` window under it, and drops on release. Under Xwayland, owning `XdndSelection` while the button is held makes the compositor take over the drag (button state then reads "up"), so the selection is claimed only just before the drop and the target is re-entered so it reads the offered types. Wayland-native sessions use `fm-wayland`. On Windows `fm-winout` (L0, `windows` crate) runs `DoDragDrop` on the UI thread (OLE reads the button state and takes the mouse capture from the calling thread; a helper thread saw the button as released and ended the drag at once) with a minimal data object offering only `CF_HDROP` (a shell data object also advertised other formats and Chrome's image-search drop then lost the files' types) and an `IDropSource` that ends on button release or Escape; the call blocks until then and the capture is released afterwards. Paths are validated in safe code (`paths.rs`: absolute, no NUL, count cap, `\\?\` prefixes stripped); only `ole.rs` uses `unsafe`. The source never reads, copies or deletes the files; a receiver that moves them does so itself. macOS is not covered.

**External drops** (files from the OS): `ViewportBuilder::with_drag_and_drop(true)`, `ctx.input(|i| i.raw.dropped_files)`; the pane under the pointer (`dnd::pane_at`, focused pane as fallback) becomes the destination and the paths go through `dnd.drop` with `origin: external` (always copy; bad paths are rejected individually and counted); payloads are native paths → imported through `ops` (copy in v1; move when the source is an app that reports a move intent).

**Media drops:** internal and external paths dropped onto Media dispatch
`media.drop` instead of `dnd.drop`; modifiers never copy, move or delete files.
The hover badge says "Add to media queue"; validation and folder expansion run
off-thread. Remote media remains unsupported until the spool slice.

**Visual feedback:** hovered target gets an accent outline + badge (`Copy 12 items` / `Move 3 items`) with the resolved action from the modifier table; invalid targets use `CursorIcon::NoDrop`.

---

## 9. UI architecture (`fm-ui` + `fm-app`)

- **Pane chrome (`fm-ui::pane`):** location bar with focus marker, focus ring (accent stroke on the focused pane), tree/listing split with a drag handle that writes `PaneState::tree_width`; a click inside the pane focuses it. Returns an "open" request; the shell dispatches `nav.open`.
- **Listing (`fm-ui::listing`):** fixed row height + `ScrollArea::show_rows` (virtualized) so 200 000 rows cost one screenful of widgets; columns name/size/mtime/type with header-click sort (dispatched into the engine's per-pane view index, never a re-read); `.` (the pane's own folder) and `..` rows, with navigation leaving the cursor on `.`; click/ctrl/shift selection; keyboard (arrows/Home/End/Ctrl+A/Enter) on the focused pane; loading / empty / failed (+Ctrl+R hint) states.
- **Tree (`fm-ui::tree`):** lazy — rows are flattened from `DirCache` for expanded nodes only (row cap 10 000); expand on click/Right, collapse Left, Enter opens; loading/error affordances inline; long names ellipsize with a full-path tooltip.
- **Shell (`fm-app`):** recursive `render_pane_node` walks `PaneNode`; splits lay out two children with a drag handle writing `Layout::set_ratio` (clamped); the status bar shows the focused pane's location, item count, selection count and theme; `Session::poll()` runs once per frame and the startup read is submitted before the first paint.
- **Shortcut routing:** `fm-app`'s catalogue maps keys → command ids (`Ctrl+Q` quit, `Ctrl+Shift+T` theme toggle, `Tab` pane.swap, `Backspace` nav.up, `Alt+Left/Right` nav.back/forward, `Ctrl+R` nav.refresh, `Ctrl+H` view.hidden.toggle (dot-files hidden by default, session-wide)); every entry must exist in the registry or the crate tests + `brand-check` fail it.
- **Dialogs:** conflict resolution, connect (SMB/SFTP), progress/Activity panel, error toasts — later changes, all `fm-ui` components driven by `Session`.

Terminal mouse input stays on the command path: `fm-term` exposes the parser's mouse
mode/encoding in `Snapshot.mouse`, `fm-ui::terminal` tracks pointer capture and cell
changes, and `engine::term_mouse_cmds` dispatches structured `term.mouse` events to the
running terminal. Protocol encoding happens in `fm-term` so legacy raw bytes are never
UTF-8 converted by `term.input`; the existing bounded writer queue keeps pty I/O off the
UI thread. Shift bypasses application mouse tracking for selection and scrollback.

Terminal resizing uses a focused vendored `vt100` patch (see `vendor/FM-VENDORED.md`):
normal-screen soft wraps and retained history reflow at the new column count,
preserving attributes, hard breaks and cursor positions. The packer streams rows
within the configured history plus screen bound, preserving the scrolled logical-line
anchor where retained. Alternate-screen applications keep the upstream grid resize
and redraw themselves after the pty resize. Grid-size changes still clear selection.
`prefs.set` validates `terminal_scrollback_rows` from 5 000 to 1 000 000 visual rows
(default 10 000); new/restarted `TermSession`s capture that capacity, and selection
eviction checks use each session's limit. History allocation remains lazy.

Accent text uses a separate `Tokens::accent_text` colour: custom accents are blended
toward the theme foreground only as needed for WCAG 4.5:1 contrast against normal,
panel and hover surfaces. Sidebar headings instead retain the original accent and
use a small horizontal overstrike for visibly heavier lettering with the bundled
monospace font (which has no bold face), without darkening the chosen colour.
Accent-labelled paths, directory text
and links use it; focus strokes, selection tints and decorative accents retain the
user's original `accent` colour.

Listing column widths are preferred widths in `PaneState.col_widths` (zero means
content-fit), changed through `view.columns.resize`. The UI caches bounded content
measurements by directory revision and font/scale, leaves Name flexible, and fits the
preferred widths continuously to the available space without persisting compression.
Divider drags use their original geometry and redistribute adjacent space; double-click
restores content fitting. Headers and rows share the geometry and clip truncated text
to each cell. Legacy fixed defaults migrate to content fitting during layout validation.

Log time presets and custom date/time fields dispatch the existing
`preview.log.time_range` command. Presets calculate From relative to current UTC time
on Apply and leave To open, so Live tail can include newly appended records. From stays
fixed after Apply; this is not a sliding-window filter. Custom bounds retain calendar
selection, typed dates/times, optional endpoints and inline validation. Filtering affects
only loaded records and selecting a preset performs no additional log scanning or I/O.

### 9.1 Theming and dark mode

#### File timestamp presentation

File timestamps share `listing::format_mtime` across listings, Properties and
conflict dialogs. `Session::new` resolves the operating system timezone with Jiff
before the UI starts; the shell supplies this already-loaded zone through
`listing::set_time_zone`. Rendering performs only in-memory conversions, using
the offset/DST rules at each file's timestamp rather than today's offset.
Detection failure is reported in the status bar and stderr with an explicit UTC
fallback. `Session::default` and standalone widget tests use UTC for deterministic
fixtures. OS timezone changes require restarting the app; the standard `TZ`
environment override is supported. UTC log filtering is independent.

#### Theme presentation

Dark mode is a v1 feature, not a later polish pass: **every widget is built against semantic tokens from day one, so both themes are always renderable.**

- **Tokens, not colours.** `fm-ui/src/theme.rs` defines one `Tokens` struct of *semantic* roles — `bg`, `panel`, `text`, `text_muted`, `accent`, `border`, `selection_bg/selection_text`, `hover_bg`, `error`, `warning`, `success` — plus non-colour metrics (row height, padding, corner radius, border width, font size, terminal font size). `Tokens::resolve_with(&Prefs, dark)` layers the user's accent, density and text sizes on the palette; `apply_to` scales text styles from egui's defaults so repeated changes never compound. Size and date formats reach the UI through a thread-local set each frame (`listing::set_display`). Single-value settings go through `prefs.set` (`display_prefs_cmds.rs`). Widgets take `&Tokens`; a widget that asks egui for `Color32::GRAY` is a bug and fails CI grep.
- **Two token sets.** `Tokens::dark()` and `Tokens::light()` are exhaustive siblings with the same field list, so adding a token forces both palettes to be defined (a compile-time guarantee — no `..Default::default()` escapes).
- **Preference model.** `Session::prefs.theme: egui::ThemePreference` = `System` (default) | `Light` | `Dark`. At startup and on change, `fm-app` calls `ctx.set_theme(pref)`; `ctx.system_theme()` supplies the resolved theme when `System` is selected. egui's `Theme`/`ThemePreference` types are used directly rather than a home-grown enum, so system-following works for free on all three platforms.
- **Commands** (so the switch is scriptable later): `view.theme.dark`, `view.theme.light`, `view.theme.system`, `view.theme.toggle` — registered in the engine like every other behaviour, menu path `View ▸ Theme`, shortcut `Ctrl+Shift+T` for toggle. They mutate `Prefs` only; the next frame re-renders.
- **Persistence:** `engine::prefs` reads/writes `prefs.toml` under the platform config dir (`$XDG_CONFIG_HOME/<slug>`, `~/Library/Application Support/<slug>`, `%APPDATA%\<slug>`) — written on change, loaded before the first frame. Session/pane persistence is a separate (later) concern; only prefs live here in v1.
- **Native window geometry:** the shell captures the last non-minimized normal size (native logical pixels), outer position (physical desktop pixels) and native scale in `Prefs`, alongside maximized state. The existing atomic, owner-only preferences writer saves these once they have been stable for a second and again on exit, so killed processes keep the geometry. A window manager that places the new window itself is corrected by re-sending the position for 1.5 s after launch (saving is suspended meanwhile). Startup restores them during native window creation, rejects invalid geometry, bounds size to available monitors and avoids positioning on a disconnected monitor. Native Wayland retains compositor-controlled placement.
- **Derived egui visuals:** `fm-app` maps the resolved theme to `Theme::default_visuals()` and then overrides only the tokens egui needs (panel fill, selection, hyperlink, faint grid), so egui's built-in widgets (scrollbars, tooltips, menus) match without hand-tuning 40 fields.
- **Testing:** `egui_kittest` snapshots run **twice** (dark and light) for the pane/tree/listing/dialog states; a token-coverage test asserts `Tokens::dark()` and `Tokens::light()` have identical field sets (struct-of-fields, destructured exhaustively in both constructors), and a CI grep forbids literal `Color32::` outside `theme.rs`.

---

## 10. Errors, safety, testing

- `FmError` (workspace error type in `fm-vfs`, hand-rolled `Display` that **redacts credential shapes** on every string it carries) with variants: `NotFound`, `PermissionDenied`, `AlreadyExists`, `NotADirectory`, `NotEmpty`, `Cancelled`, `Unsupported`, `Backend { scheme, message }`, `InvalidUri`, `Io`, `UnknownCommand`, `InvalidParams`, `DuplicateCommand`. UI maps each to a toast with a retry where meaningful.
- Workspace lints: `unsafe_code = "forbid"`; clippy `unwrap_used`/`expect_used`/`panic`/`todo` denied outside tests.
- **Test layers:** `vfs` (uri parsing/property tests), backends (tempdir round-trips; `testkit` in-memory backend for everything else), `ops` (progress/cancel/conflict on synthetic trees), `layout` (split/merge invariants), `engine` (command fuzz: every command × malformed params → `Err`, not panic), `ui` (`egui_kittest` snapshots for 1-pane, 2-pane, drag-hover states — each captured in dark **and** light, see §9.1).
- CI (`cargo xtask ci`): `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test --workspace`, `cargo xtask layers`, `cargo xtask brand-check` (stale-name scan, `Color32::` literal scan outside `theme.rs`, menu/shortcut-vs-registry id check).

---

## 11. Platform notes

| Concern | Linux | macOS | Windows |
|---|---|---|---|
| Roots | `/` + `/media/$USER`, `/mnt`, `/run/user/$USER/gvfs` | `/`, `/Volumes` | `C:`–`Z:` (mounted only) |
| Network discovery | none in v1 (type the path); gvfs mounts appear as local | none in v1 | none in v1 |
| SMB | `smb` crate | `smb` crate | `smb` crate |
| Trash | XDG (`trash` crate) | `~/.Trash` (`trash`) | Recycle Bin (`trash`) |
| Watch | inotify | FSEvents | ReadDirectoryChangesW |
| Packaging | `.deb`, AppImage | `.dmg` (signed/notarized) | `.msi` / portable zip |

Winit/egui have no Wayland-native **window-wide** file drop in some setups; external drops fall back to "paste from clipboard" (documented in `docs/development.md`), internal pane-to-pane DnD is unaffected (it never leaves the app).

---

### Managed tools (`managed-ffmpeg`)

`fm-engine::ffmpeg_install` can download ffmpeg/ffprobe on Windows (command
`tools.ffmpeg.install`, Preferences > Media). Lookup order in `fm-preview` is
`$PATH`/known locations first, then `<local data dir>/tools (`%LOCALAPPDATA%` on Windows)` (`set_managed_tools_dir`),
so a system install always wins. Rules: a compiled-in `Pin` (https URL, exact
size, SHA-256 cross-checked against the publisher); https-only with a redirect
policy and size cap; the hash is verified before anything is unpacked; only the
two named `bin/` members are extracted, to fixed names (no paths, symlinks or
duplicates); temp files are persisted atomically and `manifest.json` is written
last; the folder is owner-only on Unix. Updates arrive only with a new app
release carrying a new pin. Linux and macOS have no pin (package manager hints).

## 12. Security and threat model

Security is scaffold work (OpenSpec capability `security`), not a later pass. Three attacker classes, each with named defences:

| Attacker class | Examples | Defences (all test-backed) |
|---|---|---|
| **Hostile files & names** on any volume | 4 KiB filenames, non-UTF-8 names, symlink farms, a "directory" that is a fifo/device, junctions on Windows | `VfsUri` parsing is total and rejects `..`/NUL/overlong (property tests); `ops` never follows a symlink that resolves outside the destination root (`SymlinkPolicy`, enforced from change 3); entries are data, never interpreted as paths by the UI |
| **Hostile remote servers** (SMB/SFTP) | fake listings, absurd entry counts, error strings containing credentials, changed host keys | size/count caps on listings before allocation; `FmError::Display` redacts `user:password@` and `password=` shapes (test over every variant); trust-on-first-use host keys with explicit fingerprint confirm, changed key = hard refuse (change 4); secrets held in `secrecy`/`zeroize` types, never `Debug`-printed |
| **Hostile local input** | drop payloads, command params, `prefs.toml` overwritten with junk | param size/depth caps enforced in `dispatch` before any command runs; command fuzz suite (no panic, bounded allocation); prefs parsing capped at 10 MiB, corrupt → defaults, atomic temp-file+rename writes, mode `0600`/dir `0700` |

Standing rules (checked in review and by the `security` spec's tests):

- **No secrets on disk in v1.** `Prefs` is a closed schema with no slot that could hold a password; credentials are memory-only until a later change introduces an explicit, audited store.
- **State files are owner-only** (`0600`/`0700`; Windows restrictive ACL when that path is implemented).
- **No `unsafe` in our crates** — the workspace forbids it; FFI exists only inside vetted dependencies, isolated in L0 backends and listed in the dependency table (§4.4).
- **Bounded everything.** Any allocation sized by input (params, listings, path length, prefs file) has an explicit cap with a test at the cap and above it.
- **Errors are data.** Messages shown to the user or written to logs go through `FmError`, whose formatting redacts credential shapes — a panic-free app that leaks a password in a toast is still a security bug.

Change 3 extends `security` with drag & drop and `ops` rules (destination containment, overwrite policy); change 4 extends it with credential storage, host-key handling and remote listing caps.

---

## 12a. Git status and diff view (`git-status-and-diff`)

`fm-git` (L0, `gix`, read-only, isolated config) computes a per-repo status map (folders aggregate their strongest mark; capped at 200 000 entries) and loads blobs (HEAD/rev/index, 2 MiB cap). `fm-git::diff` aligns two texts with `similar` (binary detection, 100 000-line and 2 s caps, word marks). The engine's `git.rs` keeps an LRU of at most 8 repos, refreshed on the local lane after watcher events or a `.git/index`/`HEAD` stamp change; `fm-git::refs` lists branches, tags and recent checkouts (capped) for the revision picker. The diff is a view of the Preview pane, not a pane kind: `diff_cmds.rs` derives a `PaneDiff` per Preview pane from its subject (one local file inside a repo), defaults to Diff for Modified/Added/Renamed/Conflicted text files, keeps a sticky per-pane choice (view, revision within the repo, mode, EOL) across selections, loads both sides on the local lane keyed by the file's mtime+size, and drops results when the pane goes away (`preview.diff.toggle/picker/rev/next/prev/mode/eol/reload`; F7/Shift+F7 map to next/prev while a diff is shown). Hostile repos are inert: the repository is opened with isolated config (no user/system git config), repo-local `filter.*` drivers are stripped, and no hook, fsmonitor or textconv ever runs. `core.autocrlf` is honoured by re-checking modified files modulo CRLF (≤1000 per scan); on Windows `true` is the default. Measured: 50 000 files / 500 modified → 78 ms, 21 MB RSS. The UI draws marks in the icon gutter and name tint (`git_marks.rs`) and the Diff view inside the Preview pane (`diff.rs`: revision bar, picker, aligned/unified rows).

Split diffs share one horizontal scrollbar and pixel offset for both text
columns; horizontal gestures over either side stay linked. Gutters, row
backgrounds, divider and change markers stay fixed. A two-axis viewport paints
only visible rows with independent text clips per column. Measured line widths
are cached as rows are visited, keyed by document/font/mode; widths never feed
back into pane sizing. Unified diffs use the same virtualized scrolling.

## 13. Change history

| Date | Change |
|---|---|
| 2026-10-22 | Initial architecture (project bootstrap, before change 1). |
| 2026-10-22 | Added §9.1 theming/dark mode as a v1 requirement; added §4.5 SFTP-client-in-a-pane as an explicit requirement (connect flow, lane, credentials, TOFU host keys). |
| 2026-10-22 | Added §12 security threat model; `security` is a first-class capability of change 1 with test-backed requirements. |

### Preview pane (`pane-preview`)

`engine::preview_types` compiles persisted filename filters into a fixed-priority rule set for single/multi-file render workers and thumbnail candidacy. `prefs.preview.filters` validates an atomic replacement of the ordered `{type, patterns}` groups (array order is priority) or a reset, invalidates thumbnail associations and marks preview slots stale. `fm-preview::render_configured` accepts an optional renderer association, protects text renderers from binary input and uses content-only fallback when unmatched; the standalone legacy renderer retains its built-in classifier. HTML/CSV associations use text previews.

`fm-preview` (L1) turns a `Vfs` entry into a pure `Preview` model (text/structured/markdown/image/hex/dir/log/meta/error): magic-byte sniffing, syntect highlighting mapped to semantic `Class`es, capped reads, `catch_unwind` containment. `engine::preview_cmds` owns a `PreviewHub` of per-pane `PreviewSlot`s: the target is the Files pane's selected entries in display order, falling back to its cursor (or a pinned pane). Changes are debounced (120 ms), rendered on the pane's lane with a 15 s budget, and results come back through an inbox tagged with a token so stale renders are dropped. Single log files become a `LogView` over `Arc<RwLock<LogDoc>>`: tail-first 8 MiB window, "Load earlier" chunks (64 MiB cap), filters run as jobs and are accepted only for the matching `rev`, Follow polls every 500 ms and survives truncate/rotation. `fm-ui::preview` only draws; colours come from `Tokens::{syntax, level, status}`. Commands: `view.preview.toggle`, `pane.preview.source`, `preview.{zoom,toggle_wrap,toggle_pretty}`, `preview.log.*`.

`engine::preview_multi` combines up to 64 selected text/log sources into one
document with source-record ranges; images and mixed formats become a grid of
labelled preview cards. The total initial log read window is 8 MiB, expanded by
Load earlier up to 32 MiB, shared across sources; gzip decompression and record
counts are separately bounded (100,000 records across the selection). Source
failures and truncation remain visible. `preview.log.order` switches concatenation
to a stable chronological index (unknown timestamps last) on the filter worker,
so search/severity/time filters span all loaded sources without duplicating
their contents. Live tail is single-file only. Source filenames and fold controls
are excluded from text selection; log bodies use selectable labels and a
virtualized cumulative row index for multiline expansion.

Whole-document preview selection is model-backed in `engine::preview_text_cmds`,
not egui's visible-label selection: `preview.text.select_all` captures the loaded
document identity and raw/pretty mode; `preview.text.copy` exports its full bounded
text, excluding gutters and Markdown formatting. The UI highlights loaded text
and routes Ctrl/Cmd+A and copy only while the preview owns the keyboard.
Document replacement or mode changes make stale selections unavailable.

`preview.open {uri}` (`engine::preview_open_cmds`) is the explicit, non-toggle
item-context action. It validates the pointed URI against the loaded listing or
thumbnail grid, selects it in its source Files pane, and reuses an existing
Preview before creating a right split. The Preview is connected to that Files
pane; maximized-pane mode is cleared so it is visible. Thumbnail targets navigate
the connected Files pane to their parent when needed. Reads and rendering use
the existing asynchronous preview pipeline. The separate `view.preview.toggle`
command keeps its open/close semantics.

`engine::preview_find_cmds` owns transient per-slot Find options, navigation and
bounded match indices for loaded text, structured raw/pretty text and Markdown.
At most one worker per pane matches Unicode text/regexes; document identity,
mode and option checks discard stale results. Queries cap at 1024 characters,
compiled regex/DFA caches at 1 MiB each, and stored hits at 10 000. Text-line and
Markdown render-segment indices let the UI highlight syntax sections without
changing copyable text, retain unwrapped text virtualization, and reveal matches
through nested scroll areas. Find never performs filesystem I/O. Ctrl+F is
context-routed before the Files filter shortcut; in logs it opens/focuses the
existing record filter instead. Non-text/card-grid previews disable Find.
Find, zoom and content-specific controls share the source/title toolbar row,
using measured sizing and a truncated title to keep the overflow menu visible
in narrow panes. Logs keep their Filter toggle next to the source selector and
omit the redundant Find button.

Markdown uses `pulldown-cmark` to build a bounded recursive document tree in
`fm-preview::markdown`: nested lists/items/quotes, aligned table cells, hard breaks,
task state and inline strong/emphasis/strike/link/code runs. Code syntax spans
reuse `fm-preview::highlight` on the worker. Source/line, node, inline, depth and
table budgets set an explicit truncation flag. `fm-ui::preview_markdown` lays out
readable proportional documents, top-aligned fixed-column tables with shared row
heights, read-only task boxes, inset quotes and code panels. Tables scroll when
their minimum column widths exceed the pane. No HTML execution, URL activation or
image fetching is introduced. Clipboard export traverses the same tree; table
cells are separated with tabs and task markers retain their checked state.
Code panels reserve a header with a non-selectable language label and a painted
copy icon (50% idle opacity, full on hover/focus). The widget reports a code-block
ordinal to `preview.code.copy`; the engine resolves it in the loaded document,
including nested blocks and explicitly targeted multi-file cards, and publishes
only the code through the clipboard outbox. UI labels/tooltips are never part of
the Markdown export or selectable text.
Document-only Atkinson Hyperlegible regular/bold faces are bundled under the SIL
Open Font License in `crates/ui/assets/fonts/OFL.txt`; named font families leave
the application's normal UI/terminal faces unchanged.

`PreviewSlot::scale` holds pane-local 50%-400% content magnification, independent
of image Fit/Actual mode and `Prefs::zoom`. `preview.scale.{in,out,reset}` commands
drive common toolbar controls and focused-preview keyboard chords. Content uses
scaled tokens and a temporarily scoped egui style (restored before returning);
image/video sizes, thumbnail cells and multi-file cards also use the pane scale.
Source reads and decoded-image limits do not change. Existing Shift zoom chords
are routed to preview magnification only while a Preview owns the keyboard;
application-wide zoom remains available through the normal View menu.

### Thumbnails and file activation (`thumbnails-and-open`)

A Preview pane showing a directory can switch to a **thumbnail grid** (`preview.thumbs.*`): `fm-preview::thumbs` classifies entries (images, video posters, name tiles), decodes with the existing caps and resizes with `fast_image_resize`; `engine::thumbs` schedules only the visible window on its own worker pool (see §7) and caches results in memory (192 MiB LRU) and on disk (`<cache>/thumbs`, 256 MiB, owner-only). The grid is local-only and keyboard-navigable; remote folders keep the summary card.

**Activation** is a command path, not a UI shortcut: double-click/Enter calls `open_cmds::plan` (pure, from the cached listing) → `Navigate` / `Launch` / `ConfirmRun`. `file.open {uri, mode?, confirm?}` re-stats the entry, refuses special files and non-local uris, and starts documents via `opener` or programs via a direct detached spawn — never a shell, and never without `confirm: true` (the UI shows the *Run* / *Open as document* / *Cancel* dialog for programs). Listing paging (`PageUp`/`PageDown`) rides the existing move/extend machinery.

Dependencies added: `opener 0.9` (no default features; `xdg-open`-class hand-off on Linux, `ShellExecuteW`, `NSWorkspace`) and `fast_image_resize 6` (pure Rust, runtime SIMD). Neither needs system libraries or a C toolchain at build time. `fm-preview` is pinned to `opt-level = 3` in dev profiles because decode/resize generics are inlined into it.

### Workspace tabs (`workspaces`)

`Session::layout` is the active tab's layout; inactive tabs park theirs in `Tab.layout` (`engine::workspace`). Each tab gets a disjoint pane-id range so terminals, previews and view indexes of background tabs stay valid. Up to 10 workspaces persist in `workspaces.json` (owner-only, atomic, no secrets); the engine does no I/O: `workspace_tick()` returns the JSON and `fm-app` writes it (~1 s, and on exit). Commands: `workspace.*`.
