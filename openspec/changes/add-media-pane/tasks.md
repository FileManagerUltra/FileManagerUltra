## 1. Foundations (slice A)

- [x] 1.1 Record the approved `cpal`/ALSA build-time exception in AGENTS.md and `docs/development.md`; install `libasound2-dev` in CI
- [x] 1.2 Create `crates/media` (L1, no egui), register it in `xtask/src/layers.rs`, add `symphonia`, `cpal`, `rubato` (and `lofty` if used)
- [x] 1.3 Implement the `AudioSink` trait, a bounded f32 ring buffer, a null/fake sink for tests, the audio-master `Clock`, and the pipeline (single resample, preamp, volume, soft limiter)
- [x] 1.4 Implement `symphonia` audio decoding on a dedicated thread (mp3, flac, ogg, wav, aac/m4a, alac) with seek, duration, tags and size-capped cover art
- [x] 1.5 Tests: decode round-trips on generated fixtures, clock/ring-buffer invariants, corrupt and truncated files return errors and never panic

## 2. Preview playback with sound (slice A)

- [x] 2.1 Add the ffmpeg PCM audio subprocess next to the video pipe in `fm-preview` player, sharing discovery, timeouts and kill-on-drop
- [x] 2.2 Drive video frames from the audio clock (drop late, hold early); keep the monotonic clock fallback for silent videos
- [x] 2.3 Add a `PreviewPlayer` for audio files (cover art, tags) with play/pause, seek, volume, mute and the "autoplay audio" preference (off by default)
- [x] 2.4 Add the session `AudioOwner` rule so starting Preview or Media playback pauses the other
- [x] 2.5 UI: volume and mute controls and audio card in the Preview pane, dark and light snapshots via `egui_kittest`
- [x] 2.6 Tests: selection change/close stops decoders, no audio device plays silently with a hint, one-player rule, no unbounded memory under a stalled UI

## 3. Media pane and queue (slice B)

- [x] 3.1 Add `PaneKind::Media` in `fm-layout` with serde fallback, view state and `pane.kind.set` support; update the kind selector; enforce exactly one Media pane (focus the existing one; extra panes in saved workspaces become Files)
- [x] 3.2 Implement `Queue` in `fm-media` (enqueue, remove, move, clear, shuffle order, repeat off/all/one, previous/next) with invariant tests
- [x] 3.3 Implement M3U/M3U8/PLS parse and M3U8 write with size, entry, line-length caps, URL-scheme refusal and relative-path resolution; tests incl. hostile input
- [x] 3.4 Add `engine/src/media_cmds.rs` with all `media.*` commands, register in `commands.rs`, menu paths and shortcuts; bad-params tests and command-fuzzer coverage (commands are split across `media_*_cmds.rs`; the Media menu was later replaced by pane controls, Preferences > Media and `fm-ui::media_keys`)
- [x] 3.5 Session `MediaSession`: current item, position, volume, auto-advance, skip-with-notice on errors, background-audio preference and status-bar indicator with stop
- [x] 3.6 Persist queue (paths only), index, volume, shuffle and repeat owner-only; skip vanished items with a notice
- [x] 3.7 UI: Media pane widgets (cover/tags, transport bar, queue list with reorder/remove, drag-and-drop enqueue) using theme tokens only; dark and light snapshots
- [x] 3.8 Entry points: Play, Play next, Add to queue and Play folder in Files and Preview menus; preserve playlist entries, append vs insert-after-current semantics, and auto-create or switch a Media pane when none exists
- [x] 3.9 Tests: auto-advance, repeat all/one, shuffle cycle, save/load playlist round trip, close-pane behaviour for audio and video

Local-audio transport and queue commands, artwork, a virtualized queue view and the
background Stop indicator are wired. Local playlist load/save commands and path
dialogs, natural-order non-recursive Play folder, and internal/external Media
drops are implemented. Imports append without changing active playback; idle drops
start the first new item. Playlist export creates a new M3U8 file without overwriting.
Files/Preview local audio/video entry points are implemented: Add appends, Play next
inserts after current without interrupting playback, and Play inserts there and
starts immediately. Existing entries are retained; queue-row Play never duplicates
the row. Preview Play transfers position and stops its decoder, while Add/Play
next leave playback unchanged. Explicit folder context targets are preserved.
Video uses the shared decoder and clock with worker-backed cancellable probing,
fitted frames, seeking, paused frame stepping and pitch-preserving 0.5×–3× speed.
Closing/switching away from the Media pane or switching to a workspace without it
stops video even when background audio is enabled. Task 3.4 remains open for later
commands; audio-only speed, remote playback, tracks, fullscreen and keyboard
controls remain pending.

## 4. Video, tracks and fullscreen (slice C)

