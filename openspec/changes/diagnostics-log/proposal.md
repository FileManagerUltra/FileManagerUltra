## Why

Video playback on Windows grew memory again in the 0.2.2 installer, while the `da22642` build, which was effectively identical, did not. The source, toolchain and packaged files match, so the difference cannot be found by reading code. What's needed is a measurement from the user's machine: process and GPU memory over time next to what the app did (texture uploads, decoder restarts, cache evictions, commands). The video-frame texture also still has two paths that create a new GPU texture (a frame of a new size, and the texture cache dropping the texture when a frame was not drawn), so a texture that is reused is added behind a switch that can be compared within one session.

## What Changes

- Preferences › Advanced › Troubleshooting: **Write diagnostic log** (off by default) and **Open log folder**. While on, the app writes an owner-only, size-capped text log: a startup summary (version, build commit, OS, OpenGL vendor/renderer/version, display scale, video settings), one sample per second (working set, commit size, dedicated GPU memory, egui texture count and bytes, frames drawn, video frames and texture uploads by kind, cache evictions, live decoders, decoded frames alive and their buffer bytes, running ffmpeg/ffprobe helpers, thumbnails made vs loaded from disk and pending, preview renders, whether the Media video plays while hidden, panes, workspaces, preview slots, thumbnail memory) and events (video texture created or resized, decoder started with its width/still/hardware/subtitle settings, command ids dispatched, warnings from the graphics stack). No file names, paths, URIs, command parameters or credentials are written.
- Preferences › Advanced › Troubleshooting: **Reuse video texture** (on by default). Each video picture keeps one GPU texture of exactly its frame size, written in place, which survives a few seconds without a frame (seek, decoder restart). Off releases it at the first frame that does not draw it, as before. (A first version kept a grow-only texture rounded up to 256 px and drew a sub-rectangle; on Windows/NVIDIA OpenGL that crept ~7 MB/min during playback, so storage now matches the frame.)
- About shows the build commit for builds made by the release workflow.

## Capabilities

### New Capabilities
- `diagnostics`: the diagnostic log (switch, contents, privacy, bounds).

### Modified Capabilities
- `media-pane`: new requirement "Video frames reuse one texture".
- `security`: new requirement "Diagnostic log is private and bounded".

## Impact

New `fm-engine::diag` (global, lock-free when off; a writer thread does all file I/O and sampling, so the UI thread never blocks), `prefs.set` keys `diagnostics_log` and `video_texture_reuse`, command `app.diagnostics.folder`. Hooks in `Session::dispatch`, the video decoder pump, `fm-ui` video texture upload and frame-cache sweep, and the app frame loop (once-per-second UI sample). `fm-preview::player::live_players()` counts running decoders. No new dependency: `log` and `sysinfo` are already in the workspace.
