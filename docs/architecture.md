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
- Commander-style shortcuts (F5 copy, F6 move, F7 mkdir, F8 delete, Tab = swap panes).

**Non-goals (v1)**

- Archive browse/extract, image preview, tabs, dual-pane-only mode, plugins, file content search.
- CLI, JSON control channel and MCP — *the command registry exists from day one* (the UI dispatches through it), but the automation crates are a later change. Nothing else may bypass the registry.

---

## 2. Golden rules

These apply to every crate. They are checked in review and by CI (`cargo xtask layers`, `cargo clippy`).

1. **Never crash.** Non-test code contains no `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `unreachable!` — errors propagate as `FmError` with `?`. A malformed filename, a vanished file mid-copy, an unreachable SMB server, a full disk or a 0-byte write must produce a user-visible error, never a panic. Unimplemented backends return `FmError::Unsupported`.
2. **No `unsafe` in our code.** `unsafe_code = "forbid"` workspace-wide. FFI (libsmbclient) exists only inside third-party crates.
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
  fs-smb       L0  in-app SMB backend (pavao/libsmbclient; native UNC on Windows)
  fs-sftp      L0  SSH/SFTP backend (russh + russh-sftp, owns its tokio runtime)
  ops          L1  copy/move/delete/mkdir/rename pipelines: streaming, conflicts, progress
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
- Windows drive letters normalize to `file:///C:/...` internally and display as `C:\` in the UI (`fm-vfs/src/windows_path.rs`).

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
- `VfsRegistry` maps `Scheme -> Arc<dyn Vfs>`; a connect (SMB/SFTP) creates a session-backed instance and registers it.
- Backends are dumb: no caching, no sorting, no watching. That is `engine`'s job.

### 4.4 Backend choices

| Backend | Implementation | Notes |
|---|---|---|
| `fs-local` | `std::fs` + `notify` | Linux `/`, `/media/$USER`, `/mnt`; macOS `/`, `/Volumes`; Windows `A:`–`Z:` + UNC |
| `fs-smb` | Linux/macOS: `pavao` (libsmbclient, `vendored`); **Windows: native UNC via `WNetAddConnection2`, then the local backend** | avoids porting libsmbclient to Windows; SMB1 only on explicit opt-in |
| `fs-sftp` | `russh` + `russh-sftp` | one tokio runtime inside the crate; `Handle::block_on` satisfies the blocking `Vfs` contract |

`pavao` serializes **all** SMB calls process-wide, so SMB jobs get their own scheduler lane (§7) — a slow share can never starve local work.

### 4.5 Remote backends in a pane: the SFTP client

**Requirement:** an SFTP location is a first-class pane. Connecting fills the focused pane's file tree and listing from the remote host, and every pane feature (split, drag & drop, copy/move, shortcuts, selection, history) works across the local↔remote boundary without special cases.

**Connect flow** (all steps are commands; nothing blocks the UI thread):

1. `remote.connect` opens the Connect dialog (scheme `SFTP` | `SMB`, host, port, user, auth: *password* | *private key* + optional passphrase, starting directory).
2. `engine` opens the connection on the **`sftp` lane** (`russh` handshake + `sftp` subsystem, host-key check), emitting `JobEvent`s; the dialog shows a spinner and stays responsive.
3. On success the backend is registered in `VfsRegistry` under its authority (`user@host:port`) and the focused pane's `location` becomes `sftp://user@host/<start-dir>`; tree root and listing fill through the normal `ReadDir` → `DirCache` path. On failure: `FmError::Backend` → dialog shows the reason + Retry; the pane is untouched.

**Pane behaviour while remote — identical to local, with three differences:**

| Aspect | Behaviour |
|---|---|
| Reads | `read_dir`/`stat` go to the `sftp` lane; results land in the same `DirCache`, so panes/tabs sharing a directory share the cache. |
| Watching | No `notify` for SFTP — a pane refreshes on focus regain, on a manual `F5`, and on a 5 s poll while visible (§7). |
| Connection state | Per-authority state machine `Disconnected → Connecting → Ready → Failed(err)`, shown as a badge in the pane chrome. On disconnect/error the last listing stays visible with a *stale* badge and all mutating actions are disabled for that pane (retry via `remote.reconnect`), never a blank window and never a panic. |

**Credentials (v1):** passwords and passphrases live **in memory only** for the lifetime of the connection (prompted on connect, zeroed/dropped on disconnect, never written to disk). Connection *profiles* — host, port, user, key file path, start directory — persist in `prefs.toml`. Known-hosts is trust-on-first-use with an explicit fingerprint confirm dialog; a changed key refuses the connection with an actionable error.

**Operations across the boundary:** `ops` streams `open_read`/`open_write` for every local↔SFTP pair (progress + cancel work the same); same-authority moves use `Vfs::rename`. Dragging files from a local pane onto the SFTP pane's tree or listing is exactly the §8 flow — the payload carries `VfsUri`s, so nothing in the UI knows which side is remote.

