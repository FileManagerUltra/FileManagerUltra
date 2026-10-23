## 1. Layout and engine plumbing

- [x] 1.1 `fm-layout`: `PaneKind::Preview`, `PaneState.preview_source`, `Focus.last_files_pane`; tests
- [x] 1.2 `pane.kind.set` accepts `preview`; leaving it drops preview state; spec delta tests
- [x] 1.3 `Session.previews`, source resolution (pure + tests), token/debounce, job submission, stale-result drop
- [x] 1.4 `preview_cmds.rs`: `pane.preview.source`, `preview.zoom`, `preview.toggle_wrap`, `preview.toggle_pretty`; register; bad-param fuzz

## 2. `fm-preview` crate (L1)

- [x] 2.1 Scaffold, register at L1 in `xtask/src/layers.rs`; `Preview` model, limits as constants
- [x] 2.2 Sniffing (magic, UTF-8/BOM, NUL) with table tests incl. misleading extensions
- [x] 2.3 Text + syntax classes (syntect, lazy), truncation, long-line clipping
- [x] 2.4 JSON/YAML/TOML/XML highlighted raw by default plus on-demand pretty form (validity check, size cap, never changes the raw view); Markdown blocks
- [x] 2.5 Images (png/jpeg/gif/webp/bmp/ico, header check, frame/size caps); SVG (resvg) deferred: shown as XML text
- [x] 2.6 Hex dump, directory summary; audio tags (lofty) and PDF info (lopdf) deferred: metadata card only
- [x] 2.8 UTF-16/BOM/CRLF decoding; `platform` module (decoder discovery, no-console flags, per-OS paths) with pure tests that cover all three OS branches
- [x] 2.7 Panic/timeout containment wrapper; hostile-file tests (bomb header, truncated, garbage, deep JSON nesting)

### Log mode

- [x] 2.9 `fm-preview::log`: format detection + parsers (nginx/apache access+error, monolog, PHP, magento report JSON, generic), multi-line folding, timestamp parsing; table-driven tests with real-world samples and garbage lines
- [x] 2.10 Bounded tail-first window (8 MiB, 64 MiB cap, "Load earlier") over `Vfs`, `.gz` streaming with size cap, cancellable regex/level/time filters, follow with truncate/rotation handling; tests for 1 GB sparse file, rotation, bad regex
- [x] 3.6 UI log view: virtualized rows, level/status colouring tokens (dark+light), access-log column view, fold stack traces, filter bar, search, follow toggle (next/prev and snapshots deferred)
- [x] 4.3 Commands `preview.log.*`, menu entries, shortcuts, graceful-failure tests

## 3. UI

- [x] 3.1 Theme tokens for syntax classes + preview chrome (dark and light, exhaustive)
- [x] 3.2 `fm-ui::preview`: text with line numbers/wrap, structured, markdown, image with fit/zoom/pan + animation, meta card, hex, directory card, error card
- [x] 3.3 "Pretty" toggle button in the preview header (JSON/YAML/TOML/XML only; disabled + tooltip when invalid/too large)
- [x] 3.5 Header source picker (Follow focus / pane list); type switcher entry
- [ ] 3.4 Snapshots: text, image, error card in dark and light

## 4. App wiring

- [x] 4.1 Menu entries (`View ▸ Preview`, Panes ▸ Preview source), shortcuts (terminal-safe chords), `all_ids`, shortcut tests
- [x] 4.2 Repaint cadence while animating/playing only

## 5. Video

- [x] 5.1 `fm-preview::video`: locate ffmpeg/ffprobe, probe metadata, poster frame; absent → hint card
- [ ] 5.2 (deferred: poster + metadata only for now) Playback: rawvideo/s16le pipes, frame pump with byte budget, play/pause/seek, kill on leave/close/quit; audio output (rodio) behind feature
- [x] 5.3 Sidecar/PATH/Homebrew discovery order, `CREATE_NO_WINDOW` (audio feature deferred with 5.2)
- [x] 5.4 Hostile argument tests; hung child is killed; real-video test (skips without ffmpeg)

## 6. Docs and gate

- [x] 6.1 `docs/architecture.md` (preview data flow, limits), `docs/development.md` smoke list, roadmap
- [ ] 6.2 Dependency audit: every new crate builds on Linux/Windows/macOS with no system libs (record in design); CI matrix item added or tracked in roadmap
- [ ] 6.3 `cargo xtask ci` green; user smoke; `log/devlog.md` entry
