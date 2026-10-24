## Context

Pane kinds exist (`PaneKind::{Files, Terminal, Sftp, Network}`); per-pane runtime objects live in `Session` keyed by `PaneId` (terminals). Heavy work must run off the UI thread on the job lanes. Only `fm-term`/`fm-sftp` may own threads or runtimes below L4.

## Decisions

**D1 — Model/render split.** `fm-preview` (L1) turns bytes into a plain `Preview` enum: `Text{lines, language, truncated}`, `Structured{kind, text}`, `Markdown{blocks}`, `Image{frames, size}`, `Svg{rgba}`, `Meta{rows}`, `Hex{rows}`, `Dir{summary}`, `Video{...}`, `Error{message}`. No egui. `fm-ui::preview` paints it; colours come from theme tokens (syntax token classes → semantic colours, dark and light, exhaustive).

**D2 — Request/response with tokens.** `Session.previews: BTreeMap<PaneId, PreviewState>` holds `{source: Source, target: Option<VfsUri>, token, result: Loading|Ready(Arc<Preview>)|Failed}`. When the source pane's focused entry (or selection of exactly one item) changes, the engine bumps `token` and submits a job on the entry's lane. Results with a stale token are dropped. Debounce 120 ms so holding an arrow key does not render every file.

**D3 — Source selection.** `PreviewSource::{Follow, Pane(PaneId)}`. `Follow` resolves to `layout.focus.last_files_pane` (updated by `focus_pane` whenever a Files pane gains focus); the preview pane itself never counts. A pinned source that closes reverts to `Follow`. Resolution is a pure function with tests.

**D4 — Sniffing.** Read up to 64 KiB head; detect by magic bytes (png/jpeg/gif/webp/bmp/ico/pdf/zip/ogg/mp3/mp4/mkv…), then BOM/UTF-8 validity for text, then extension for the language/structure hint. Binary with NUL bytes in the head → hex. Extension never overrides magic.

**D5 — Limits (hostile input).** Text/structured: read ≤ 1 MiB, show first 20 000 lines, lines clipped at 4 096 chars. Pretty-printing is opt-in (`Preview::Structured{raw, pretty: Option<..>}`: the raw text is always shown first; the pretty form is produced on demand by the toggle, only when the whole file ≤ 1 MiB, and the toggle is disabled with a reason for invalid or oversized files). The choice is per Preview pane and resets to raw when the file changes. Images: file ≤ 64 MiB, decoded size ≤ 100 megapixels checked from the header *before* allocating, animated frames capped (200 frames / 256 MiB total), downscaled to the pane's max texture side (4096). SVG: size/node caps, no external resources, no scripts. Markdown: ≤ 1 MiB, links never followed. Hex: first 4 KiB. Directory summary: reads only the cached listing.

**D6 — Panic isolation.** Each render runs in its own short-lived thread joined by the job; a panic in a third-party decoder becomes `Preview::Error("the file could not be previewed")` and never reaches the UI. A wall-clock budget (3 s) marks the result `Failed("timed out")` and discards a late result via the token.

**D7 — Remote files.** Previews read through `Vfs::open_read` with the same caps, so SFTP/SMB work once those backends land; reads of remote files happen only for the selected entry, never for hover.

**D8 — Video (decision needed).** The requirement is "full playback". Options:
1. *ffmpeg bindings* (`ffmpeg-next`): needs libav dev headers, `pkg-config`, `libclang` at build time and a shared libav at run time on every platform; bloats installers and complicates the licence story (LGPL/GPL build flags).
2. *ffmpeg subprocess* (recommended): spawn `ffmpeg`/`ffprobe` found on `PATH` (or a bundled sidecar binary in installers), decode to `rawvideo` rgba frames over a pipe at the pane's size and to `s16le` for audio, so no native link dependencies, no `unsafe`, and a crash or hang in the decoder cannot take the app down. Audio output through `rodio` (needs ALSA dev headers on Linux builds). Without ffmpeg the pane shows poster-less metadata and an "install ffmpeg" hint.
The tasks implement option 2 behind a `video` cargo feature that is on by default; switching to bindings later only replaces `fm-preview::video`.

**D9 — Playback is bounded.** One playing video per pane; leaving the file, switching kind or closing the pane kills the child (same ownership rule as terminals). Frame pump has a per-frame byte budget; repaint requested only while playing.

