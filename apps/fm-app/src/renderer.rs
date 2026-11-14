//! Which renderer draws the window (change `windows-wgpu`).
//!
//! Windows uses wgpu on Direct3D 12 (or Vulkan): NVIDIA's OpenGL driver with "Threaded
//! optimization" kept memory for every drawn frame. OpenGL stays available there as a
//! preference and as the fallback when wgpu cannot start. Other platforms use OpenGL.

use fm_engine::prefs::Prefs;

/// The renderer to try first for these preferences.
pub fn first_choice(prefs: &Prefs) -> eframe::Renderer {
    #[cfg(windows)]
    if prefs.windows_renderer == fm_engine::prefs::WindowsRenderer::Direct3d {
        return eframe::Renderer::Wgpu;
    }
    let _ = prefs;
    eframe::Renderer::Glow
}

/// Whether `renderer` failing to start should be retried with OpenGL.
pub fn has_fallback(renderer: eframe::Renderer) -> bool {
    renderer != eframe::Renderer::Glow
}

/// Set `options` up for `renderer`.
pub fn configure(options: &mut eframe::NativeOptions, renderer: eframe::Renderer) {
    options.renderer = renderer;
    #[cfg(windows)]
    {
        use eframe::egui_wgpu::{WgpuSetup, wgpu};
        // Direct3D 12 or Vulkan only: wgpu's default also allows its OpenGL backend, which
        // would put the window back on the OpenGL driver. `WGPU_BACKEND` still overrides.
        if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            create.instance_descriptor.backends =
                wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12 | wgpu::Backends::VULKAN);
        }
    }
}

/// A short description of what draws the window, for Preferences and the diagnostic log:
/// "Direct3D 12 (NVIDIA GeForce RTX 4090)" or "OpenGL".
pub fn describe(cc: &eframe::CreationContext<'_>) -> String {
    #[cfg(windows)]
    if let Some(state) = &cc.wgpu_render_state {
        let info = state.adapter.get_info();
        let backend = match info.backend {
            eframe::egui_wgpu::wgpu::Backend::Dx12 => "Direct3D 12",
            eframe::egui_wgpu::wgpu::Backend::Vulkan => "Vulkan",
            eframe::egui_wgpu::wgpu::Backend::Gl => "OpenGL (wgpu)",
            _ => "wgpu",
        };
        return format!("{backend} ({})", info.name.trim());
    }
    let _ = cc;
    "OpenGL".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_platforms_draw_with_opengl_and_windows_prefers_direct3d() {
        let prefs = Prefs::default();
        let first = first_choice(&prefs);
        if cfg!(windows) {
            assert!(has_fallback(first), "Direct3D first, OpenGL as fallback");
        } else {
            assert_eq!(first, eframe::Renderer::Glow);
            assert!(!has_fallback(first));
        }
    }
}
