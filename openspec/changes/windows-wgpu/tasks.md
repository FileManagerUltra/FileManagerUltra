## 1. Renderer

- [x] 1.1 Remove the vendored egui_glow patch (first attempt, no effect)
- [x] 1.2 `fm-app`: `eframe/wgpu` on Windows only; `renderer.rs` (`first_choice`, `has_fallback`, `configure` limiting wgpu to DX12/Vulkan, `describe`); unit test
- [x] 1.3 `main.rs`: run with the preferred renderer, retry with OpenGL and a notice if it fails to start
- [x] 1.4 `windows_renderer` preference (`direct3d`/`opengl`) with the display prefs command; round-trip and malformed-value tests
- [x] 1.5 Preferences › Advanced › Troubleshooting › Renderer row (Windows only) with "In use: …"; test
- [x] 1.6 Diagnostic log header names the renderer; wgpu targets accepted at warning level

## 2. Video redraws

- [x] 2.1 Player frame waker, called after a due frame is published; test
- [x] 2.2 Wake only while a playing player was drawn last frame; playing timer 250 ms; test

## 3. Docs and checks

- [x] 3.1 CHANGELOG, devlog, architecture note
- [ ] 3.2 Manual (Windows, NVIDIA, Threaded optimization on Auto): Preferences shows "In use: Direct3D 12 (…)"; with the diagnostic log on, commit stays flat while hovering, playing video and viewing thumbnails; drawing looks right (text, images, video, tooltips, menus, fullscreen)
- [ ] 3.3 Manual: switching to OpenGL and back applies after a restart; `WGPU_BACKEND=gl`-style failure shows the OpenGL notice
- [ ] 3.4 Manual: video plays smoothly with the frame-driven redraws on Linux and Windows; a hidden Media pane draws nothing extra