**D10 — Cross-platform (Linux, Windows, macOS).**
- Dependencies must build on all three with no system libraries at build time: pure-Rust crates only (`syntect` with `fancy-regex`, not `onig`; `image`, `resvg`, `lofty`, `lopdf`, `pulldown-cmark`). Anything needing pkg-config, libclang or a C toolchain beyond what the workspace already needs is out.
- Video via subprocess needs no link-time dependency. The decoder is found by: bundled sidecar next to the executable (installers ship it), then `PATH` (`ffmpeg`/`ffmpeg.exe`, also Homebrew `/opt/homebrew/bin` and `/usr/local/bin` on macOS, where GUI apps get a minimal `PATH`). Child processes on Windows use `CREATE_NO_WINDOW` so no console flashes; killing the child uses the platform kill (`Child::kill`) and waits for it.
- Audio output (`rodio`/cpal) is behind a feature: ALSA headers on Linux, CoreAudio on macOS, WASAPI on Windows; with the feature off, video plays silently and says so. CI builds both feature settings.
- Paths and names: files are opened through `Vfs` (no raw `Path` string handling), non-UTF-8 names are shown lossily and passed to the decoder as `OsStr`, never through a shell; Windows `\\?\` and reserved names, macOS NFD names and case-insensitive filesystems need no special casing because the preview keys on `VfsUri`.
- Text decoding: UTF-8 (with/without BOM), UTF-16 LE/BE by BOM, otherwise lossy; CRLF and LF both render as line breaks.
- Behaviour that differs per OS is isolated in `fm-preview::platform` with `cfg` and a fake used by tests, so the Linux test run exercises the argument-building and discovery logic for all three.
- Verification gap: this machine only builds Linux; Windows/macOS paths are written carefully, covered by pure-logic tests, and flagged unverified until CI runs there.

### D11 Log mode
- Sniffed as a log by name (`*.log`, `*.log.N`, `*.log.gz`, under `var/log`/`var/report`/`/var/log/{nginx,apache2,httpd}`) or by the first lines matching a known format; user can force plain text.
- `fm-preview::log`: pure parsers per format (`nginx_access`, `nginx_error`, `apache_access`, `apache_error`, `monolog` for Magento/Pimcore, `php`, `magento_report` JSON, `generic`) producing `LogRecord{ts, level, fields, text, line_range}`; multi-line records (stack traces) fold into the preceding record. Level maps to semantic tokens (error/warn/info/debug), HTTP status to 2xx/3xx/4xx/5xx tokens; no `Color32::` outside `theme.rs`.
- Large files: never read whole. A background indexer records record-start byte offsets in chunks (4 MiB) starting from the end, so the tail is visible at once and older history fills in; the view virtualizes over the index and reads only visible ranges through `Vfs`. Hard cap on index memory; beyond it, older records are reachable only by search.
- `.gz` is decompressed in a streaming reader (pure Rust `flate2` rust backend) with a decompressed-size cap; index is built forward since gz can't seek from the end.
- Follow: re-stat on a 500 ms timer (not `notify`, works on remote/SMB), read appended bytes, handle truncate/rotation by reindexing; stops when the user scrolls up, resumes on End. Local and SFTP only via `Vfs`.
- Filters (level, regex, time range) are evaluated on the job lane over the index, cancellable by token; regex uses `regex` (linear time, size-limited) so hostile patterns can't hang.
- Timestamps parsed per format (nginx `dd/Mon/yyyy:HH:MM:SS +zzzz`, error `yyyy/mm/dd`, ISO 8601 / Monolog `[yyyy-mm-dd hh:mm:ss]`); unparsable lines stay visible and are excluded only by an active time filter.

## Risks / Trade-offs

- Syntax highlighting adds start-up cost → load syntaxes lazily on first text preview.
- Debounce vs. responsiveness: 120 ms feels instant for keyboard navigation and avoids thrash.
- Subprocess video is less integrated than bindings (no frame-exact seeking at first) → seek restarts the child at a timestamp; acceptable for a preview.

**D12 — Dependency audit (task 6.2).** Every preview dependency is pure Rust and needs no system libraries or C toolchain at build time on Linux, Windows or macOS: `image` (rust decoders for png/jpeg/gif/webp/bmp/ico), `syntect` with `default-fancy` (fancy-regex, not onig), `pulldown-cmark`, `serde_yaml_ng`, `quick-xml`, `toml`, `serde_json`, `regex`, `flate2` (rust backend), `fast_image_resize` (runtime SIMD dispatch), `crossbeam-channel`. Video does not link ffmpeg: `ffmpeg`/`ffprobe` are subprocesses discovered from a sidecar/PATH, so a missing decoder is a UI hint, not a build failure. The only platform-sensitive pieces are the process flags (`CREATE_NO_WINDOW` on Windows) and the decoder search paths (Homebrew locations on macOS), both in `fm-preview::platform` with OB/OS-branch unit tests. No CI matrix job exists yet (the workspace is developed on Linux); the roadmap tracks a Windows/macOS build check for `live-watch-and-packaging`.
