## Why

Preview can only play one video, silently, and stops as soon as the selection changes. There is no audio output, no playlist and no fullscreen, so FileManagerUltra cannot replace a separate player for music or video folders. A dedicated Media pane that owns playback independently of what is selected closes that gap.

## What Changes

- New `Media` pane kind: a dockable pane that plays audio and video with sound, a transport bar (play/pause, seek, volume, mute, speed, previous/next, shuffle, repeat), cover art and tags for music, and a queue list.
- Playback queue owned by the engine `Session`, not the pane: it survives selection changes, pane resizes and, for audio, closing the pane when the user chooses "keep playing". Auto-advance, shuffle, repeat (off/all/one), M3U/M3U8/PLS import and export, and a persisted queue (paths only).
- Entry points from Files and Preview panes: "Play", "Add to queue", "Play folder", drag and drop of files onto the pane, and "Play in Media pane" from a Preview video card.
- Audio output with A/V sync: audio is the master clock; video frames are dropped or held to follow it. Decoding stays out of process (`ffmpeg`/`ffprobe` sidecar) except for audio-only files that `symphonia` can decode.
- Fullscreen: a fullscreen mode for video (and the cover/queue view for audio) with auto-hiding controls, Esc to leave, and a window-level fullscreen toggle.
- Audio and subtitle track selection for video, frame step while paused, and keyboard control while the pane is focused.
- Remote files (SMB/SFTP) play through a bounded local spool.
- Preview keeps simple single-file playback and gains sound: video with audio, and audio files with cover art and tags (play/pause, seek, volume, mute, no queue; audio never autoplays by default). Preview and the Media pane share one audio output (starting one stops the other) and Preview offers "Play in Media pane" / "Add to queue".
- **No breaking changes.** The "audio output is out of scope" statement is removed.

## Capabilities

### New Capabilities
- `media-pane`: the Media pane kind, playback queue and playlists, transport and track selection, audio output and A/V sync, fullscreen, remote spooling, and media commands.

### Modified Capabilities
- `pane-types`: adds the `Media` kind to `pane.kind.set` and its lifecycle rules.
- `preview-pane`: simple single-file video playback gains sound, audio files get simple playback, and both hand off to the Media pane; Preview and Media share one player.
- `app-shell`: window fullscreen toggle and its interaction with saved window geometry.
- `security`: media files, playlists and decoder subprocesses are untrusted and bounded.

## Impact

- New crate `crates/media` (L1, no egui): queue model, playlist parsing, audio sink abstraction, decoder subprocess control, A/V clock. Registered in `xtask/src/layers.rs`.
- `fm-layout`: `PaneKind::Media` and per-pane view state. `fm-engine`: `media_cmds.rs` (new module), session queue, job lane use for spooling. `fm-ui`: media pane widgets in dark and light themes (new theme tokens only, no `Color32::` literals). `fm-app`: menus, shortcuts, fullscreen handling.
- Dependencies: `symphonia` (pure Rust) and an audio output crate (`cpal`/`rodio`); Linux builds may need ALSA headers, which conflicts with the "no system libraries at build time" rule, so the design must resolve this. Possible `lofty` for tags.
- Docs: `docs/architecture.md`, `docs/roadmap.md`, specs above, `CHANGELOG.md`.
