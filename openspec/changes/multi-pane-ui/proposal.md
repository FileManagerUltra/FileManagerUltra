## Why

The scaffold can open a themed window and dispatch commands, but it has no panes, no working filesystem backend and no way to look at a directory — every later change (drag & drop, ops, remotes) needs panes that browse real directories through the `Vfs` trait first. Change 2 makes the app a *file manager you can navigate*: recursive split panes over local directories, a lazy file tree, a virtualized sortable listing with selection, and commander-style navigation — all data-driven from `fm-layout` state and reachable only through engine commands.

## What Changes

- **`fs-local` is implemented for real**: `read_dir`/`stat`/`open_read`/`open_write`/`create_dir`/`remove`/`rename`/`same_device`/`space` against `std::fs`, with tempdir round-trip tests and correct `FmError` mapping. (`fs-smb`/`fs-sftp` remain stubs — change 4.)
- **`fm-layout` grows the per-pane state model**: `TreeState` (root, expanded set, scroll), `ListingState` (sort, selection, focused row, scroll), `History` (back/forward stacks) — pure data with invariant tests.
- **`fm-engine` gains a `DirCache`** (uri → revision + entries) and a `ReadDir` job kind on the existing lanes; commands enqueue reads, the UI drains events once per frame. Sort runs in the engine over the cached entries, with hard caps on entries per directory.
- **New commands**, all dispatched through `Session::dispatch`: `pane.split.horizontal`, `pane.split.vertical`, `pane.close`, `pane.swap` (Tab), `pane.focus.next`, `nav.open`, `nav.up`, `nav.back`, `nav.forward` (Alt+Left/Right), `nav.refresh`, `view.sort.name|size|mtime|type`, `view.sort.dirs_first` toggle.
- **`fm-ui` widgets**: lazy file tree (expand → one cached read), virtualized listing (fixed row height, `show_rows`, columns name/size/mtime/type, header-click sort, single/multi/shift/ctrl selection, keyboard navigation), pane chrome (location bar, loading/empty/error/stale states), all from theme tokens.
- **`fm-app` renders the pane tree recursively** into egui panels with draggable split handles, a focus model (click focuses; Tab swaps the two most-recently-focused panes), a `Panes` menu, and a status bar showing the focused pane's location, item/selection counts and theme.
- **Security extension**: hostile directory content (100 000+ entries, 4 KiB names, control characters, non-UTF-8 bytes, fifo/device nodes) renders and caches without panic or unbounded allocation — caps and truncation/error policy are specified and tested.
- UI snapshots (`egui_kittest`) for 1-pane, 2-pane split, tree-expanded and selection states, each in **dark and light**.

## Capabilities

### New Capabilities

- `panes`: recursive pane tree — split/close/swap/focus, ratios, at least one pane always present.
- `local-backend`: the local `Vfs` implementation is complete and correct (tempdir round-trips, error mapping).
- `directory-cache`: engine-side cached directory reads over job lanes, revisions, invalidation, bounded size.
- `file-tree`: the per-pane lazy directory tree.
- `file-listing`: the per-pane virtualized, sortable, selectable file listing.
- `navigation`: per-pane navigation — open, up, back/forward, refresh, location bar, shortcuts.

### Modified Capabilities

- `security`: extend the "hostile input" requirement with directory-listing scenarios (huge directories, absurd names, special file kinds) and the caps/truncation policy.

## Impact

- **Code:** `crates/fs-local`, `crates/layout`, `crates/engine` (DirCache + commands), `crates/ui` (tree/listing/pane widgets), `apps/fm-app` (render loop, menus, shortcuts, status bar); `crates/testkit` gains a hostile-tree builder.
- **Dependencies:** `egui_kittest` (dev-only, `fm-ui`/`fm-app` snapshots). No new runtime dependencies — local I/O is `std::fs`.
- **Docs:** `docs/architecture.md` §6/§7/§9 refined if the landed shape differs; `docs/development.md` snapshot instructions.
- **No existing requirement changes** beyond the `security` extension; theming/command-registry/app-shell requirements are already satisfied by new behaviour.
- **Risks:** huge local directories and non-UTF-8 names are the hostile inputs of this change; a blocking read on the UI thread would violate golden rule 4 — every read goes through the job lanes and is covered by a test that the UI poll path never calls `std::fs` directly.