**Sequencing:** `fs-sftp` lands in change 4 (`network-backends`), but pane code is written against the `Vfs` trait from change 2 on — tests use `fm-testkit`'s in-memory remote backend, so "remote fills a pane" is provable before the real SSH code exists.

---

## 5. Branding / rename story

The product name must be changeable in one commit. Therefore:

- All user-visible identity lives in **`crates/brand`**: `APP_NAME`, `APP_SLUG`, `APP_ID`, `ORG_NAME`, `EXECUTABLE`, `DESKTOP_ID`. Nothing else hard-codes the name — window title, About, installers, desktop entry, file associations read these constants.
- Crate and directory names use the neutral `fm-*` prefix (`fm-vfs`, `fm-app`), so a product rename never touches crate names.
- `cargo xtask rename "NewName"` rewrites `crates/brand`, the `[[bin]]` name, desktop/NSIS/DMG identifiers, and runs `cargo xtask brand-check`.
- `cargo xtask brand-check` fails if the previous name appears anywhere outside `openspec/` and `log/`.

---

## 6. Engine: session, commands, state

`fm-engine` owns all mutable application state; the UI borrows it per frame.

```rust
pub struct Session {
    pub layout: Layout,             // pane tree + per-pane state (from fm-layout)
    pub jobs: JobQueue,
    pub dirs: DirCache,             // uri -> (revision, Vec<Entry>)
    pub registry: Registry,         // command id -> CommandSpec
    pub watcher: WatchHandle,
    pub clipboard: Option<Clipboard>, // cut/copy buffer for Ctrl+C/V
    pub prefs: Prefs,
}

pub struct CommandSpec {
    pub id: &'static str,           // e.g. "edit.paste", "pane.swap", "file.copy"
    pub label: &'static str,
    pub menu: &'static [&'static str],
    pub shortcut: Option<Shortcut>,
    pub params: &'static str,       // human doc for future automation
    pub enabled: fn(&Session) -> bool,
    pub run: fn(&mut Session, Params) -> Result<(), FmError>,
}
```

- `Session::dispatch(id, params)` is the **only** entry point for behaviour — menus, shortcuts, toolbar buttons and (later) the control channel all call it.
- Commands that take longer than a frame enqueue a `Job` and return `Ok(())`; progress arrives asynchronously (§7).
- Pane/tree/listing view state (scroll, expansion, sort, selection) lives in `fm-layout`'s per-pane state so it survives pane splits and can be serialized into a session file later.

### Layout model

```rust
pub enum PaneNode {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, first: Box<PaneNode>, second: Box<PaneNode> },
}

pub struct Pane {
    pub id: PaneId,
    pub location: VfsUri,          // current directory of the listing
    pub tree: TreeState,           // root, expanded set, scroll, lazy children
    pub listing: ListingState,     // sort, selection, scroll, focused row
    pub history: History,          // back/forward stack
}
```

Splits are pure data: `layout::split(pane, axis)` / `layout::close(pane)` / `layout::swap(a, b)` (Tab), with unit tests for ratio preservation and single-pane degenerate cases.

---

## 7. Concurrency model

```text
UI thread (eframe)                 engine workers                backends
┌────────────────────┐   commands   ┌──────────────────┐
│ read Session state │─────────────▶│ local lane (4 t) │──▶ std::fs
│ dispatch(command)  │              │ smb   lane (1 t) │──▶ pavao (process-wide lock)
│ poll JobQueue ─────│◀── events ───│ sftp  lane (2 t) │──▶ russh (tokio inside)
│ request_repaint()  │              └──────────────────┘
└────────────────────┘
```