- [x] 4.1 Video in the Media pane using the shared decoders and clock; frame step while paused; pitch-preserving speed control from 0.5× to 3× in 0.05× steps
- [x] 4.2 Probe embedded audio and subtitle tracks with `ffprobe` and add `media.track.audio` / `media.track.subtitle`
- [x] 4.3 Implement `media.fullscreen` (pane fullscreen preserving layout, focus and selection) and `window.fullscreen` (viewport fullscreen, geometry not overwritten, not persisted)
- [x] 4.4 Auto-hide controls and cursor after 3 s, Esc/F/double-click to leave, leave on pane close or stopped queue
- [x] 4.5 Remote items: spool through the job lane to the owner-only temp folder with progress, size cap and cleanup
- [x] 4.6 Preview hand-off: Play transfers file and position and stops Preview's decoder; Add appends and Play next inserts after current without changing playback
- [x] 4.7 Implement the YouTube-style keyboard mapping in `media-pane` (Space/K, J/L, arrows, 0–9, Home/End, M, F/Esc, Shift+N/P and N/P aliases, comma/period, </>, C), basic Preview transport equivalents, shortcut help/tooltips and menu wiring; test exact increments, clamping, unknown-duration guards, paused frame stepping, single dispatch and terminal/text/modal/widget input ownership
- [x] 4.8 Tests: fullscreen round trip, A/V drift with the fake sink and a stalled UI, hung decoder is killed, vanished file advances

## 5. Audio features (slice D)

- [x] 5.1 Gapless: expose encoder delay/padding, pre-open the next item, keep the sink open; tests with generated tones
- [x] 5.2 10-band biquad equalizer with preamp, presets (built-in and user), persistence, `media.eq`; tests for response, range validation and no-dropout changes
- [ ] 5.3 Speed 0.5×–3× with pitch kept via ffmpeg `atempo` chain; per-book persistence; `media.speed` validation tests (video speed done in 4.x; audio-only pitch-preserving speed and per-book persistence are still open: they need a time-stretcher in the audio path)
- [x] 5.4 A-B loop (`media.loop`) and sleep timer with fade-out (`media.sleep`: minutes, end of item, end of chapter; end of chapter arrives with chapters in 7.x)

## 6. Subtitles and hardware decoding (slice E)

- [x] 6.1 SRT/WebVTT parser with caps, external-file discovery (`.en.srt` style), `media.subtitle.load`, `media.subtitle.delay`, UI overlay with size/position prefs (`fm-media::subtitle`, `fm-preview::subtitles`, `engine::media_subtitle_cmds`; embedded text tracks and ASS/SSA are converted to cues by ffmpeg so they share the overlay; G/H keys move the delay)
- [x] 6.1a Subtitle appearance for low vision, identical for every text format: `engine::subtitle_style` + `media.subtitle.style` (size, position, font, bold, colours and opacity, background box, edge style/colour/thickness, letter/line spacing, presets), a Subtitles section in Preferences > Media with live preview and WCAG contrast check (`fm-ui::subtitles` paints both), theme tokens `subtitle_text`/`subtitle_outline`
- [ ] 6.2 Bitmap (PGS, VobSub) subtitles shown over the video, honouring size/position/background where possible; track switching restarts only the video decoder (ASS/SSA is no longer burned in: it is drawn as text with the user's style, see 6.1)
- [ ] 6.3 Hardware decoding flags per platform with `hwdownload`, automatic software fallback, preference and active-mode indicator
- [ ] 6.4 Tests: malformed and oversized subtitle files, delay arithmetic (done with 6.1), fallback restarts at the current position (with 6.3)

## 7. Audiobooks and resume (slice F)

- [x] 7.1 Book model: m4b, genre tag and user-marked folders; multi-file timeline with per-file chapters; chapters from `ffprobe`
- [x] 7.2 Controls: next/previous chapter, ±15/30 s skip, progress percentage, `media.skip`, `media.chapter`, `media.book`
- [x] 7.3 Resume store (500 items, size+mtime check, 3 s rewind, finished at 98 % or 30 s), bookmarks (100 per item), `media.bookmark`, `media.finished`, clear from Preferences; owner-only file
- [x] 7.4 Tests: boundary-crossing seeks, resume after file replacement, cap eviction, bad params for every new command

## 8. System integration and packaging (slice G)

- [ ] 8.1 `souvlaki` (or equivalent) behind a trait in `fm-app`: MPRIS (zbus), SMTC, macOS Now Playing; dispatch only `media.*`; re-add `raw-window-handle` if needed
- [x] 8.2 ~~Bundle pinned LGPL `ffmpeg`/`ffprobe` in the installers~~ Superseded by `managed-ffmpeg`: installers do not bundle ffmpeg; Windows offers a pinned, SHA-256-verified download from Preferences > Media, elsewhere ffmpeg is discovered on the system (`build-guardrails` delta updated)
- [ ] 8.3 Verify the `.deb`/AppImage depend on or bundle `libasound2`, and that no installer contains `ffmpeg`/`ffprobe`

## 9. Hardening, docs and release

- [ ] 9.1 Extend `security` coverage: playlist caps, decoder timeouts, spool permissions, no credentials in queue state or notices
- [ ] 9.2 Update `docs/architecture.md` (crate map, `AudioOwner`, clock, lanes), `docs/roadmap.md`, `docs/development.md` (audio backend, ffmpeg), and sync the delta specs
- [ ] 9.3 Add `CHANGELOG.md` entries under Unreleased and a `log/devlog.md` entry
- [ ] 9.4 Gate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`; Windows and macOS smoke tests for audio device, ffmpeg discovery and fullscreen
