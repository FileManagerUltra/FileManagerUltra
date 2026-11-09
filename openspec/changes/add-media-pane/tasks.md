## 1. Foundations (slice A)

- [ ] 1.1 Record the approved `cpal`/ALSA build-time exception in AGENTS.md §2 and `docs/development.md`; install `libasound2-dev` in CI
- [ ] 1.2 Create `crates/media` (L1, no egui), register it in `xtask/src/layers.rs`, add `symphonia`, `cpal`, `rubato` (and `lofty` if used)
- [ ] 1.3 Implement the `AudioSink` trait, a bounded f32 ring buffer, a null/fake sink for tests, the audio-master `Clock`, and the pipeline (single resample, preamp, volume, soft limiter)
- [ ] 1.4 Implement `symphonia` audio decoding on a dedicated thread (mp3, flac, ogg, wav, aac/m4a, alac) with seek, duration, tags and size-capped cover art
- [ ] 1.5 Tests: decode round-trips on generated fixtures, clock/ring-buffer invariants, corrupt and truncated files return errors and never panic

## 2. Preview playback with sound (slice A)

- [ ] 2.1 Add the ffmpeg PCM audio subprocess next to the video pipe in `fm-preview` player, sharing discovery, timeouts and kill-on-drop
- [ ] 2.2 Drive video frames from the audio clock (drop late, hold early); keep the monotonic clock fallback for silent videos
- [ ] 2.3 Add a `PreviewPlayer` for audio files (cover art, tags) with play/pause, seek, volume, mute and the "autoplay audio" preference (off by default)
- [ ] 2.4 Add the session `AudioOwner` rule so starting Preview or Media playback pauses the other
- [ ] 2.5 UI: volume and mute controls and audio card in the Preview pane, dark and light snapshots via `egui_kittest`
- [ ] 2.6 Tests: selection change/close stops decoders, no audio device plays silently with a hint, one-player rule, no unbounded memory under a stalled UI

## 3. Media pane and queue (slice B)

- [ ] 3.1 Add `PaneKind::Media` in `fm-layout` with serde fallback, view state and `pane.kind.set` support; update the kind selector; enforce exactly one Media pane (focus the existing one; extra panes in saved workspaces become Files)
- [ ] 3.2 Implement `Queue` in `fm-media` (enqueue, remove, move, clear, shuffle order, repeat off/all/one, previous/next) with invariant tests
- [ ] 3.3 Implement M3U/M3U8/PLS parse and M3U8 write with size, entry, line-length caps, URL-scheme refusal and relative-path resolution; tests incl. hostile input
- [ ] 3.4 Add `engine/src/media_cmds.rs` with all `media.*` commands, register in `commands.rs`, menu paths and shortcuts; bad-params tests and command-fuzzer coverage
- [ ] 3.5 Session `MediaSession`: current item, position, volume, auto-advance, skip-with-notice on errors, background-audio preference and status-bar indicator with stop
- [ ] 3.6 Persist queue (paths only), index, volume, shuffle and repeat owner-only; skip vanished items with a notice
- [ ] 3.7 UI: Media pane widgets (cover/tags, transport bar, queue list with reorder/remove, drag-and-drop enqueue) using theme tokens only; dark and light snapshots
- [ ] 3.8 Entry points: Play, Add to queue and Play folder in Files and Preview menus; auto-create or switch a Media pane when none exists
- [ ] 3.9 Tests: auto-advance, repeat all/one, shuffle cycle, save/load playlist round trip, close-pane behaviour for audio and video

## 4. Video, tracks and fullscreen (slice C)

