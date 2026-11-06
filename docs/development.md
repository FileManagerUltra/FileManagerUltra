# Development

## Prerequisites

- Rust stable via [rustup](https://rustup.rs) (`cargo 1.99+`). The workspace uses edition 2024.
- No system `-dev` packages are required to **build** (verified: `eframe 0.37` + winit/wgpu compile on stock Ubuntu 24.04 with only `gcc`, `make` present).
- A display is required to **run** the app (Wayland or X11). CI and headless work use the test/snapshot paths that never open a window.
- Linux runtime packages for the window itself: `libxkbcommon0`, `libwayland-client0`, `libGL.so.1`, `libEGL.so.1` — present on a normal desktop install; a bare server may need them (`sudo apt install libxkbcommon0 libgl1 libegl1`).

## Everyday commands

```sh
cargo run -p fm-app                 # run the desktop app
cargo xtask desktop-install         # Linux: register this build's launcher and dock icon
cargo test  --workspace              # the suite
cargo test  -p fm-ops                # one crate
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask layers                   # layering enforcement (fails on a bad dependency)
cargo xtask brand-check              # no hard-coded product name outside brand/
cargo xtask ci                       # everything CI runs, locally
```

Faster inner loop: `cargo nextest run` if `cargo install cargo-nextest` is available; while iterating use `cargo test -p <crate> --lib --tests` to skip examples/doctests.

For dependency/dead-code cleanup, check source, tests and platform-specific
configuration before removing a dependency. Some dependencies intentionally
enable transitive features (for example `winit`'s Wayland title bar support).
Run Clippy with `--workspace --all-targets --all-features` to cover optional test
helpers too; public APIs still need a separate caller search because Rust's
dead-code lint does not flag exported, unused APIs.

On memory-constrained desktops, limit compilation and test concurrency:

```sh
CARGO_BUILD_JOBS=1 cargo build -p fm-app
CARGO_BUILD_JOBS=1 cargo test -p fm-engine -- --test-threads=1
```

Avoid simultaneous builds. On Linux, programs launched from the embedded terminal
share the application's systemd scope. Memory pressure from a Cargo build can
therefore cause the desktop's OOM manager to kill the whole scope and attribute
the notification to the file manager, not just the compiler.

Development builds keep line-table debug information (`debug = 1`) rather than
full variable/type information to reduce build-artifact size. Use
`CARGO_PROFILE_DEV_DEBUG=2` when full debugger inspection is needed.
Cargo output lives under `target/`; `cargo clean` removes it, including the
binary used by the development desktop launcher, so rebuild before launching
again. Remove unused agent-specific target directories after their work ends.
Keep `crates/ui/tests/snapshots/`: these are test baselines, not disposable
screenshots.

Use your own target dir when working in parallel with another agent:

```sh
CARGO_TARGET_DIR=target/agent-<name> cargo test -p fm-engine
```

## Layout of the tree

See `AGENTS.md` §2. `docs/architecture.md` explains why each crate exists and how data flows.

## Platform notes

- **Application artwork**: the yellow/blue split-folder masters live in
  `crates/brand/assets/logo.svg` (flat Linux), `logo-windows.svg` (shaded), and
  `logo-macos.svg` (rounded tile). After editing them, run
  `python3 crates/brand/assets/generate-icons.py` with Inkscape and ImageMagick
  (`magick`) installed. This regenerates all three 256 px runtime/About PNGs,
  packaging PNG/SVG, the six-size Windows ICO and standard/Retina macOS ICNS.
  These tools are only required for artwork changes, not normal builds.
  `cargo test -p fm-brand` checks asset dimensions and container representations.

- **Linux development icon**: `cargo run` sets the window icon but does not install
  an icon-theme entry or desktop launcher. GNOME/Wayland docks need those to show
  the application icon. Run `cargo xtask desktop-install` once, then restart the
  app and re-pin any old generic dock shortcut. It writes
  `$XDG_DATA_HOME/applications/<slug>.desktop` and
  `$XDG_DATA_HOME/icons/hicolor/256x256/apps/<slug>.png` (default data directory:
  `~/.local/share`). The launcher points at the binary alongside the running
  xtask, normally `target/debug/<executable>`; re-run after moving the checkout
  or switching the Cargo target/profile. No root privileges are needed. To undo,
  remove those two files. An installed package supplies its own desktop entry.

- **Windows upgrade icons**: the executable and NSIS installer both embed the
  branded ICO. Interactive installers notify Explorer to refresh its icon cache
  when the installer closes. Windows can still retain an old icon in a pinned
  shortcut: unpin the old entry, start the installed app from Start, and pin it
  again. Check a fresh desktop shortcut too. The installer does not restart
  Explorer or delete user shortcuts/cache files. Verify upgrades on Windows;
  config tests on Linux cannot confirm Explorer's cache behavior.

- **Dragging files out** to other apps (`fm-dragout`, XDND source) works on X11 targets (XDND) and on native Wayland (`fm-wayland`, `wl_data_source`). Wayland sessions run natively by default; `FM_X11=1` forces XWayland. macOS/Windows: not implemented.
- **Wayland**: file drops from other applications are received by `fm-wayland` (winit 0.30 cannot); in-app pane-to-pane drag & drop is unaffected, and pasting paths from the clipboard always works. See `docs/architecture.md` §11.

The main window saves its normal size, position and maximized state about a second
after they stop changing (and on exit) and restores them on launch, re-asserting the
position for the first moments in case the window manager places the window itself. Positions on disconnected monitors are not restored.
Native Wayland controls placement in the compositor, so only size/maximized state
can be restored there; X11/XWayland, Windows and macOS also restore position.
- **Network discovery** (manual smoke; multicast cannot run in CI): File ▸ Network should list Samba/NAS hosts (mDNS) and Windows PCs (WS-Discovery) within ~5 s; opening one asks for credentials in a popup, a wrong password reopens it with an error, and `Remember until you quit` reconnects silently. VLANs/VPNs may hide hosts; use the Network pane's Connect… button (or File ▸ Connect to SMB Server…) as the fallback.
- **Manual SMB host**: Host accepts `nas`, an IP address, or a UNC server name
  such as `\\192.168.0.50` (an optional final backslash is allowed). Enter the
  share separately in Share; a complete `\\server\share` path is not a host name.
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

In the file tree, clicking a folder name opens it and expands it; repeated or
double clicks on the name never collapse it. Click the disclosure arrow to
expand/collapse without changing the listing. Right/Left keyboard behavior is
unchanged.

The windowed launch is `#[ignore]`d in CI. Run it by hand:

```sh
cargo test -p fm-app -- --ignored window_launches_and_closes --nocapture
cargo run -p fm-app   # or the installed binary name from fm_brand::EXECUTABLE
```

Manual checklist:

Preferences: select General, Appearance, Files, Copy and move, Preview, Terminal, Connections,
Workspaces, Keyboard shortcuts and Advanced in the left topic column; only that topic's controls appear on the right.
Keyboard shortcuts is a grouped read-only reference, searchable by action, key or
context; check pane-local keys, aliases and focus-dependent meanings without
changing settings or bindings.
With Preview focused, Ctrl+0 (Cmd+0 on macOS) resets its zoom rather than
switching to workspace 10; Ctrl+1..9 still switch tabs. In Files, Ctrl+0
continues to select workspace 10. Dialogs, text fields and terminals keep their
keyboard ownership.
Check proportional typography, left-aligned navigation and flat, divider-separated sections
with aligned controls; the file listing keeps its existing typography. Connections separates SMB
security, remote timing and a bordered trusted-host list with aligned Forget buttons.
Theme and shell choices use radio controls. Changes apply immediately; Close/Escape and
reopening retain the topic. Shrink the window to check independent scrolling and that Close
remains accessible; verify both themes.

All dialogs follow [the dialog style guide](style-guide.md). Drag title bars and
check that the position stays put on subsequent frames. Check the title-bar close
button in file-operation confirmations, unsaved-workspace prompts, remote login /
host-key prompts, the key-file picker, Properties, Find Files, Open With, log time
range, Jobs and About. Closing must cancel/dismiss, never approve. The key picker
closes independently; hiding Jobs must not cancel work and its indicator reopens it.

Multi-file log preview: select several logs, open Filter, and use Refresh (in place of
Live tail) after appending to the files. It reloads all selected sources even before a
change notification; filters, ordering and expanded-record settings remain applied.
Single-file logs still offer Live tail.

Preferences > Preview also sets Live tail's polling interval (100..60000 ms, default
500) and initial log tail (1..64 MiB, default 8). Unlimited initial tail removes that
window preference, not safety limits: at most 64 MiB for a single log or 32 MiB shared
across selected files, with existing parser/time limits. Larger files still start at
the tail. Interval changes reschedule active polling; initial size applies on next open
or reload. Hover Live tail for delta-transfer and marker behavior.

File-column resizing: Size, Modified and Type start fitted to content; Name takes spare
space. Drag a header divider back and forth, including beyond a width limit: it should
track the pointer and redistribute neighboring space. Double-click it to fit the column
to its right. Shrink and expand a pane: details compress smoothly, text never overlaps,
and preferred widths return. Save/reload a workspace to verify manual widths and fit
choices persist. Check both themes and application zoom.

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
2. Terminal: shell prompt in the pane's directory; `top`/`vim` redraw; Backspace, Ctrl+C/D/L work; Ctrl+Shift+C/V copy/paste; drag a file from a Files pane (or the OS) onto it to paste its quoted path, and the terminal regains focus. Select output, scroll with the wheel or Shift+PageUp/PageDown, then Shift-click to extend: selection stays anchored to the original text, and copy includes off-screen rows without moving the view. Drag near the top/bottom edge to extend selection with automatic scrolling. Resizing or clearing/evicting history may clear selection. Print long lines mixed with explicit newlines, colours and wide characters, then narrow/widen the pane: normal output and scrollback reflow, hard line breaks remain, and typing continues at the right position. Repeat while scrolled up; the same logical line should remain at the top unless history was evicted (10 000 visual-row cap). Resize `vim`/`top`: alternate-screen content is not reflowed and the app redraws; exit and check the reflowed shell output is restored. In mouse-enabled applications such as `mc` and `btop`, click menus, drag, and scroll the wheel; hold Shift to select text or scroll terminal history instead. After exiting the application, ordinary selection and scrollback work again.
3. `Ctrl+Shift+E` split right, `Ctrl+Shift+O` split down (new pane next to the active one); `Ctrl+Shift+Arrows` resize the nearest divider, `Ctrl+Shift+B` equalize, `Ctrl+Shift+R` rotate, `Ctrl+Shift+X` zoom toggle. All also work with a terminal focused.
5. Preview: `Ctrl+Shift+P` opens a preview to the right of the active pane and toggles it closed. Arrow through text, JSON (Pretty button), Markdown, an image, a binary (hex) and a folder; the preview follows the cursor, or pin it with the source picker. A `*.log` switches to Log mode: level/search/time filters, Live tail (`Ctrl+Shift+F`) while another process appends, Mark (`Ctrl+Shift+M`) to drop a divider rule at the current end, folded stack traces, access-log columns. Time offers quick ranges starting the selected interval before Apply time with To left open for new Live tail rows, or custom `YYYY-MM-DD` dates (also calendars) and `HH:MM[:SS]` times in UTC. Check open-ended bounds, invalid/reversed input, Cancel without changes, and Clear keeping other filters; only loaded records are filtered and From does not slide automatically. Video shows metadata and a poster frame when ffmpeg is installed (no playback yet).
4. Drag a pane by its outer border (the edge highlights) onto another: the outer quarter of an edge inserts there, the middle swaps; Esc cancels. Terminals keep running through moves. Switch focus repeatedly between panes: only the accent outlines change, never header/body size or content positions, in either theme.
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

6. Right-click a row or thumbnail: the same item context menu in both views; entries are greyed when not applicable. **Open in Preview Panel** shows the pointed item in an existing Preview, or creates one to the right when none exists; repeated actions must never close it or add extra panes. Test another previously unfocused Files pane, multiple selected files, a Preview pinned to another Files pane, and a zoomed Files pane. A thumbnail action selects the item in its connected Files pane, navigating to its parent if needed. Ctrl+Shift+P remains a toggle. Right-click blank Files-listing space or `.` for current-folder actions (New Folder, New File, Paste, Refresh, Copy Path/Name, Add Bookmark, Properties). The same folder menu on a tree folder targets that folder without navigating into it; saved-server entries retain their Connect/Edit/Delete menu.
Works as expected

7. Every pane header has a `×` (hidden on the last pane).
8. `Ctrl+C` / `Ctrl+X` / `Ctrl+V` copy, cut and paste files (Files pane or thumbnail grid) into the focused folder; Edit menu and context menu show the same entries. Double-clicking a folder thumbnail navigates the connected Files pane. Clicking the only selected file again selects `.`.
Works as expected

### Preview file-filter checks

In Preferences > Preview, scroll to File type filters. In both themes, press Show on Logs, add `*.trace` (Enter or Add) and confirm a selected `.trace` file changes to log preview immediately. Edit a row and leave the field to save; Remove rows to drop name associations (unmatched files still use content detection). Add `*.trace` to CSV too and confirm Logs wins; press Up on CSV and confirm CSV now wins. Try `../*.log` or `a;b` and confirm an inline error preserves the previous setting. Restore default file filters, verify every field resets, then restart to check persistence. Check custom image extensions in both individual previews and directory thumbnails. HTML/CSV remain text previews.

Focus a long `.txt`/source or `.md` preview, press Ctrl+A then Ctrl+C (Cmd on macOS)
and paste elsewhere: all loaded text, including offscreen rows, should copy without
line numbers. Markdown copies rendered text/code, not source formatting; structured
text copies the current raw/pretty view. Check selected colours in both themes,
Escape/click to resume normal selection, file changes invalidating selection, and
Preferences/text fields retaining their own select-all/copy shortcuts. Truncated
previews copy only their loaded portion.

Markdown document checks: preview `crates/ui/tests/fixtures/markdown.md` and
`markdown-table.md` in dark/light themes. Check real bold and combined emphasis,
strikethrough, preserved hard breaks, nested ordered/bullet lists and read-only
tasks, multi-paragraph/nested quotes, fenced code highlighting and language labels.
Tables must have aligned top edges/shared row heights and continuous borders,
including short symptoms alongside long explanations with inline code. Narrow the
pane: columns retain readable widths and scroll horizontally, never justify letters
or stagger rows. Select/copy includes tasks, nested blocks and tab-separated cells.
Links, raw HTML and image descriptions remain inert, with no external loads.

Preview-only zoom: use its `- / percentage / +` controls or focus it and press
Ctrl+= / Ctrl+- / Ctrl+0 (Cmd on macOS; Shift zoom chords also target a focused
Preview). Verify text/Markdown, raw/pretty, logs, image Fit/Actual, hex, metadata,
folder summaries/thumbnails, mixed selections and video. Content scales between
50% and 400%; the toolbar, other panes and application zoom stay unchanged.
Open Preferences or focus a text field and verify its keyboard ownership.

Markdown fonts are unmodified Atkinson Hyperlegible regular/bold from
<https://github.com/google/fonts/tree/main/ofl/atkinsonhyperlegible>, distributed
with their OFL copyright/license. SHA-256: regular
`7fb917c89019896d0b52ee84b7cbb3304c18cb90b19a62f5e32712bd23e97669`,
bold `5a3b0c8cc8ca545155150b4512a1fa248298df121c50d6557e651e61fbdab92f`.

Preferences > Terminal > Scrollback rows accepts 5,000-1,000,000 visual rows
(default 10,000). Change it with a terminal running, then open/restart another:
only new/restarted sessions adopt the new cap. Restart the app to check persistence.
The terminal resize smoke check above uses the configured cap (10,000 by default);
wrapped lines count as multiple rows and large histories consume substantial memory.

Preferences > Appearance / Files / Terminal / Advanced (all through `prefs.set`): pick an accent preset and a custom `#rrggbb` (invalid text is ignored; Default restores the theme colour); switch density and change text size up and down repeatedly (sizes must track the value, not shrink cumulatively). In Files, toggle show-hidden (the same setting as View > Show Hidden Files (remember), which saves it; Ctrl+H and the status-bar button change only the current session), single-click open, size units and date formats (listing, Properties and Search all follow), and turn off the delete/run prompts to confirm they proceed directly. Terminal font size applies to open terminals. In Advanced > Thumbnails check the "Stored in" path, set a parent folder (relative paths and unwritable folders are rejected with the old one kept), press Clear thumbnail cache, and lower the caps; the preview time limit applies to the next render.

### File timestamp timezone checks

File times follow the OS timezone detected at startup, including historical DST
rules; no separate application timezone preference is needed. Restart after
changing the OS timezone. The standard `TZ` environment variable can override
detection when launching the app (for example `TZ=Europe/Amsterdam cargo run -p fm-app`).
Check local and remote listings, Properties and both sides of a copy conflict:
the same instant must show the same local time everywhere and all four date
formats must still work. Compare winter and summer dates and a time near midnight;
sorting must remain chronological. Log source text and UTC log time filters are
unchanged. If detection fails, a notice and stderr diagnostic explain the UTC
fallback.

### Preview Find checks

Focus a text, structured or Markdown Preview and press Ctrl+F (or use Find in
the toolbar/Preview menu). Check live match counts, Aa (case), Word and `.*`
(regex), invalid patterns, no matches and clearing the query. Enter/F3 advances;
Shift+Enter/Shift+F3 goes back, wrapping at either end. Confirm off-screen matches
are revealed in both axes, with Wrap off/on and preview zoom changed. Markdown
headings, formatted paragraphs, nested lists/quotes, code and table cells should
highlight; code languages, line numbers and controls should not. Ctrl+A/Ctrl+C
in the field affect only the query; copying document text must not include search
chrome. Escape/Close hides Find. Switch files/raw-pretty modes while searching
and check that stale highlights disappear and counts update.

Find and zoom controls should share the source/title toolbar row, with the
filename truncated and controls in View options when the pane is narrow.
Logs should show Filter and zoom controls, never a duplicate Find button.

In logs, Ctrl+F opens the existing filter panel and focuses record search;
Escape hides it without clearing filters. Images, videos, binary/folder previews
and multi-file card grids show Find disabled. Files Ctrl+F must still filter the
listing, terminal Ctrl+F must reach the running application, and open dialogs
must retain keyboard ownership. Search is loaded-content-only, with at most
10,000 retained matches (`+` indicates more).

### Markdown code-copy checks

Markdown code blocks have a top-right copy icon: 50% opacity at rest, full opacity
on hover/focus. Check fenced/indented code, nested lists/quotes and multi-file
cards at narrow widths and different preview zoom levels. Copy must preserve
the loaded code exactly, omitting language labels and fences. Drag selection and
Ctrl+A/Ctrl+C must never include the copy control or its tooltip.

### Devices and mounted-volume checks

- With no eligible mounted volumes, there is no Devices heading in a Files pane
  or Go menu. Mount a USB stick: within 5 s while focused, its label, path and
  free/total appear. Background polling is 10 s; regaining focus refreshes now.
- Open the row, navigate into subfolders, and copy/drop files onto it. Collapse
  Devices, save a workspace and reopen it: the collapse state survives.
- Mount VeraCrypt externally, including a container on an internal disk. Its
  volume appears even when non-removable; the locked container does not.
  Unsupported Dismount explains that VeraCrypt/OS tools must be used. The app
  never asks for a password or powers off the container's host disk.
- Start a transfer from or to the device and try removal: it must fail before
  moving panes Home. After finishing, Eject/Dismount must move affected panes
  (including parked workspaces) Home and confirm the volume disappears.
- Unplug while browsing: the row disappears and the pane shows unavailable, not
  an empty mount directory. A new copy to that old location is refused until the
  volume reappears. Already-running OS I/O may fail normally.
- Linux requires `udisksctl` for block-device removal (`umount` for network mounts).
  Other mounted partitions prevent whole-disk power-off. macOS currently offers
  volume-only Dismount for identified physical volumes. Windows offers Eject for
  identified removable drive letters; fixed/virtual mappings remain browse-only.
  Test real media and VeraCrypt on each OS before considering the change archived.
- Test both themes, a long/hostile label, a busy mount, a missing tool and a network
  mount with unavailable capacity. Errors must be visible without blocking frames.

## Live directory refresh checks

Run on Linux, macOS and Windows (manual smoke; the OS watchers differ per platform).

- Open a Files pane on a local folder, then create, rename and delete files from
  another program (terminal, Explorer/Finder/file manager). The listing updates
  within ~1 s without Ctrl+R; the selected file stays selected and scroll holds.
- Show the same folder in two panes and in two workspace tabs; change it, then
  navigate one pane away: the other keeps updating. Switch tabs: hidden tabs
  must not keep refreshing (check CPU stays near idle).
- Delete the folder a pane shows (or unplug its volume): the pane shows an error,
  not a stale listing, and recovers when the folder reappears and is retried.
- Run a burst in a shown folder (`for i in $(seq 5000); do touch f$i; done`, or a
  build/`git checkout`): the UI stays responsive and settles on the final list.
  Watch memory in the task manager before and after; it must not grow.
- Windows: repeat on a mapped network drive and a UNC path (`\\server\share`).
  These poll every 2 s instead of using events; changes still appear, with delay.
  Also check a path with a trailing/mixed-case drive letter and a `\\?\` path.
- macOS: repeat on a folder under `/Volumes` and on an iCloud/Dropbox folder.
- Linux: lower `fs.inotify.max_user_watches` (for example `sudo sysctl
  fs.inotify.max_user_watches=8`), open several panes: a notice says some folders
  refresh periodically, and listings still update. Restore the value afterwards.
  Also try an NFS/sshfs mount (must poll, not error).
- Remote (SFTP/SMB) panes are unchanged: refresh on focus regain and every 5 s.

## Git status and diff checks

1. In a git repo, open it in a pane: modified files show a tinted name and an `M` in the icon gutter, untracked `U`, folders a dot; the footer shows `git: <branch> · N changed`.
2. Edit/save a file externally: the marker appears within about a second.
3. Open a Preview pane and select a modified file: it opens in the Diff view (aligned columns, word highlights, "Change N of M"). F7 / Shift+F7 (or the ▲▼ buttons) put the next/previous change's first line at the top; with the pane focused away from the diff, F7 is still New Folder. Narrow the pane: Unified/Ignore EOL/Reload fold into the ☰ menu and the columns shrink with it.
4. Click the revision button (`HEAD ▾`): a resizable picker lists Recent, Local, Remote and Tags with search. Pick a branch, then select another file: it stays in Diff against that branch. Click Preview: following files stay in Preview. Folders and multi-selections show no Diff button.
5. Preferences > Files > Git toggles the markers. Opening a huge repo must keep the UI responsive.
6. Tests: `cargo test -p fm-git -p fm-engine --test git_status --test diff_pane`; snapshots `preview_diff_*` and `listing_git_markers_*` in `crates/ui/tests/snapshots.rs` (`UPDATE_SNAPSHOTS=1` to refresh). The fixtures call the real `git` binary; the product never does.
7. Windows/CRLF is untested on real hardware: check that a `core.autocrlf=true` checkout shows no markers.

## Dev log

Append to `log/devlog.md` at the end of every working session (what landed, numbers, what is still open). It is gitignored and exists so the next session can pick up cold.
