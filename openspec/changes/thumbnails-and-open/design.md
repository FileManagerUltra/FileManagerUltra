## Context

The Preview pane (change `pane-preview`) renders one entry at a time; directories fall back to a summary card. `Session` owns per-pane runtime state keyed by `PaneId` (terminals, previews). Work must leave the UI thread, and only filesystem/tool crates below L4 may own threads or spawn processes. The engine already classifies files by sniffing and has capped decode paths in `fm-preview`.

## Decisions

**D1 — Thumbnails are an engine service, not a UI concern.** `fm-preview::thumbs` (L1) answers "is this entry thumbnailable / how is a thumbnail made / how is it cached on disk"; `fm-engine::thumbs` owns a `ThumbService` with its own small worker pool (max 4) and an in-memory `HashMap<Key, ThumbState>` + byte budget, plus a pending queue per pane. `fm-ui` only asks `s.thumbs.get(entry, edge)` and paints; requesting is expressed as commands (`preview.thumbs.visible`) so the same behaviour is reachable from the control channel later.

**D2 — Windowed, superseding requests.** The grid renders with `ScrollArea::show_rows`; each frame where the visible range changed it dispatches `preview.thumbs.visible {first, count}`. The engine queues visible entries first, then one screenful after, then one before; a new request increments a per-pane generation and in-flight results of an older generation are dropped, so fast scrolling does not build a backlog. Leaving the grid or the pane cancels outstanding work for that pane.

**D3 — Cache keys and limits.** Key = `(uri, modified, size, edge)`; disk filenames are a hash of that key plus a format version, so an edited file misses and a format change invalidates. Memory LRU cap 192 MiB; disk cap 256 MiB trimmed oldest-first on first use. Files are written through a temp file + atomic rename with `0600`/`0700`, and any unreadable/oversized/foreign-version file is ignored. `preview.thumbs.clear_cache` drops both.

**D4 — Only local files.** Thumbnailing a remote directory would transfer every file; the grid is therefore a local-only mode (`Scheme::File`), and remote folders keep the summary card. This mirrors the preview's "never read more than the selected entry" rule.

**D5 — Activation plan is pure; dispatch is a command.** `open_cmds::plan(session, uri)` classifies from the *cached* listing (no I/O) into `Navigate | Launch | ConfirmRun{name}`; the shell either dispatches `nav.open` / `file.open` or shows the confirm dialog. `file.open` re-stats through the backend, validates params, refuses special files and non-local uris, and spawns on the job lane. Document hand-off uses `opener` (OS shell APIs — never a shell command); running a program on Unix uses `Command::new(absolute_path)` with null stdio and `process_group(0)` and is reaped on a detached thread. Windows uses `opener` for both (ShellExecute already detaches).

**D6 — Paging.** `key_outcome` gained `Page(±1, shift)`; the listing computes rows-per-screen from `available_height / (row_height + item_spacing)` minus one, reusing the existing move/extend machinery so Shift and selection semantics stay identical.

**D7 — Dependency audit.** `opener 0.9` (default features off) is a thin cross-platform wrapper over `xdg-open`/`gio`-style APIs on Linux, `ShellExecuteW` on Windows and `open`/`NSWorkspace` on macOS; no system libraries at build time, no C toolchain. `fast_image_resize 6` is pure Rust with runtime SIMD dispatch (no build-time CPU flags) and is used only in `fm-preview`. `fm-preview` is pinned to `opt-level = 3` in dev profiles because decoding/resizing is generic and gets inlined.

## Risks / Trade-offs

- **Decode cost on huge folders.** Bounded by the visible window + one screenful either side, four workers, and cancellation; a dropped generation is discarded before decoding starts. Memory is capped, so a very large folder cannot exhaust RAM through thumbnails.
- **Thumbnail staleness.** mtime+size is the identity the filesystem gives us cheaply; an edit that preserves both keeps a stale thumbnail until the cache is cleared (the Clear command exists for that). Content hashing was rejected as too expensive for a grid.
- **`opener` on a bare Linux box without a desktop portal** fails with an OS error; the command surfaces it as `FmError::Backend` (a notice), never a crash and never a silent no-op. Running programs directly still works because it does not use `opener`.
- **Programs classified by extension on Windows / bit+extension on Unix.** A text file marked executable opens the confirm dialog (safe default: nothing runs without confirmation). A binary with no runnable extension is opened as a document.