- [ ] 4.1 Video in the Media pane using the shared decoders and clock; frame step while paused; speed control
- [ ] 4.2 Probe embedded audio and subtitle tracks with `ffprobe` and add `media.track.audio` / `media.track.subtitle`
- [ ] 4.3 Implement `media.fullscreen` (pane fullscreen preserving layout, focus and selection) and `window.fullscreen` (viewport fullscreen, geometry not overwritten, not persisted)
- [ ] 4.4 Auto-hide controls and cursor after 3 s, Esc/F/double-click to leave, leave on pane close or stopped queue
- [ ] 4.5 Remote items: spool through the job lane to the owner-only temp folder with progress, size cap and cleanup
- [ ] 4.6 Preview hand-off: "Play in Media pane" and "Add to queue" transfer file and position
- [ ] 4.7 Keyboard shortcuts for the focused Media pane (Space, arrows, M, N/P, F) and menu wiring; verify shortcut conflicts with terminal and text ownership
- [ ] 4.8 Tests: fullscreen round trip, A/V drift with the fake sink and a stalled UI, hung decoder is killed, vanished file advances

## 5. Audio features (slice D)

- [ ] 5.1 Gapless: expose encoder delay/padding, pre-open the next item, keep the sink open; tests with generated tones
- [ ] 5.2 10-band biquad equalizer with preamp, presets (built-in and user), persistence, `media.eq`; tests for response, range validation and no-dropout changes
- [ ] 5.3 Speed 0.5×–3× with pitch kept via ffmpeg `atempo` chain; per-book persistence; `media.speed` validation tests
- [ ] 5.4 A-B loop (`media.loop`) and sleep timer with fade-out (`media.sleep`: minutes, end of item, end of chapter)

## 6. Subtitles and hardware decoding (slice E)

- [ ] 6.1 SRT/WebVTT parser with caps, external-file discovery (`.en.srt` style), `media.subtitle.load`, `media.subtitle.delay`, UI overlay with size/position prefs
- [ ] 6.2 ASS/SSA and bitmap (PGS, VobSub) subtitles burned in by the ffmpeg filter graph; track switching restarts only the video decoder
- [ ] 6.3 Hardware decoding flags per platform with `hwdownload`, automatic software fallback, preference and active-mode indicator
- [ ] 6.4 Tests: malformed and oversized subtitle files, delay arithmetic, fallback restarts at the current position

## 7. Audiobooks and resume (slice F)

- [ ] 7.1 Book model: m4b, genre tag and user-marked folders; multi-file timeline with per-file chapters; chapters from `ffprobe`
- [ ] 7.2 Controls: next/previous chapter, ±15/30 s skip, progress percentage, `media.skip`, `media.chapter`, `media.book`
- [ ] 7.3 Resume store (500 items, size+mtime check, 3 s rewind, finished at 98 % or 30 s), bookmarks (100 per item), `media.bookmark`, `media.finished`, clear from Preferences; owner-only file
- [ ] 7.4 Tests: boundary-crossing seeks, resume after file replacement, cap eviction, bad params for every new command

## 8. System integration and packaging (slice G)

- [ ] 8.1 `souvlaki` (or equivalent) behind a trait in `fm-app`: MPRIS (zbus), SMTC, macOS Now Playing; dispatch only `media.*`; re-add `raw-window-handle` if needed
- [ ] 8.2 Bundle pinned LGPL `ffmpeg`/`ffprobe` per platform in the release workflow with SHA-256 verification, licence text and source offer; update `cargo xtask packager-config`; check `find_tool` prefers the sidecar
- [ ] 8.3 Verify the `.deb`/AppImage depend on or bundle `libasound2`, the NSIS installer and `.dmg` include the sidecars, and installed size

## 9. Hardening, docs and release

- [ ] 9.1 Extend `security` coverage: playlist caps, decoder timeouts, spool permissions, no credentials in queue state or notices
- [ ] 9.2 Update `docs/architecture.md` (crate map, `AudioOwner`, clock, lanes), `docs/roadmap.md`, `docs/development.md` (audio backend, ffmpeg), and sync the delta specs
- [ ] 9.3 Add `CHANGELOG.md` entries under Unreleased and a `log/devlog.md` entry
- [ ] 9.4 Gate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`; Windows and macOS smoke tests for audio device, ffmpeg discovery and fullscreen
