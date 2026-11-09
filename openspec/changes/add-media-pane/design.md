## Context

Preview plays one silent video through an `ffmpeg` child piping RGBA frames (`crates/preview/src/player.rs`) and stops when the selection changes. Panes are views (`PaneKind` in `fm-layout`); behaviour lives in engine commands; the UI thread never blocks; backends are pure Rust with no FFI and no `unsafe`; decoders already run out of process so a crash cannot take the app down.

## Goals / Non-Goals

**Goals:**
- Audio and video playback with sound in a dedicated pane, with a queue, playlists and fullscreen.
- Playback independent of selection and pane focus; one place that drives the audio device.
- Everything is a command; the pane is a thin view over session state.
- Same behaviour on Linux, Windows and macOS; a missing decoder or audio device degrades with a hint.

**Non-Goals:**
- Streaming URLs, DVD/Blu-ray, DRM, video filters/equalizer, casting, a library database, OS media-key/MPRIS integration (follow-up), gapless crossfade.
- A queue, playlists or fullscreen in Preview (those live in the Media pane); changing Preview's poster and metadata cards.

## Decisions

1. **New `PaneKind::Media`, queue in `Session`.** The pane renders `Session::media` state; closing or switching the pane does not clear the queue. Audio may continue with the pane closed ("keep playing", default on) and shows a status-bar indicator; video stops with the pane. Alternative (extend Preview) rejected: its lifetime is tied to selection.
2. **New crate `fm-media` at L1** (no egui, no I/O beyond decoder subprocesses and the audio device): `Queue` (items, current, shuffle order, repeat), playlist parse/write (M3U/M3U8/PLS), `AudioSink` trait with a null/fake implementation for tests, `Clock`, decoder control. `engine` owns the single `MediaSession` and ticks it from its job lanes; `ui` only reads snapshots.
3. **Decoding.** Audio-only files `symphonia` can decode (mp3, flac, ogg/vorbis, wav, aac/m4a, alac) play in-process on a dedicated audio thread inside `fm-media`. Everything else, and the soundtrack of videos, uses `ffmpeg` subprocesses discovered like today (`find_tool`): one for RGBA video frames on stdout, one for raw `s16le` PCM. A decoder crash or hang ends playback of that item with an error and advances; it never takes the app down.
4. **Clocking.** The audio sink's played-sample count is the master clock. The video reader keeps only the newest frames; the UI picks the frame whose pts is closest to the clock, dropping late frames and holding early ones. Videos without audio use a monotonic wall clock. Seeking restarts both decoders at the target (with a short cap on seek rate); pause stops the sink and the clock without killing decoders for audio-only playback.
5. **Audio device crate.** Use `cpal` behind the `AudioSink` trait. On Linux its ALSA backend needs `libasound2-dev` at build time; to keep the "no system libraries" rule, evaluate a pure-Rust PipeWire/PulseAudio backend first and only fall back to ALSA behind a default-on cargo feature with the exception documented in `docs/development.md` and AGENTS.md (user approval required). No device available means the pane shows a hint and plays silently (video) or refuses (audio) with an error.
6. **Fullscreen.** Two levels: pane fullscreen (media pane fills the window; other panes hidden, state restored on exit) via `media.fullscreen`, and optional window fullscreen via `window.fullscreen` (`ViewportCommand::Fullscreen`) toggled together. Controls auto-hide after 3 s without pointer movement; Esc, `F`, or double-click leaves. Fullscreen never changes saved normal geometry.
7. **Remote files.** Non-local items are copied through the normal job lane into an owner-only temp spool (same rules as "Open remote file": 0700 folder, size cap, deleted on quit), then played from disk. Progress shows in the pane.
8. **Commands** (all in `engine/src/media_cmds.rs`): `media.play`, `media.pause`, `media.toggle`, `media.stop`, `media.next`, `media.prev`, `media.seek`, `media.volume`, `media.mute`, `media.speed`, `media.enqueue`, `media.play_folder`, `media.remove`, `media.move`, `media.clear`, `media.shuffle`, `media.repeat`, `media.track.audio`, `media.track.subtitle`, `media.step`, `media.fullscreen`, `media.playlist.load`, `media.playlist.save`, plus `window.fullscreen`. Each has id, label, menu path, shortcut, params doc, `enabled` and a bad-params test.
9. **Persistence.** The queue (file paths/URIs only), current index, volume, shuffle and repeat persist in prefs/workspace state; no credentials, owner-only files. Items that no longer exist are skipped with a notice.
10. **Preview keeps simple playback, on the same engine.** Preview's single-file player (video and audio) uses the same `fm-media` decoders, audio sink and clock as the Media pane, but through its own lightweight `PreviewPlayer` owned by the pane (stopped on selection change/close). A session-level `AudioOwner` guarantees one active player: starting either pauses the other. "Play in Media pane" transfers file and position. Audio does not autoplay in Preview by default.
11. **Delivery in two slices.** Slice A: `fm-media` audio sink/clock, audio via `symphonia`, Preview audio and video-with-sound, the `AudioOwner` rule, tags/cover art. Slice B: Media pane kind, queue, playlists, commands. Slice C: A/V sync polish, track selection, frame step, fullscreen, remote spool, hand-off.

## Risks / Trade-offs

- **ALSA build dependency on Linux** → decision 5; may need a documented exception.
- **A/V drift or stutter with two subprocesses** → audio master clock, frame dropping, and a drift test with a fake sink.
- **Seek latency from restarting decoders** → accept for v1; keyframe-aligned seeks for video.
- **Bundled ffmpeg size and licensing** → keep the sidecar optional as today; document LGPL/GPL build requirements before packaging.
- **Hostile media and playlists** → decoders stay out of process with timeouts, playlist size/entry caps, path validation (see `security` delta).
- **Audio from a closed pane surprises users** → status-bar indicator with a stop button; pref to stop on close.
- **Memory** → only the newest video frames and a bounded PCM ring buffer are kept.
