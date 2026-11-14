## Context

Only Windows with NVIDIA's threaded OpenGL driver is known to leak, but any OpenGL driver is a risk there, and Direct3D is the native, best-supported path on Windows. egui ships a wgpu renderer behind an eframe feature, so switching is configuration, not new drawing code.

## Decisions

**D1: wgpu on Windows only.** Linux/macOS keep glow: no reported problem, smaller binaries, shorter builds (the original D6 reasoning). The feature is enabled through a `[target.'cfg(windows)'.dependencies]` entry in `fm-app`, so other platforms do not compile wgpu (snapshot tests already use it via `egui_kittest`).

**D2: Direct3D 12 only.** egui-wgpu's default backends include Vulkan and GL. GL would quietly put the window back on the OpenGL driver. Vulkan was tried first (2026-11-14, it was listed before Direct3D and egui-wgpu took it): moving the window stuttered, because a Vulkan swapchain on Windows is not composed like DXGI's flip model, and opening Vulkan loads every GPU's Vulkan driver into the process (commit started at ~400 MB instead of ~105 MB). A `native_adapter_selector` picks the Direct3D 12 adapter: dedicated GPU first, then integrated, never the WARP software rasterizer (OpenGL on the real GPU is the better fallback). It logs the adapter list. `Backends::from_env()` keeps `WGPU_BACKEND` working for troubleshooting; the selector then ranks Direct3D before Vulkan before others.

**D3: Fallback in-process.** `main` keeps the `Session` in an `Rc<RefCell<Option<…>>>`; the app creator takes it. If `run_native` returns an error and the session was not taken, the same session runs again with `Renderer::Glow` and a notice. If the app started and failed later, no retry happens (the session has been consumed).

**D4: Preference, not automatic switching.** Users who see a problem with Direct3D (drawing glitches, drivers) can pick OpenGL in Troubleshooting. The choice takes effect at the next start because the renderer cannot change under a running window.

**D5: Frame-driven video redraws.** `fm_preview::player::set_frame_waker` installs one callback (a `OnceLock`) that decoder threads call after publishing a due frame, outside the snapshot lock. The app's callback calls `request_repaint` only if `fm_ui::media::video_on_screen(ctx)`: the frame number of the last frame that drew a playing player is kept in egui's temp data, and must be the previous frame. Playing players request a 250 ms repaint for the clock; audio-only playback needs no more.

## Risks

- wgpu startup on unusual setups (remote desktop, VMs, very old GPUs): covered by D3 and D4.
- Binary size and Windows build time grow: accepted.
