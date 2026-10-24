# Tasks — thumbnails-and-open

## 1. Thumbnail cache in `fm-preview`

- [x] 1.1 `thumbs.rs`: `ThumbKind`/`classify(entry)`, `EDGES` presets, `make(vfs, entry, edge)` for images (img decode + `fast_image_resize` to a square-ish RGBA) and video posters (existing ffmpeg poster path), name tiles for everything else
- [x] 1.2 Versioned owner-only disk cache (`<cache>/thumbs`, `0600`/`0700`, temp + atomic rename, 256 MiB trim on first use, key = uri+mtime+size+edge); tests for permissions, truncation/foreign version and size cap
- [x] 1.3 Local-only policy in `classify` (non-`file://` never thumbnails); hostile-input caps (file size, dimensions) reused from `Limits`; tests

## 2. Engine service and commands

- [x] 2.1 `Session.thumbs: ThumbService` — dedicated pool (≤ 4 workers), per-pane generation + pending window, byte-capped in-memory LRU (192 MiB), `get(entry, edge) -> ThumbState`, cancellation on hide/close
- [x] 2.2 `thumb_cmds.rs`: `preview.thumbs.toggle {on?, pane?}`, `preview.thumbs.size {size?, pane?}`, `preview.thumbs.visible {first, count, pane?}`, `preview.thumbs.clear_cache`; register in `commands.rs`; `menu.rs` ids + View menu entries; shortcut/brand-check tests
- [x] 2.3 Window queueing (visible first, one screenful after, one before; supersede older generation) with integration tests in `engine/tests/preview.rs` (only the window is decoded, sizes re-request, negative/oversized params → `Err`)
- [x] 2.4 `prefs::default_cache_dir`; app bootstrap points the service at `<cache>/thumbs`

## 3. UI

- [x] 3.1 `fm-ui::preview` grid: `show_rows` tiles at the preset edge, hover selection, name tile fallback, `…` while pending, texture cache keyed by thumb pointer, visible-window dispatch once per change, truncation notice
- [x] 3.2 Grid keyboard: arrows/Page/Home/End, focus lock (`grid_has_keyboard`) so the listing's keys and the grid never fight; minimal scroll; click focuses a tile
- [x] 3.3 Header toolbar: thumbnails toggle + size cycle for directory bodies; theme tokens only (dark + light)
- [x] 3.4 Tests: headless render of a grid with thumbnails on (waits for `ThumbState::Ready`), keyboard focus test; listing paging outcome test (`PageUp`/`PageDown` → `Page`)
- [x] 3.5 Snapshots: confirm-run dialog (dark + light) — the grid's own snapshot is covered by the headless render test (textures are non-deterministic)

## 4. Activation

- [x] 4.1 `open_cmds.rs`: `OpenKind`/`classify` (dirs, documents, programs; macOS `.app`; Unix bit+runnable extension; Windows extension list), `plan` from the cached listing
- [x] 4.2 `file.open {uri, mode?, confirm?}`: param validation, directory → `nav.open`, special files refused, run requires `confirm`, default-open of a program requires `confirm`, local-only, spawn on the job lane (detached process group, null stdio); tests for every refusal and the classification table
- [x] 4.3 `fm-ui::dialogs::Dialog::ConfirmRun` — *Run* / *Open as document* / *Cancel*, dispatching `file.open` with the chosen mode and `confirm: true`
- [x] 4.4 App activation path: double-click/Enter → `plan` → navigate / open / dialog (no direct backend calls)

## 5. Listing paging

- [x] 5.1 `KeyOutcome::Page(±1, shift)`, rows-per-screen from the pane height, move/extend through the existing helpers, same-frame scroll-to-row
- [x] 5.2 Tests: `key_outcome` mapping, move semantics with a page delta

## 6. Docs and gate

- [x] 6.1 `docs/architecture.md`: thumbnail service (pool, caches, keys, local-only), activation flow, dependency table (`opener`, `fast_image_resize`, `fm-preview` dev opt-level)
- [x] 6.2 `docs/development.md` smoke checklist: thumbnails (toggle/size/scroll/clear), document open, program confirm dialog, paging
- [x] 6.3 Dependency audit recorded (design D7): `opener` and `fast_image_resize` build on Linux/Windows/macOS without system libs
- [x] 6.4 `cargo fmt --all && cargo xtask ci` green; `log/devlog.md` entry; roadmap current focus updated
