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
- **Windows**: network shares are reached through native UNC paths (`WNetAddConnection2`), so no libsmbclient build is needed there.
- **macOS**: local roots are `/` and `/Volumes`; SMB uses the same pavao path as Linux.
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

6. Click a sort header to re-sort in place; `Ctrl+H` toggles directories-first.
Works, but size column is overflowed by date column, no clear column distinctions/boundaries.


7. `Backspace` goes up, `Alt+Left`/`Alt+Right` walk history, `Ctrl+R` refreshes; double-click or `Enter` opens a directory.
Works

8. Help → About opens a dialog with name, version and organisation; closing it does not quit.
Works, but it would be nice to have the dialog centered in the view window

9. Quit; `prefs.toml` appears in the config directory (see below) with mode `0600`.
Correct


## Config files

Preferences live in the platform config directory for the brand slug
(`fm_brand::APP_SLUG`), file `prefs.toml`:

- Linux: `~/.config/<slug>/prefs.toml`
- macOS: `~/Library/Application Support/<slug>/prefs.toml`
- Windows: `%APPDATA%\<slug>\prefs.toml`

The file is created `0600` (directory `0700`) on Unix and contains **only**
non-secret settings (theme, window state, last directory).

## Dev log

Append to `log/devlog.md` at the end of every working session (what landed, numbers, what is still open). It is gitignored and exists so the next session can pick up cold.
