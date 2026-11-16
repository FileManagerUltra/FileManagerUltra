## Why

Preview can only play one video, silently, and stops as soon as the selection changes. There is no audio output, no playlist and no fullscreen, so FileManagerUltra cannot replace a separate player for music or video folders. A dedicated Media pane that owns playback independently of what is selected closes that gap.

## What Changes

- New `Media` pane kind: a dockable pane that plays audio and video with sound, a transport bar (play/pause, seek, volume, mute, speed, previous/next, shuffle, repeat), cover art and tags for music, and a queue list.
- Playback queue owned by the engine `Session`, not the pane: it survives selection changes, pane resizes and, for audio, closing the pane when the user chooses "keep playing". Auto-advance, shuffle, repeat (off/all/one), M3U/M3U8/PLS import and export, and a persisted queue (paths only).
- Entry points from Files and Preview panes: "Play", "Add to queue", "Play folder", drag and drop of files onto the pane, and "Play in Media pane" from a Preview video card.
- Queue actions preserve the playlist: Add appends, Play next inserts after the current item, and Play inserts there and starts immediately; played files are never automatically removed.
- Audio output with A/V sync: audio is the master clock; video frames are dropped or held to follow it. Decoding stays out of process (`ffmpeg`/`ffprobe` sidecar) except for audio-only files that `symphonia` can decode.
- Fullscreen: a fullscreen mode for video (and the cover/queue view for audio) with auto-hiding controls, Esc to leave, and a window-level fullscreen toggle.
- Familiar YouTube-style keyboard controls: Space/K, J/L, arrows, number-key seeking, mute, fullscreen, queue navigation, paused frame stepping, speed and subtitle toggles, with visible shortcut help and input-ownership safeguards. Preview shares the basic single-file transport shortcuts.
- Audio and subtitle track selection, frame step, A-B loop, sleep timer, and speed 0.5×–3× with pitch kept.
- Audio pipeline: one float pipeline with a single high-quality resample, a 10-band graphic equalizer with preamp and presets, and gapless playback for in-process formats.
- Subtitles: external and embedded SRT/WebVTT drawn by the UI; ASS/SSA and bitmap (PGS, VobSub) rendered by the decoder; delay adjustment.
- Audiobooks: m4b and marked folders, chapters, multi-file books as one timeline, ±15/30 s skip, per-book speed, bookmarks, progress and finished state; resume position for audiobooks and video (500 entries).
- System integration: media keys and now-playing via MPRIS, Windows SMTC and macOS Now Playing; hardware video decoding (VA-API, D3D11/DXVA, VideoToolbox) with automatic software fallback.
- Exactly one Media pane exists at a time; a second request focuses it.
- `ffmpeg`/`ffprobe` are not bundled: they are discovered on the system, and Windows offers a pinned, verified download (see the `managed-ffmpeg` change); `cpal` is used for audio output, with ALSA headers as a documented Linux build-time exception.
- Remote files (SMB/SFTP) play through a bounded local spool.
- Preview keeps simple single-file playback and gains sound: video with audio, and audio files with cover art and tags (play/pause, seek, volume, mute, no queue; audio never autoplays by default). Preview and the Media pane share one audio output (starting one pauses the other); explicit Play handoff stops Preview's decoder and transfers its position. Preview also offers "Play next" / "Add to queue" without interrupting playback.
- **No breaking changes.** The "audio output is out of scope" statement is removed.

## Capabilities

### New Capabilities
- `media-pane`: the Media pane kind (exactly one), playback queue and playlists, transport, fullscreen, remote spooling, and media commands.
- `media-audio-pipeline`: float audio pipeline and output, gapless, equalizer, speed with pitch kept, A-B loop and sleep timer.
- `media-subtitles`: subtitle sources, text and bitmap rendering, delay.
- `media-audiobooks`: audiobooks, chapters, multi-file books, resume position, bookmarks, finished state.
- `media-integration`: OS media controls and hardware video decoding.

### Modified Capabilities
- `pane-types`: adds the `Media` kind to `pane.kind.set` and its lifecycle rules.
- `preview-pane`: simple single-file video playback gains sound, audio files get simple playback, and both hand off to the Media pane; Preview and Media share one player.
- `app-shell`: window fullscreen toggle and its interaction with saved window geometry.
- `security`: media files, playlists and decoder subprocesses are untrusted and bounded.
- `build-guardrails`: ffmpeg discovered rather than bundled, and the documented ALSA build exception.

## Impact

- New crate `crates/media` (L1, no egui): queue model, playlist parsing, audio sink abstraction, decoder subprocess control, A/V clock. Registered in `xtask/src/layers.rs`.
- `fm-layout`: `PaneKind::Media` and per-pane view state. `fm-engine`: `media_cmds.rs` (new module), session queue, job lane use for spooling. `fm-ui`: media pane widgets in dark and light themes (new theme tokens only, no `Color32::` literals). `fm-app`: menus, shortcuts, fullscreen handling.
- Dependencies: `symphonia`, `cpal` (audio output), `rubato` (resampling), possibly `lofty` (tags) and `souvlaki` (media controls, zbus feature on Linux). `cpal` needs ALSA development headers on Linux: a third user-approved exception to "no system libraries", to be recorded in AGENTS.md and `docs/development.md`.
- Packaging: no ffmpeg in the installers (originally planned as bundled LGPL sidecars; replaced by the `managed-ffmpeg` download); Linux packages need `libasound2`.
- Docs: `docs/architecture.md`, `docs/roadmap.md`, specs above, `CHANGELOG.md`.