- **Lanes, not a shared pool:** `local` (4 threads), `smb` (1 thread — matches pavao's global lock), `sftp` (2 threads). A hung share blocks only the `smb` lane.
- **Channels:** `crossbeam-channel` for job submission/events. The UI drains job events once per frame (`Session::poll()`), applies them to `Session`, then repaints if anything changed. Workers never touch egui.
- **Job model:** `JobId`, `JobKind` (Copy/Move/Delete/Mkdir/Rename/Connect), `Progress { done, total, current_uri }`, `cancel: Arc<AtomicBool>` (checked between chunks), `JobEvent::Done(Result<usize, FmError>)`.
- **Conflict policy** is resolved by a UI dialog *before* the job starts (batch of decisions: overwrite / keep-both / skip / cancel), never inside a worker.
- **Undo/trash:** delete moves entries to the platform trash (`trash` crate) and records a journal entry (`trash://` ids + original uris) for Undo; `ops::restore` puts them back.

### Watching

`notify` (non-recursive, per watched directory) with a 100 ms debounce, owned by `engine`:

1. event → normalize to `VfsUri` → `DirCache::invalidate(uri)` → increment revision
2. panes whose `location` or tree node matches a changed uri re-read on next frame (worker) and repaint
3. remotes without a watcher (SMB/SFTP) refresh on focus regain + a 5 s poll while visible

The cache is invalidated, never patched — one code path, no stale-entry bugs.

---

## 8. Drag & drop contract

**Payload** (single type, set with `Response::dnd_set_drag_payload`, read with `dnd_release_payload`):

```rust
pub struct DragPayload {
    pub sources: Arc<[VfsUri]>,      // the selection at drag start
    pub origin: DragOrigin,          // Pane { id, dir } | External
}
```

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
| listing background / `..` row | the pane's current directory |
| another pane's border/chrome | rejected (no drop) |

Guards enforced by `ops` regardless of UI: never drop a directory into itself or a descendant; never overwrite without an explicit conflict decision; never copy a file onto itself (same uri) — silently ignored.

**External drops** (files from the OS): `ViewportBuilder::with_drag_and_drop(true)`, `ctx.input(|i| i.raw.dropped_files)`; the hovered pane at release becomes the destination; payloads are native paths → imported through `ops` (copy in v1; move when the source is an app that reports a move intent).

**Visual feedback:** hovered target gets an accent outline + badge (`Copy 12 items` / `Move 3 items`) with the resolved action from the modifier table; invalid targets use `CursorIcon::NoDrop`.

---

## 9. UI architecture (`fm-ui` + `fm-app`)

- **Pane chrome:** `fm-app` renders `PaneNode` recursively into `egui` panels with draggable split handles; ratio changes update `Layout` only (no I/O).
- **Listing:** fixed row height + `ScrollArea::show_rows` (virtualized) so 200 000 rows cost one screenful of widgets; columns (name/size/mtime/type), sortable via header click (sort runs in `engine` when the dir is read).
- **Tree:** lazy — children are fetched from `DirCache` on expand (one worker read), collapsed nodes keep no children in memory beyond the expansion cache; arrow keys/typing navigate.
- **Shortcut routing:** `fm-app` maps keys → command ids (`F5 → file.copy`, `F6 → file.move`, `F7 → file.mkdir`, `F8 → file.trash`, `Tab → pane.swap`, `Alt+Left/Right → nav.back/forward`); every map entry must exist in the registry or `brand-check`-style CI fails it.
- **Dialogs:** conflict resolution, connect (SMB/SFTP), progress/jobs panel, error toasts — all `fm-ui` components driven by `Session`.

### 9.1 Theming and dark mode

Dark mode is a v1 feature, not a later polish pass: **every widget is built against semantic tokens from day one, so both themes are always renderable.**

- **Tokens, not colours.** `fm-ui/src/theme.rs` defines one `Tokens` struct of *semantic* roles — `surface`, `surface_raised`, `text_primary`, `text_muted`, `accent`, `border`, `selection_bg`, `row_odd_bg/row_even_bg`, `drop_target_outline`, `danger`, `success` — plus non-colour metrics (spacing, radii, row height, font sizes). Widgets take `&Tokens`; a widget that asks egui for `Color32::GRAY` is a bug and fails review/CI grep.
- **Two token sets.** `Tokens::dark()` and `Tokens::light()` are exhaustive siblings with the same field list, so adding a token forces both palettes to be defined (a compile-time guarantee — no `..Default::default()` escapes).
- **Preference model.** `Session::prefs.theme: egui::ThemePreference` = `System` (default) | `Light` | `Dark`. At startup and on change, `fm-app` calls `ctx.set_theme(pref)`; `ctx.system_theme()` supplies the resolved theme when `System` is selected. egui's `Theme`/`ThemePreference` types are used directly rather than a home-grown enum, so system-following works for free on all three platforms.
- **Commands** (so the switch is scriptable later): `view.theme.dark`, `view.theme.light`, `view.theme.system`, `view.theme.toggle` — registered in the engine like every other behaviour, menu path `View ▸ Theme`, shortcut `Ctrl+Shift+T` for toggle. They mutate `Prefs` only; the next frame re-renders.
- **Persistence:** `engine::prefs` reads/writes `prefs.toml` under the platform config dir (`$XDG_CONFIG_HOME/<slug>`, `~/Library/Application Support/<slug>`, `%APPDATA%\<slug>`) — written on change, loaded before the first frame. Session/pane persistence is a separate (later) concern; only prefs live here in v1.
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
| SMB | `pavao` (vendored libsmbclient) | `pavao` (vendored) | `WNetAddConnection2` → UNC → local backend |
| Trash | XDG (`trash` crate) | `~/.Trash` (`trash`) | Recycle Bin (`trash`) |
| Watch | inotify | FSEvents | ReadDirectoryChangesW |
| Packaging | `.deb`, AppImage | `.dmg` (signed/notarized) | `.msi` / portable zip |

Winit/egui have no Wayland-native **window-wide** file drop in some setups; external drops fall back to "paste from clipboard" (documented in `docs/development.md`), internal pane-to-pane DnD is unaffected (it never leaves the app).

---

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

## 13. Change history

| Date | Change |
|---|---|
| 2026-10-22 | Initial architecture (project bootstrap, before change 1). |
| 2026-10-22 | Added §9.1 theming/dark mode as a v1 requirement; added §4.5 SFTP-client-in-a-pane as an explicit requirement (connect flow, lane, credentials, TOFU host keys). |
| 2026-10-22 | Added §12 security threat model; `security` is a first-class capability of change 1 with test-backed requirements. |
