## Why

Browsing a folder of photos, screenshots or downloads is guesswork in a plain listing: names and sizes say little. And the app can show a file's *contents* but does nothing when the user wants to *use* it — double-clicking a document currently goes nowhere. Filling those two gaps makes the Preview pane genuinely useful for folders and turns file activation into a first-class, safe command.

## What Changes

- **Thumbnail grid** for directories in a Preview pane (`preview.thumbs.*`): image and video-poster thumbnails in a sized grid (Small / Medium / Large), windowed — only the visible tiles (plus a screenful either side) are requested; keyboard navigation inside the grid; a **Clear thumbnail cache** command. Work runs on a dedicated worker pool, never on the file-job lanes or the UI thread; results live in a byte-capped in-memory LRU plus an owner-only on-disk PNG cache (versioned keys by uri + mtime + size + edge). Only local files are thumbnailed; remote folders keep the summary card.
- **File activation** (`file.open {uri, mode?, confirm?}`): activating a **directory** navigates; a **document** is handed to the system default application; an **executable/program** asks first — *Run*, *Open as document*, or *Cancel* — and only runs with explicit confirmation; special files are refused. All local-only (remote files return an actionable error), no shell, child detached in its own process group.
- **Paging keys** in the listing: PageUp/PageDown move a screenful (Shift extends the selection).
- New dependencies: `opener` (OS "open with" hand-off) and `fast_image_resize` (thumbnail scaling); `fm-preview` is built with `opt-level = 3` even in dev profiles because resize/decode gets inlined into it.

## Capabilities

### New Capabilities

- `preview-thumbnails`: the thumbnail grid in a directory Preview, its cache lifecycle, scheduling and local-only policy.

### Modified Capabilities

- `file-op-commands`: file activation (`file.open`) becomes a command with modes, confirmation and refusal rules.
- `file-listing`: PageUp/PageDown paging in the keyboard-navigation requirement area.
- `security`: launching programs and writing the thumbnail cache extend the existing rules (explicit confirmation, no shell, owner-only cache files).

## Impact

- **Code:** `fm-preview` (`thumbs.rs`: classify/make/disk cache), `fm-engine` (`thumbs.rs` worker pool + LRU, `thumb_cmds.rs`, `open_cmds.rs`, `Session.thumbs`, `prefs::default_cache_dir`), `fm-ui` (`preview.rs` grid + toolbar, `dialogs.rs` ConfirmRun, `listing.rs` paging), `apps/fm-app` (activation through `plan`, cache dir bootstrap, menu ids).
- **Dependencies:** `opener 0.9` (no default features), `fast_image_resize 6`; both pure Rust on Linux/Windows/macOS.
- **Limits:** memory LRU 192 MiB, disk cache 256 MiB (trimmed on first use), thumbnails only from local `file://` uris, decode via the existing image/video poster paths with their hostile-input caps.
