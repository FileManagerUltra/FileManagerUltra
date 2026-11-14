//! Which renderer draws the window (change `windows-wgpu`).
//!
//! Windows uses wgpu on Direct3D 12: NVIDIA's OpenGL driver with "Threaded optimization"
//! kept memory for every drawn frame. Vulkan is not offered there: its swapchain is not
//! composed like Direct3D's flip model, so moving the window stutters, and opening it loads
//! every GPU's Vulkan driver into the process (several hundred MB). OpenGL stays available
//! as a preference and as the fallback when Direct3D cannot start. Other platforms use
//! OpenGL.

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
        if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            // Direct3D 12 only (wgpu's default also opens Vulkan and its own OpenGL backend).
            // `WGPU_BACKEND` still overrides for troubleshooting.
            create.instance_descriptor.backends =
                wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
            create.native_adapter_selector = Some(std::sync::Arc::new(windows::pick_adapter));
        }
    }
}

/// A short description of what draws the window, for Preferences and the diagnostic log:
/// "Direct3D 12 (NVIDIA GeForce RTX 4090)" or "OpenGL".
pub fn describe(cc: &eframe::CreationContext<'_>) -> String {
    #[cfg(windows)]
    if let Some(state) = &cc.wgpu_render_state {
        let info = state.adapter.get_info();
        return format!(
            "{} ({})",
            windows::backend_name(info.backend),
            info.name.trim()
        );
    }
    let _ = cc;
    "OpenGL".to_string()
}

// Used by the Windows adapter choice; elsewhere only by tests.
#[cfg_attr(not(windows), allow(dead_code))]
/// Graphics API of a wgpu adapter, as far as choosing one is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Api {
    Direct3d12,
    Vulkan,
    Other,
}

// Used by the Windows adapter choice; elsewhere only by tests.
#[cfg_attr(not(windows), allow(dead_code))]
/// Kind of device behind a wgpu adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Device {
    Discrete,
    Integrated,
    Virtual,
    /// A software rasterizer (Microsoft WARP): far slower than OpenGL on the real GPU.
    Software,
    Unknown,
}

// Used by the Windows adapter choice; elsewhere only by tests.
#[cfg_attr(not(windows), allow(dead_code))]
/// Lower is better; `None` means "never": software adapters, so the window falls back to
/// OpenGL on the real GPU instead.
fn rank(api: Api, device: Device) -> Option<u8> {
    let api = match api {
        Api::Direct3d12 => 0,
        Api::Vulkan => 1,
        Api::Other => 2,
    };
    let device = match device {
        Device::Discrete => 0,
        Device::Integrated => 1,
        Device::Virtual | Device::Unknown => 2,
        Device::Software => return None,
    };
    Some(api * 3 + device)
}

#[cfg(windows)]
mod windows {
    use super::{Api, Device, rank};
    use eframe::egui_wgpu::wgpu;

    pub fn backend_name(backend: wgpu::Backend) -> &'static str {
        match backend {
            wgpu::Backend::Dx12 => "Direct3D 12",
            wgpu::Backend::Vulkan => "Vulkan",
            wgpu::Backend::Gl => "OpenGL (wgpu)",
            _ => "wgpu",
        }
    }

    fn api(backend: wgpu::Backend) -> Api {
        match backend {
            wgpu::Backend::Dx12 => Api::Direct3d12,
            wgpu::Backend::Vulkan => Api::Vulkan,
            _ => Api::Other,
        }
    }

    fn device(kind: wgpu::DeviceType) -> Device {
        match kind {
            wgpu::DeviceType::DiscreteGpu => Device::Discrete,
            wgpu::DeviceType::IntegratedGpu => Device::Integrated,
            wgpu::DeviceType::VirtualGpu => Device::Virtual,
            wgpu::DeviceType::Cpu => Device::Software,
            wgpu::DeviceType::Other => Device::Unknown,
        }
    }

    /// The dedicated GPU on Direct3D 12 if there is one, never a software adapter.
    pub fn pick_adapter(
        adapters: &[wgpu::Adapter],
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<wgpu::Adapter, String> {
        fm_engine::diag::event("renderer", || {
            let list: Vec<String> = adapters
                .iter()
                .map(|a| {
                    let info = a.get_info();
                    format!(
                        "{} {:?} {}",
                        backend_name(info.backend),
                        info.device_type,
                        info.name.trim()
                    )
                })
                .collect();
            format!("adapters: {}", list.join("; "))
        });
        let mut best: Option<(u8, &wgpu::Adapter)> = None;
        for adapter in adapters {
            if surface.is_some_and(|s| !adapter.is_surface_supported(s)) {
                continue;
            }
            let info = adapter.get_info();
            let Some(score) = rank(api(info.backend), device(info.device_type)) else {
                continue;
            };
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, adapter));
            }
        }
        best.map(|(_, a)| a.clone())
            .ok_or_else(|| "no Direct3D 12 graphics card found".to_string())
    }
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

    #[test]
    fn the_dedicated_gpu_on_direct3d_wins_and_software_never_does() {
        let d3d_discrete = rank(Api::Direct3d12, Device::Discrete);
        let d3d_integrated = rank(Api::Direct3d12, Device::Integrated);
        let vulkan_discrete = rank(Api::Vulkan, Device::Discrete);
        assert!(d3d_discrete < d3d_integrated);
        // Direct3D on the integrated GPU still beats Vulkan (only offered via WGPU_BACKEND).
        assert!(d3d_integrated < vulkan_discrete);
        assert!(d3d_discrete.is_some() && vulkan_discrete.is_some());
        assert_eq!(rank(Api::Direct3d12, Device::Software), None, "WARP");
        assert!(rank(Api::Other, Device::Unknown).is_some());
    }
}
