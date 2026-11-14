## Context

Only Windows with NVIDIA's threaded OpenGL driver is known to leak, but any OpenGL driver is a risk there, and Direct3D is the native, best-supported path on Windows. egui ships a wgpu renderer behind an eframe feature, so switching is configuration, not new drawing code.

## Decisions

**D1: wgpu on Windows only.** Linux/macOS keep glow: no reported problem, smaller binaries, shorter builds (the original D6 reasoning). The feature is enabled through a `[target.'cfg(windows)'.dependencies]` entry in `fm-app`, so other platforms do not compile wgpu (snapshot tests already use it via `egui_kittest`).

**D2: Direct3D 12 only.** egui-wgpu's default backends include Vulkan and GL. GL would quietly put the window back on the OpenGL driver. Vulkan was tried first (2026-11-14, it was listed before Direct3D and egui-wgpu took it): moving the window stuttered, because a Vulkan swapchain on Windows is not composed like DXGI's flip model, and opening Vulkan loads every GPU's Vulkan driver into the process (commit started at ~400 MB instead of ~105 MB). A `native_adapter_selector` picks the Direct3D 12 adapter: dedicated GPU first, then integrated, never the WARP software rasterizer (OpenGL on the real GPU is the better fallback). It logs the adapter list. `Backends::from_env()` keeps `WGPU_BACKEND` working for troubleshooting; the selector then ranks Direct3D before Vulkan before others.

**D3: Fallback chain in-process.** `renderer::attempts` lists Direct3D (hardware) → OpenGL → Direct3D software (WARP) when Direct3D is preferred, OpenGL → Direct3D → software when OpenGL is. `main` keeps the `Session` in an `Rc<RefCell<Option<…>>>`; the app creator takes it. If `run_native` returns an error and the session was not taken, the next attempt runs with the same session. If the app started and failed later, no retry happens (the session has been consumed). WARP comes last because OpenGL on the real GPU is faster, and Windows' own OpenGL without a GPU driver (1.1) is too old for egui, so a VM without 3D acceleration only opens with WARP.

**D3a: Announce a fallback once.** `Prefs::windows_renderer_fallback` remembers which attempt drew the window when the first choice failed (`None` when it worked); `note_fallback` returns a notice only when that changes, and the app creator saves prefs immediately. An old PC hears about OpenGL once instead of at every start, Direct3D is still tried at every start (a temporary failure does not switch anyone for good), and a later change (worse: software; better: back to Direct3D, then failing again) is announced.

**D3b: Memory hints.** egui-wgpu asks for `MemoryHints::Performance`, which makes wgpu's allocator reserve 128-256 MB device blocks and 64-128 MB host (system memory, counted in commit) blocks. The device descriptor is wrapped to ask for `MemoryUsage` (8-64 MB / 4-32 MB): a file manager's buffers and textures are small.

**D3c: One queued frame.** egui-winit repaints on every `WindowEvent::Moved`, and eframe's wgpu default (`SurfaceConfig::HIGH_THROUGHPUT`) lets two frames queue behind vsync, so on Direct3D the window trailed the mouse while dragged (OpenGL felt smooth because NVIDIA's threaded driver never waited, which is the same queue that leaked). `configure` uses egui's low-latency setting: vsync, `desired_maximum_frame_latency: Some(1)`. The `FM_PRESENT` environment variable (`mailbox`, `immediate`) tries non-waiting present modes for troubleshooting; the renderer line in Preferences and the log names an override.

**D3d: No repaint on window moves (Windows).** One queued frame and even `FM_PRESENT=mailbox` only helped a little (user test, 2026-11-14): with the flip model, each frame presented while the window moves ties that step of the move to the frame, whereas a move without a new frame is done by the compositor alone. egui-winit asks for a repaint on every `WindowEvent::Moved`, and eframe has no event filter, so egui-winit 0.37.0 is vendored (`vendor/FM-VENDORED.md`) with `Moved` not repainting on Windows. The window's position is read at every frame, and the app draws at least once a second, so the saved geometry stays current. `EGUI_WINIT_MOVE_REPAINT=1` restores upstream behaviour for comparison.

**D3e: Nothing draws while idle.** The log of 2026-11-14 21:06 showed the drag itself drew no extra frames any more (all `moved` frames came from the idle rate): the device scan, every 2 s while focused, polled for its result every 100 ms, about 5 frames each time. The scan thread now wakes the UI when its result is ready (`Devices::set_waker`, like the file watcher); a 1 s fallback keeps the slow-scan warning on time.

**D4: Preference, not automatic switching.** Users who see a problem with Direct3D (drawing glitches, drivers) can pick OpenGL in Troubleshooting. The choice takes effect at the next start because the renderer cannot change under a running window.

**D5: Frame-driven video redraws.** `fm_preview::player::set_frame_waker` installs one callback (a `OnceLock`) that decoder threads call after publishing a due frame, outside the snapshot lock. The app's callback calls `request_repaint` only if `fm_ui::media::video_on_screen(ctx)`: the frame number of the last frame that drew a playing player is kept in egui's temp data, and must be the previous frame. Playing players request a 250 ms repaint for the clock; audio-only playback needs no more.

## Risks

- wgpu startup on unusual setups (remote desktop, VMs, very old GPUs): covered by D3 and D4.
- Binary size and Windows build time grow: accepted.
