# Development

## Prerequisites

- Rust stable via [rustup](https://rustup.rs) (`cargo 1.99+`). The workspace uses edition 2024.
- No system `-dev` packages are required to **build** (verified: `eframe 0.37` + winit/wgpu compile on stock Ubuntu 24.04 with only `gcc`, `make` present).
- A display is required to **run** the app (Wayland or X11). CI and headless work use the test/snapshot paths that never open a window.
- Linux runtime packages for the window itself: `libxkbcommon0`, `libwayland-client0`, `libGL.so.1`, `libEGL.so.1` — present on a normal desktop install; a bare server may need them (`sudo apt install libxkbcommon0 libgl1 libegl1`).

## Everyday commands

```sh
cargo run -p fm-app                 # run the desktop app
cargo test  --workspace              # the suite
cargo test  -p fm-ops                # one crate
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask layers                   # layering enforcement (fails on a bad dependency)
cargo xtask brand-check              # no hard-coded product name outside brand/
cargo xtask ci                       # everything CI runs, locally
```

Faster inner loop: `cargo nextest run` if `cargo install cargo-nextest` is available; while iterating use `cargo test -p <crate> --lib --tests` to skip examples/doctests.

Use your own target dir when working in parallel with another agent:

```sh
CARGO_TARGET_DIR=target/agent-<name> cargo test -p fm-engine
```

## Layout of the tree

See `AGENTS.md` §2. `docs/architecture.md` explains why each crate exists and how data flows.

## Platform notes

- **Wayland**: window-wide file drops from other applications depend on winit's Wayland support and may be unavailable; in-app pane-to-pane drag & drop is unaffected, and pasting paths from the clipboard always works. See `docs/architecture.md` §11.
- **Network discovery** (manual smoke; multicast cannot run in CI): File ▸ Network should list Samba/NAS hosts (mDNS) and Windows PCs (WS-Discovery) within ~5 s; opening one asks for credentials in a popup, a wrong password reopens it with an error, and `Remember until you quit` reconnects silently. VLANs/VPNs may hide hosts; use the Network pane's Connect… button (or File ▸ Connect to SMB Server…) as the fallback.
- **SMB**: pure Rust on every OS, no system libraries. The `fm-fs-smb` integration tests start `dperson/samba` in Docker and skip themselves when Docker or the image is missing.
- **macOS**: local roots are `/` and `/Volumes`.
- **Trash**: only local paths can be trashed (SMB/SFTP report `Unsupported`; permanent delete is always an explicit command). Undoing a trash restores via `trash::os_limited`, available on Linux and Windows; on macOS the `trash` crate cannot restore, so undo-delete is unavailable there (undo of rename/move/create still works).

## Adding tests

- Backend behaviour → `crates/fs-*/tests/` against `tempfile` tempdirs.
- Anything with side effects on a tree → `fm-testkit`'s in-memory `Vfs` (fast, no disk), including `hostile_tree()` for adversarial fixtures.
- Commands → fuzzed inside `crates/engine/src/commands.rs`: every command × empty/wrong-typed/oversized/deep params → `Ok`/`Err`, never panic.
- UI widgets → headless `egui::Context::run_ui` renders in `crates/ui/src/*` unit tests, plus hostile end-to-end renders in `crates/ui/tests/hostile.rs`.
- UI snapshots → `egui_kittest` image snapshots in `crates/ui/tests/snapshots.rs`, committed under `crates/ui/tests/snapshots/`.

## Updating UI snapshots

Snapshots need a GPU (or CPU) adapter for `wgpu`; on machines without one the
snapshot tests **skip themselves** (they print a note and pass) so headless CI
stays green. To (re)generate or verify:

```sh
cargo test -p fm-ui --test snapshots                      # verify against committed PNGs
UPDATE_SNAPSHOTS=1 cargo test -p fm-ui --test snapshots   # rewrite the committed PNGs
```

Commit the updated PNGs together with the change that altered the pixels.
Snapshot fixtures use a fixed mtime and synthesized error messages so the
images are machine-independent.

## Running the real window (manual smoke)

The windowed launch is `#[ignore]`d in CI. Run it by hand:

```sh
cargo test -p fm-app -- --ignored window_launches_and_closes --nocapture
cargo run -p fm-app   # or the installed binary name from fm_brand::EXECUTABLE
```

Manual checklist:

1. Window opens with the product name from `fm-brand` in the title bar.
No, titlebar is empty

2. Menu bar shows File / Panes / View / Help; `Ctrl+Q` quits, `Ctrl+Shift+T` toggles dark ↔ light.
Yes, works as expected

3. View → Theme → System follows the OS setting (toggle the OS theme and watch the window).
No, system does behave different to dark or light, but my system is set to dark and I would expect dark. I only see the menubar and the status bar go dark.

4. One pane shows a tree (left) and a virtualized listing (right); the status bar shows the focused pane's location, item count and theme.
Yes works

5. Panes → Split Horizontally/Vertically creates a second pane; click focuses a pane (accent ring), `Tab` swaps with the previous pane, `Panes → Close` removes it.
Everything works except Tab, this tabs through menu items, then direcotries in tree, then items in the file pane

6. Click a sort header to re-sort in place; `Ctrl+H`, View → Show Hidden Files or the status-bar button toggles hidden files (hidden by default).
Works, but size column is overflowed by date column, no clear column distinctions/boundaries.


7. `Backspace` goes up, `Alt+Left`/`Alt+Right` walk history, `Ctrl+R` refreshes; double-click or `Enter` opens a directory.
Works

8. Help → About opens a dialog with name, version and organisation; closing it does not quit.
Works, but it would be nice to have the dialog centered in the view window

9. Quit; `prefs.toml` appears in the config directory (see below) with mode `0600`.
Correct

### File operations & drag and drop (change 3)

Use a scratch directory with a few files, a subfolder and one name that exists in two places. Split into two panes.

1. Drag a file from one pane onto a folder row in the other: accent outline + `Move 1 item` (same device) or `Copy 1 item`. Hold Ctrl → `Copy`, Shift → `Move`, Ctrl+Shift or dropping a folder onto itself → no-drop cursor. Escape cancels the drag.
2. Select several files, drag one of them: the badge counts the whole selection. Drag a tree node onto the other pane's listing background.
3. `F5` / `F6` copy / move to the other pane; with a single pane a destination dialog asks. Same-named file: the conflict dialog offers Overwrite / Keep both / Skip / Cancel + Apply to all; Escape cancels.
4. `Shift+F7` new empty file (never overwrites); `Ctrl+,` opens Preferences (theme, re-target links on copy); copying a tree with absolute in-tree symlinks (e.g. `node_modules/.bin`) recreates them as links.
4. `F7` new folder (invalid or duplicate name shows the error and disables OK); `F2` rename.
5. `F8` moves to the OS trash, `Shift+Delete` asks for confirmation; `Ctrl+Z` undoes the last copy / move / rename / trash (macOS cannot restore from the trash — an error is shown).
6. A large copy shows progress in the status bar and jobs panel; Cancel stops it and leaves no partial files. Failures list per item and can be dismissed.
7. Drop files from the OS file manager onto a pane: they are copied into that pane's directory.


### Pane types & arrangement (changes `pane-types`, `pane-arrange`)

1. Pane header, right side: the type button (Files / Terminal / Preview; Sftp and Network show "(soon)"). Switching a pane with a running foreground job asks "End running process?".
2. Terminal: shell prompt in the pane's directory; `top`/`vim` redraw without mouse input; Backspace, Ctrl+C/D/L work; Ctrl+Shift+C/V copy/paste; drag a file from a Files pane (or the OS) onto it to paste its quoted path, and the terminal regains focus.
3. `Ctrl+Shift+E` split right, `Ctrl+Shift+O` split down (new pane next to the active one); `Ctrl+Shift+Arrows` resize the nearest divider, `Ctrl+Shift+B` equalize, `Ctrl+Shift+R` rotate, `Ctrl+Shift+X` zoom toggle. All also work with a terminal focused.
5. Preview: `Ctrl+Shift+P` opens a preview to the right of the active pane and toggles it closed. Arrow through text, JSON (Pretty button), Markdown, an image, a binary (hex) and a folder; the preview follows the cursor, or pin it with the source picker. A `*.log` switches to Log mode: level/search/time filters, Live tail (`Ctrl+Shift+F`) while another process appends, Mark (`Ctrl+Shift+M`) to drop a divider rule at the current end, folded stack traces, access-log columns. Video shows metadata and a poster frame when ffmpeg is installed (no playback yet).
4. Drag a pane by its outer border (the edge highlights) onto another: the outer quarter of an edge inserts there, the middle swaps; Esc cancels. Terminals keep running through moves.
5. Dividers highlight on hover; double-click resets to an equal split. In a Files pane drag the tree divider and the listing header borders (double-click resets).

### Thumbnails, opening files, paging (change `thumbnails-and-open`)

1. Open a Preview pane (`Ctrl+Shift+P`), navigate its source Files pane into a folder of images: the header offers a thumbnails toggle (also View → Preview Thumbnails). Tiles appear; scroll a large folder and confirm the UI never stalls and off-screen tiles are not all decoded.
2. View → Thumbnail Size cycles small → medium → large and the visible tiles re-render. Arrow keys / PageUp / PageDown / Home / End move the tile focus without the Files pane stealing the keys. View → Clear Thumbnail Cache empties it (tiles reload).
3. A folder on a remote location keeps the summary card (no downloads). A file edited on disk shows a regenerated thumbnail.
4. Double-click a document (e.g. a PDF or an image) in a Files pane: it opens in the system default application; the pane does not navigate.
5. Double-click an executable script: a "Run program?" dialog offers *Run* / *Open as document* / *Cancel*; nothing starts until chosen. Cancel leaves the app untouched.
6. A folder still navigates on Enter/double-click. Layout keys in the listing: PageUp/PageDown move a screenful, Shift extends the selection, the view scrolls to the focused row.

### SFTP connections (change `sftp-backend`)

Needs a real OpenSSH server (e.g. `sudo systemctl start sshd`, or `docker run -p 2222:2222 lscr.io/linuxserver/openssh-server`).

1. Pane type selector ▸ SFTP (or File ▸ Connect to Server…): the pane shows the connect form; enter host/port/user/password, Connect. A "Trust this host?" dialog shows the fingerprint (compare with `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`); Trust, and the same pane lists the remote folder, with a badge in its header and free space in the status bar.
2. Reconnect: the second connect does not prompt. Wrong password shows an error and keeps the form; the password is never written to disk (`grep -r` the config dir).
3. Copy/move between a local and the remote pane (F5/F6, drag & drop); cancel a large transfer and confirm no `.fmpart` file remains. Delete/rename/mkdir on the remote.
4. Stop sshd: within ~5 s the pane shows the offline banner, keeps its listing, mutating commands are disabled; restart sshd and click Reconnect.
5. Change the server host key: the connect refuses with the "identity changed" card (remove the entry from `known_hosts.json` by hand to trust the new key).
6. Save a profile ("Save as"), restart the app: it appears in the Profile picker (no password stored). `known_hosts.json` and `prefs.toml` are `0600`.
7. Disconnect (button left of the pane-type dropdown) returns the pane to the home folder. Preferences ▸ SFTP sets poll/timeout/keepalive; Preferences ▸ Trusted hosts forgets a host key (next connect asks again). Typing `sftp://user@host/path` in the location bar opens the connect form. A `.ppk` key shows an "Export OpenSSH key" hint.
8. Also worth a look: View ▸ Zoom In/Out/Reset (Ctrl+Shift +/-/0, remembered), Left on a tree root collapses all, `exit`/Ctrl-D closes a terminal pane, narrow Preview toolbar folds into a hamburger, text preview copies without line numbers, Trash pane: expand/open trashed folders and restore inner items.

### Current folder row (change `current-folder-row`)

1. Select folders in the tree: each time the listing opens, its cursor is on the `.` row (nothing selected) and a Preview pane following that Files pane shows the folder itself (summary card, or the thumbnail grid with thumbnails on).
2. Arrow Up from the first entry walks `..` then `.`; Home lands on `.`; Enter/double-click on `.` does nothing. Dragging from `.` starts no drag and `.` is not a drop target.
3. In an empty folder the cursor starts on `.`; Up reaches `..` and stays there across frames. `nav.refresh` leaves the cursor wherever it was.

## Config files

Preferences live in the platform config directory for the brand slug
(`fm_brand::APP_SLUG`), file `prefs.toml`:

- Linux: `~/.config/<slug>/prefs.toml`
- macOS: `~/Library/Application Support/<slug>/prefs.toml`
- Windows: `%APPDATA%\<slug>\prefs.toml`

The file is created `0600` (directory `0700`) on Unix and contains **only**
non-secret settings (theme, window state, last directory).

Regenerable data lives in the platform **cache** directory instead
(`<cache>/thumbs`, trimmed to 256 MiB on use, files `0600` / directory `0700`
on Unix): deleting it is always safe — thumbnails are rebuilt on demand.

### Workspaces, preview and panes (recent changes)

1. `Ctrl+T` adds a tab, `Ctrl+1..0` switches, `Ctrl+W` closes; a non-auto-save tab with changes prompts Save / Discard / Cancel on close and quit. Restart restores the saved workspaces flagged Open at startup.
Works as expected

2. A terminal pane restored at startup shows a live shell, not a black screen.
Works as expected

3. Folder preview: Total size is human-readable, a recursive scan keeps running in the background with a 1 s refresh and stops when you pick another folder; the list shows subfolders with recursive sizes.
Works as expected

4. Resize the window and drag split handles (also in a 2-over-1 layout): panes fill their slots and each divider moves only its own split. The preview scrolls when short.
Works as expected

5. Thumbnail grid: click / Ctrl+click / Shift+click / double-click / Enter / Ctrl+A / drag behave like the Files pane. `F5`, `F6`, `F2`, `F7`, `F8`, `Delete` act on the selection, and the grid updates after the action.
Seems to work, but Ctrl-c ctlv-v, ctrl-x to copy, paste, move a file does not, that should be added.
Double click a folder in the thumbnail view should open that folder (also affects the connected Files panel)
Files panel: Single click on a selected file selects the file, another single click on that file should deselect that file and select the current directory (.)

6. Right-click a row or thumbnail: the same context menu in both views; entries are greyed when not applicable.
Works as expected

7. Every pane header has a `×` (hidden on the last pane).
8. `Ctrl+C` / `Ctrl+X` / `Ctrl+V` copy, cut and paste files (Files pane or thumbnail grid) into the focused folder; Edit menu and context menu show the same entries. Double-clicking a folder thumbnail navigates the connected Files pane. Clicking the only selected file again selects `.`.
Works as expected

## Dev log

Append to `log/devlog.md` at the end of every working session (what landed, numbers, what is still open). It is gitignored and exists so the next session can pick up cold.
