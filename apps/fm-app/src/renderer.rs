//! Which renderer draws the window (change `windows-wgpu`).
//!
//! Windows uses wgpu on Direct3D 12: NVIDIA's OpenGL driver with "Threaded optimization"
//! kept memory for every drawn frame. Vulkan is not offered there: its swapchain is not
//! composed like Direct3D's flip model, so moving the window stutters, and opening it loads
//! every GPU's Vulkan driver into the process (several hundred MB). OpenGL stays available
//! as a preference and as the fallback when Direct3D cannot start; Direct3D's software
//! rasterizer is the last resort, for machines without a usable graphics card. Other
//! platforms use OpenGL.

use fm_engine::prefs::{Prefs, RendererFallback};

/// One way of drawing the window, tried in the order of [`attempts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attempt {
    /// wgpu on Direct3D 12 with a real graphics card.
    Direct3d,
    /// egui's OpenGL renderer.
    OpenGl,
    /// wgpu on Direct3D 12's software rasterizer (WARP): slow, but needs no graphics card.
    Software,
}

/// The renderers to try, best first, for these preferences on this platform.
pub fn attempts(prefs: &Prefs) -> &'static [Attempt] {
    if !cfg!(windows) {
        return &[Attempt::OpenGl];
    }
    match prefs.windows_renderer {
        fm_engine::prefs::WindowsRenderer::Direct3d => {
            &[Attempt::Direct3d, Attempt::OpenGl, Attempt::Software]
        }
        fm_engine::prefs::WindowsRenderer::Opengl => {
            &[Attempt::OpenGl, Attempt::Direct3d, Attempt::Software]
        }
    }
}

/// Remember which fallback drew the window (`None`: the first choice worked) and return the
/// notice to show, only when that differs from last start: someone on an old PC hears about
/// it once, and again only if things change. `failures` are the errors of the attempts that
/// did not start.
pub fn note_fallback(
    prefs: &mut Prefs,
    used: Attempt,
    first: Attempt,
    failures: &[String],
) -> Option<String> {
    let fallback = (used != first).then_some(match used {
        Attempt::Direct3d => RendererFallback::Direct3d,
        Attempt::OpenGl => RendererFallback::Opengl,
        Attempt::Software => RendererFallback::Software,
    });
    if prefs.windows_renderer_fallback == fallback {
        return None;
    }
    prefs.windows_renderer_fallback = fallback;
    let why = failures.join("; ");
    match fallback? {
        RendererFallback::Opengl => Some(format!(
            "Direct3D could not start ({why}), so the window uses OpenGL. \
             Preferences > Advanced > Troubleshooting shows the renderer in use."
        )),
        RendererFallback::Direct3d => Some(format!(
            "OpenGL could not start ({why}), so the window uses Direct3D."
        )),
        RendererFallback::Software => Some(format!(
            "The graphics card could not be used ({why}), so the window is drawn in software, \
             which is slower. Updating the graphics driver may help."
        )),
    }
}

/// Set `options` up for `attempt`.
pub fn configure(options: &mut eframe::NativeOptions, attempt: Attempt) {
    options.renderer = match attempt {
        #[cfg(windows)]
        Attempt::Direct3d | Attempt::Software => eframe::Renderer::Wgpu,
        // Only Windows builds have wgpu, and only Windows lists these attempts.
        #[cfg(not(windows))]
        Attempt::Direct3d | Attempt::Software => eframe::Renderer::Glow,
        Attempt::OpenGl => eframe::Renderer::Glow,
    };
    #[cfg(windows)]
    {
        use eframe::egui_wgpu::{WgpuSetup, wgpu};
        if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            // Direct3D 12 only (wgpu's default also opens Vulkan and its own OpenGL backend).
            // `WGPU_BACKEND` still overrides for troubleshooting.
            create.instance_descriptor.backends =
                wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
            let software = attempt == Attempt::Software;
            create.native_adapter_selector = Some(std::sync::Arc::new(move |adapters, surface| {
                windows::pick_adapter(adapters, surface, software)
            }));
            // A file manager draws little: small memory blocks instead of wgpu's 64-256 MB
            // ones, part of which is system memory and counts toward the app's commit.
            let base = std::sync::Arc::clone(&create.device_descriptor);
            create.device_descriptor = std::sync::Arc::new(move |adapter| wgpu::DeviceDescriptor {
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..base(adapter)
            });
        }
    }
}

