use eframe::egui;
use fm_engine::prefs::{Prefs, WindowGeometry};

pub fn capture(ctx: &egui::Context, prefs: &mut Prefs) {
    let viewport = ctx.input(|i| i.viewport().clone());
    capture_geometry(&viewport, ctx.zoom_factor(), ctx.pixels_per_point(), prefs);
}

fn capture_geometry(
    viewport: &egui::ViewportInfo,
    zoom: f32,
    pixels_per_point: f32,
    prefs: &mut Prefs,
) {
    if viewport.minimized == Some(true) {
        return;
    }
    // Fullscreen never replaces the normal geometry or the maximized flag.
    if viewport.fullscreen == Some(true) {
        return;
    }
    if let Some(maximized) = viewport.maximized {
        prefs.window_maximized = maximized;
    }
    if viewport.maximized.unwrap_or(prefs.window_maximized) {
        return;
    }
    let Some(rect) = viewport.inner_rect else {
        return;
    };
    let geometry = WindowGeometry {
        size: [rect.width() * zoom, rect.height() * zoom],
        position: viewport
            .outer_rect
            .map(|r| [r.min.x * pixels_per_point, r.min.y * pixels_per_point]),
        scale_factor: viewport
            .native_pixels_per_point
            .unwrap_or(pixels_per_point / zoom),
    };
    if geometry.is_valid() {
        prefs.window_geometry = Some(geometry);
    }
}

pub fn restore(options: &mut eframe::NativeOptions, prefs: &Prefs) {
    options.viewport = options
        .viewport
        .clone()
        .with_maximized(prefs.window_maximized);
    let Some(geometry) = prefs.window_geometry.filter(WindowGeometry::is_valid) else {
        return;
    };
    options.viewport = options.viewport.clone().with_inner_size(egui::vec2(
        geometry.size[0].max(640.0),
        geometry.size[1].max(400.0),
    ));
}

/// Applies the saved geometry to the freshly created window. Returns the restored physical
/// position, which [`Tracker`] re-sends once the window is mapped: window managers commonly
/// place a new window themselves and ignore a position set before it appears.
pub fn restore_native(window: &winit::window::Window, prefs: &Prefs) -> Option<[f32; 2]> {
    let geometry = prefs.window_geometry.filter(WindowGeometry::is_valid)?;
    let monitors: Vec<_> = window.available_monitors().collect();
    let rectangles: Vec<_> = monitors
        .iter()
        .map(|monitor| {
            let pos = monitor.position();
            let size = monitor.size();
            egui::Rect::from_min_size(
                egui::pos2(pos.x as f32, pos.y as f32),
                egui::vec2(size.width as f32, size.height as f32),
            )
        })
        .collect();
    let position = visible_position(geometry.position, &rectangles);
    if let Some(position) = position {
        if cfg!(target_os = "macos") {
            window.set_outer_position(winit::dpi::LogicalPosition::new(
                position[0] / geometry.scale_factor,
                position[1] / geometry.scale_factor,
            ));
        } else {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(position[0], position[1]));
        }
    }
    let largest = monitors
        .iter()
        .map(|monitor| {
            let size = monitor.size();
            let scale = monitor.scale_factor() as f32;
            egui::vec2(size.width as f32 / scale, size.height as f32 / scale)
        })
        .reduce(egui::Vec2::max);
    if let Some(largest) = largest {
        let _ = window.request_inner_size(winit::dpi::LogicalSize::new(
            geometry.size[0].max(640.0).min(largest.x),
            geometry.size[1].max(400.0).min(largest.y),
        ));
    }
    window.set_maximized(prefs.window_maximized);
    position.filter(|_| !prefs.window_maximized && !cfg!(target_os = "macos"))
}

/// How long a restored position is re-sent until the window manager honours it.
const POSITION_WINDOW: std::time::Duration = std::time::Duration::from_millis(1500);
/// Preference changes (window geometry, bookmarks, profiles…) are written to disk once they
/// have been stable this long, so a killed process or ended session keeps them.
const SAVE_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Default)]
pub struct Tracker {
    position: Option<[f32; 2]>,
    started: Option<std::time::Instant>,
    saved: Prefs,
    changed_at: Option<std::time::Instant>,
}

impl Tracker {
    pub fn new(position: Option<[f32; 2]>, prefs: &Prefs) -> Self {
        Self {
            position,
            started: None,
            saved: prefs.clone(),
            changed_at: None,
        }
    }

    /// Re-send the restored position while the window manager is still placing the window.
    /// The first frames may report the requested position before the window manager moves
    /// the window, so this keeps correcting until [`POSITION_WINDOW`] has passed.
    pub fn reposition(&mut self, ctx: &egui::Context) {
        let Some(target) = self.position else {
            return;
        };
        let started = *self.started.get_or_insert_with(std::time::Instant::now);
        if started.elapsed() >= POSITION_WINDOW {
            self.position = None;
            return;
        }
        let ppp = ctx.pixels_per_point();
        let current = ctx.input(|i| i.viewport().outer_rect).map(|r| r.min * ppp);
        let off = current
            .is_none_or(|c| (c.x - target[0]).abs() >= 2.0 || (c.y - target[1]).abs() >= 2.0);
        if off {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                target[0] / ppp,
                target[1] / ppp,
            )));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// True once the preferences (including the captured geometry) changed and then stayed
    /// put for [`SAVE_DELAY`].
    pub fn save_due(&mut self, ctx: &egui::Context, prefs: &Prefs) -> bool {
        if self.position.is_some() {
            return false;
        }
        if *prefs == self.saved {
            self.changed_at = None;
            return false;
        }
        let now = std::time::Instant::now();
        let since = *self.changed_at.get_or_insert(now);
        if now.duration_since(since) < SAVE_DELAY {
            ctx.request_repaint_after(SAVE_DELAY);
            return false;
        }
        self.saved = prefs.clone();
        self.changed_at = None;
        true
    }
}

