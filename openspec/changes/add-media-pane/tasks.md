## 1. Foundations (slice A)

- [ ] 1.1 Decide the Linux audio backend (pure-Rust PipeWire/Pulse vs `cpal`+ALSA); if ALSA is needed, get user approval and document the exception in `docs/development.md` and AGENTS.md
- [ ] 1.2 Create `crates/media` (L1, no egui), register it in `xtask/src/layers.rs`, add `symphonia` (and `lofty` if used) and the chosen audio crate
- [ ] 1.3 Implement the `AudioSink` trait, a bounded PCM ring buffer, a null/fake sink for tests, and the audio-master `Clock`
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

- [ ] 3.1 Add `PaneKind::Media` in `fm-layout` with serde fallback, view state and `pane.kind.set` support; update the kind selector
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

## 5. Hardening, docs and release

- [ ] 5.1 Extend `security` coverage: playlist caps, decoder timeouts, spool permissions, no credentials in queue state or notices
- [ ] 5.2 Update `docs/architecture.md` (crate map, `AudioOwner`, clock, lanes), `docs/roadmap.md`, `docs/development.md` (audio backend, ffmpeg), and sync the delta specs
- [ ] 5.3 Add `CHANGELOG.md` entries under Unreleased and a `log/devlog.md` entry
- [ ] 5.4 Gate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`; Windows and macOS smoke tests for audio device, ffmpeg discovery and fullscreen
