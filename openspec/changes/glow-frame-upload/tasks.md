## 1. Implementation

- [x] 1.1 Vendor egui_glow 0.37.0 unchanged, add the `[patch.crates-io]` entry, document it in `vendor/FM-VENDORED.md`
- [x] 1.2 `painter.rs`: gather the frame's meshes into one vertex/index buffer (indices rebased), grow-only storage written with `glBufferSubData`, draw each mesh as a range; `EGUI_GLOW_UPLOAD` override; strategy logged at debug level
- [x] 1.3 CHANGELOG, devlog
- [ ] 1.4 Manual (Windows, NVIDIA, Threaded optimization back on Auto): with the diagnostic log on, commit stays flat while hovering, playing video and viewing thumbnails; drawing looks right (text, images, video, tooltips, menus)
- [ ] 1.5 Manual: drawing looks right on Linux (X11/Wayland) and macOS
- [ ] 1.6 Offer the change upstream (egui_glow); drop the patch when released