fn visible_position(position: Option<[f32; 2]>, monitors: &[egui::Rect]) -> Option<[f32; 2]> {
    position.filter(|pos| {
        monitors
            .iter()
            .any(|monitor| monitor.contains(egui::pos2(pos[0], pos[1])))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_capture_ignores_zoom_and_preserves_normal_bounds_when_maximized() {
        let mut prefs = Prefs::default();
        let mut viewport = egui::ViewportInfo {
            inner_rect: Some(egui::Rect::from_min_size(
                egui::pos2(60.0, 40.0),
                egui::vec2(600.0, 400.0),
            )),
            outer_rect: Some(egui::Rect::from_min_size(
                egui::pos2(-300.0, 25.0),
                egui::vec2(600.0, 420.0),
            )),
            native_pixels_per_point: Some(2.0),
            maximized: Some(false),
            ..Default::default()
        };
        capture_geometry(&viewport, 1.5, 3.0, &mut prefs);
        let saved = prefs.window_geometry.expect("geometry");
        assert_eq!(saved.size, [900.0, 600.0]);
        assert_eq!(saved.position, Some([-900.0, 75.0]));
        assert_eq!(saved.scale_factor, 2.0);
        viewport.maximized = Some(true);
        viewport.inner_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1920.0, 1080.0),
        ));
        capture_geometry(&viewport, 1.0, 2.0, &mut prefs);
        assert!(prefs.window_maximized);
        assert_eq!(prefs.window_geometry, Some(saved));
        viewport.minimized = Some(true);
        viewport.maximized = Some(false);
        capture_geometry(&viewport, 1.0, 2.0, &mut prefs);
        assert!(prefs.window_maximized);
        assert_eq!(prefs.window_geometry, Some(saved));
    }

    #[test]
    fn fullscreen_changes_neither_geometry_nor_the_maximized_flag() {
        let mut prefs = Prefs {
            window_maximized: false,
            window_geometry: Some(WindowGeometry {
                size: [900.0, 600.0],
                position: Some([10.0, 20.0]),
                scale_factor: 1.0,
            }),
            ..Prefs::default()
        };
        let viewport = egui::ViewportInfo {
            inner_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1920.0, 1080.0),
            )),
            maximized: Some(true),
            fullscreen: Some(true),
            ..Default::default()
        };
        let before = prefs.window_geometry;
        capture_geometry(&viewport, 1.0, 1.0, &mut prefs);
        assert!(!prefs.window_maximized);
        assert_eq!(prefs.window_geometry, before);
    }

    #[test]
    fn tracker_saves_geometry_once_it_settles_but_not_while_repositioning() {
        let ctx = egui::Context::default();
        let mut prefs = Prefs::default();
        let mut tracker = Tracker::new(None, &prefs);
        assert!(!tracker.save_due(&ctx, &prefs));
        prefs.window_geometry = Some(WindowGeometry {
            size: [900.0, 600.0],
            position: Some([10.0, 20.0]),
            scale_factor: 1.0,
        });
        assert!(
            !tracker.save_due(&ctx, &prefs),
            "saved before the move settled"
        );
        tracker.changed_at = std::time::Instant::now().checked_sub(SAVE_DELAY);
        assert!(tracker.save_due(&ctx, &prefs));
        assert!(!tracker.save_due(&ctx, &prefs), "saved twice");
        let mut placing = Tracker::new(Some([10.0, 20.0]), &Prefs::default());
        placing.changed_at = std::time::Instant::now().checked_sub(SAVE_DELAY);
        assert!(
            !placing.save_due(&ctx, &prefs),
            "window manager placement was saved"
        );
    }

    #[test]
    fn restored_geometry_handles_missing_monitors_and_wayland_position() {
        let monitors = [
            egui::Rect::from_min_size(egui::pos2(-1920.0, 0.0), egui::vec2(1920.0, 1080.0)),
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0)),
        ];
        assert_eq!(
            visible_position(Some([-900.0, 75.0]), &monitors),
            Some([-900.0, 75.0])
        );
        assert_eq!(visible_position(Some([-900.0, 75.0]), &monitors[1..]), None);
        assert_eq!(visible_position(None, &monitors), None);
        let mut prefs = Prefs {
            window_geometry: Some(WindowGeometry {
                size: [900.0, 600.0],
                position: None,
                scale_factor: 1.0,
            }),
            window_maximized: true,
            ..Default::default()
        };
        let mut options = eframe::NativeOptions::default();
        restore(&mut options, &prefs);
        assert_eq!(options.viewport.inner_size, Some(egui::vec2(900.0, 600.0)));
        assert_eq!(options.viewport.maximized, Some(true));
        prefs.window_geometry.as_mut().expect("geometry").size[0] = f32::NAN;
        let mut defaults = eframe::NativeOptions::default();
        restore(&mut defaults, &prefs);
        assert!(defaults.viewport.inner_size.is_none());
    }
}
