## Why

A file manager is mostly about deciding what a file is and what to do with it. Today the only way to look inside is to leave the app. A Preview pane (a new pane kind next to Files and Terminal) shows the selected file as you move the cursor, so one pane can browse while another reads.

## What Changes

- **New pane kind `Preview`** in the type switcher. It shows a preview of the file selected in a *source* Files pane and updates as the selection changes.
- **Source pane**: by default the most recently focused Files pane; a header picker lets the user pin a specific Files pane or return to "follow focus". A pinned pane that closes falls back to follow-focus.
- **Preview renderers** (picked by sniffing content + extension, never by extension alone):
  - text / source code with syntax highlighting, line numbers, truncation notice;
  - JSON, YAML, TOML, XML shown as the raw highlighted file by default, with a **Pretty** toggle button in the preview header that re-indents them (the button is disabled with a reason when the file is invalid);
  - Markdown rendered (headings, emphasis, lists, code, links shown but not followed);
  - images: png, jpeg, gif (animated), webp, bmp, ico, svg, fit-to-pane with zoom;
  - audio and PDF: metadata card (tags, duration, page count, title);
  - video: poster frame, metadata, and playback with play/pause/seek;
  - **log files** (`*.log`, `*.log.N`, `*.gz` rotations, `var/log`, `var/report`): the preview switches to a **Log mode** with level colouring/filter, text/regex search, time-range filter, collapsible stack traces, a column view for access logs, and **Follow** (live tail). Formats: nginx access/error, Apache access (common/combined)/error, Magento `system|debug|exception.log` and `var/report/*` JSON, Pimcore/Monolog lines, PHP-FPM/PHP errors, generic plain. Large files are read tail-first in a bounded window with "Load earlier";
  - directories: summary (entry counts, total size of direct children, newest file);
  - anything else: hex dump of the first bytes plus a metadata card.
- **Commands**: `pane.preview.source {pane|"focus"}`, `preview.zoom {fit|actual|in|out}`, `preview.toggle_wrap`, `preview.toggle_pretty`, `preview.video.toggle`, `preview.log.{filter_level|search|time_range|follow|columns|fold}`; Panes ▸ Preview entries and a `View ▸ Preview` toggle that opens a preview pane to the right of the focused pane (like split right).

## Capabilities

### New Capabilities

- `preview-pane`: source selection, content sniffing, renderers, size/decode limits, off-thread rendering, video playback.

### Modified Capabilities

- `pane-types`: `Preview` becomes a selectable, supported kind (ADDED requirement).
- `security`: preview parsing limits and isolation (ADDED requirement).

## Impact

- **Code:** new crate `fm-preview` (L1: sniffing + decoding into a plain `Preview` model, no egui), `crates/engine` (`preview_cmds.rs`, `Session.previews`, source resolution, job lane work), `crates/layout` (`PaneKind::Preview`, `PaneState.preview_source`), `crates/ui` (`preview.rs` renderer, theme tokens for syntax colours in dark and light), `apps/fm-app` (menu, shortcuts, repaint while a video plays).
- **Dependencies (candidates, all pure Rust unless noted):** `syntect` (fancy-regex, no onig), `image`, `resvg`/`usvg`, `pulldown-cmark`, `serde_yaml_ng`/`toml`/`quick-xml`, `lofty` (audio tags), `lopdf` (PDF info). Video: see design D8.
- **Risks:** hostile files (decompression bombs, giant JSON, malformed media) must produce an error card, never a crash or a frozen frame; preview work must never run on the UI thread; video needs a system ffmpeg or bindings (largest risk, isolated in its own task group).