/// A short description of what draws the window, for Preferences and the diagnostic log:
/// "Direct3D 12 (NVIDIA GeForce RTX 4090)" or "OpenGL".
pub fn describe(cc: &eframe::CreationContext<'_>) -> String {
    #[cfg(windows)]
    if let Some(state) = &cc.wgpu_render_state {
        let info = state.adapter.get_info();
        let software = if info.device_type == eframe::egui_wgpu::wgpu::DeviceType::Cpu {
            ", software"
        } else {
            ""
        };
        return format!(
            "{} ({}{software})",
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
/// Lower is better; `None` means "never". Software adapters only count when `software` is
/// set (the last attempt), and then only software adapters do: the hardware ones already
/// failed.
fn rank(api: Api, device: Device, software: bool) -> Option<u8> {
    let api = match api {
        Api::Direct3d12 => 0,
        Api::Vulkan => 1,
        Api::Other => 2,
    };
    let device = match (device, software) {
        (Device::Software, true) => 0,
        (_, true) | (Device::Software, false) => return None,
        (Device::Discrete, false) => 0,
        (Device::Integrated, false) => 1,
        (Device::Virtual | Device::Unknown, false) => 2,
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

    /// The dedicated GPU on Direct3D 12 if there is one; the software adapter only when
    /// `software` is set.
    pub fn pick_adapter(
        adapters: &[wgpu::Adapter],
        surface: Option<&wgpu::Surface<'_>>,
        software: bool,
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
            let Some(score) = rank(api(info.backend), device(info.device_type), software) else {
                continue;
            };
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, adapter));
            }
        }
        best.map(|(_, a)| a.clone()).ok_or_else(|| {
            if software {
                "no Direct3D 12 software renderer".to_string()
            } else {
                "no Direct3D 12 graphics card found".to_string()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_platforms_draw_with_opengl_and_windows_tries_direct3d_opengl_software() {
        let prefs = Prefs::default();
        if cfg!(windows) {
            assert_eq!(
                attempts(&prefs),
                [Attempt::Direct3d, Attempt::OpenGl, Attempt::Software]
            );
            let opengl = Prefs {
                windows_renderer: fm_engine::prefs::WindowsRenderer::Opengl,
                ..Prefs::default()
            };
            assert_eq!(attempts(&opengl).first(), Some(&Attempt::OpenGl));
        } else {
            assert_eq!(attempts(&prefs), [Attempt::OpenGl]);
        }
    }

    #[test]
    fn the_dedicated_gpu_on_direct3d_wins_and_software_only_comes_last() {
        let d3d_discrete = rank(Api::Direct3d12, Device::Discrete, false);
        let d3d_integrated = rank(Api::Direct3d12, Device::Integrated, false);
        let vulkan_discrete = rank(Api::Vulkan, Device::Discrete, false);
        assert!(d3d_discrete < d3d_integrated);
        // Direct3D on the integrated GPU still beats Vulkan (only offered via WGPU_BACKEND).
        assert!(d3d_integrated < vulkan_discrete);
        assert!(d3d_discrete.is_some() && vulkan_discrete.is_some());
        assert_eq!(rank(Api::Direct3d12, Device::Software, false), None, "WARP");
        assert!(rank(Api::Other, Device::Unknown, false).is_some());
        // The last attempt takes only the software rasterizer.
        assert!(rank(Api::Direct3d12, Device::Software, true).is_some());
        assert_eq!(rank(Api::Direct3d12, Device::Discrete, true), None);
    }

    #[test]
    fn a_fallback_is_announced_once_and_again_only_when_it_changes() {
        let mut prefs = Prefs::default();
        let failed = ["no Direct3D 12 graphics card found".to_string()];
        let first = Attempt::Direct3d;
        let notice = note_fallback(&mut prefs, Attempt::OpenGl, first, &failed);
        assert!(notice.as_deref().is_some_and(
            |n| n.contains("uses OpenGL") && n.contains("no Direct3D 12 graphics card found")
        ));
        assert_eq!(
            prefs.windows_renderer_fallback,
            Some(RendererFallback::Opengl)
        );
        assert_eq!(
            note_fallback(&mut prefs, Attempt::OpenGl, first, &failed),
            None,
            "once"
        );
        let worse = note_fallback(&mut prefs, Attempt::Software, first, &failed);
        assert!(worse.as_deref().is_some_and(|n| n.contains("software")));
        // Direct3D works again: nothing to say, and a later failure is announced again.
        assert_eq!(
            note_fallback(&mut prefs, Attempt::Direct3d, first, &[]),
            None
        );
        assert_eq!(prefs.windows_renderer_fallback, None);
        assert!(note_fallback(&mut prefs, Attempt::OpenGl, first, &failed).is_some());
    }
}
