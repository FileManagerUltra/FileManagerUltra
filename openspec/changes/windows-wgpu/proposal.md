## Why

On Windows with an NVIDIA GPU the app's memory grew by about 25 KB for every frame it drew (1 MB a second while playing video or with the mouse over the window; 18 GB of commit in one report), and stopped completely while minimized. The diagnostic log (change `diagnostics-log`) showed egui's own memory and textures flat; turning off NVIDIA's "Threaded optimization" for the app stopped the growth, so the memory is kept by the OpenGL driver. A patched egui_glow that uploads each frame's geometry once with `glBufferSubData` did not help (tested 2026-11-14), so the leak is in that driver's per-frame path, not in how buffers are filled. Users should not have to change driver settings.

## What Changes

- **Windows draws with Direct3D 12** through egui's wgpu renderer (`eframe` feature `wgpu`, Windows only). wgpu is limited to Direct3D 12 and Vulkan so it never falls back to its own OpenGL backend; `WGPU_BACKEND` still overrides for troubleshooting. Linux and macOS keep OpenGL (design D6 of `scaffold-workspace` stays for them).
- **Renderer preference** (Windows only): Preferences › Advanced › Troubleshooting › *Renderer*: Direct3D (default) or OpenGL, applied at the next start, with an "In use: …" line naming the backend and GPU. Stored as `windows_renderer` in `prefs.toml`.
- **Fallback:** if Direct3D cannot start (no adapter, old driver, remote session), the window opens with OpenGL instead and a notice says so. eframe allows a second `run_native` after a failed one, so this happens in the same process.
- **Video draws at its own frame rate:** the decoder wakes the UI when a new frame is ready, but only while a playing player was drawn in the previous frame (a hidden pane or a minimized window draws nothing). The UI's own playing timer drops from 16 ms to 250 ms and only moves the clock and seek bar. This halves the frames drawn for a 25 fps video and helps every renderer.
- The renderer in use is written in the diagnostic log header; the log also accepts wgpu's warnings.
- The vendored egui_glow from the first attempt is removed.

## Capabilities

### Modified Capabilities
- `app-shell`: new requirements "Drawing does not grow memory" and "Renderer choice on Windows".
- `media-pane`: new requirement "Video redraws at its frame rate".

## Impact

`apps/fm-app` (`renderer.rs`, startup and fallback in `main.rs`, Windows-only `eframe/wgpu` dependency), `fm-engine` prefs (`windows_renderer`), display prefs command, diagnostics header, `fm-preview` player (frame waker), `fm-ui` Preferences row and playing redraws. Windows binaries grow by the wgpu/naga code. No `unsafe` in workspace code.
