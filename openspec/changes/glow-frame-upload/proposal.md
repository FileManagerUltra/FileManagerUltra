## Why

On Windows with an NVIDIA GPU the app's memory grew by about 25 KB for every frame it drew (1 MB a second while playing video or with the mouse over the window; 18 GB of commit in one report), and stopped completely while minimized. The diagnostic log (change `diagnostics-log`) showed egui's own memory and textures flat; turning off NVIDIA's "Threaded optimization" for the app stopped the growth. The cause is egui_glow calling `glBufferData` for every mesh of every frame, which that driver does not give back. Users should not have to change driver settings.

## What Changes

- egui_glow is vendored (`vendor/egui_glow`, `[patch.crates-io]`) with one local change in `src/painter.rs`: each frame's meshes go into one vertex and one index buffer (indices rebased), written with `glBufferSubData` into storage that only grows, and each mesh is drawn as a range of that buffer.
- `EGUI_GLOW_UPLOAD=per-mesh|frame|frame-subdata` selects upstream behaviour, one `glBufferData` per frame, or the new default, so the strategies can be compared on an affected machine; the choice appears in the diagnostic log.

## Capabilities

### Modified Capabilities
- `app-shell`: new requirement "Drawing does not grow memory".

## Impact

`vendor/egui_glow` (copy of 0.37.0 plus the painter change), root `Cargo.toml` patch entry, `Cargo.lock`, `vendor/FM-VENDORED.md`. Rendering on every platform goes through the changed painter. No workspace code and no `unsafe` in workspace crates change.
