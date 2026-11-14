//! A loading spinner that redraws about 15 times a second. egui's own spinner asks for a
//! repaint every frame, which keeps the whole window drawing at the display's refresh rate
//! (120 frames a second on a 120 Hz screen) for as long as a folder scan or a load runs.

use egui::{Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Widget, emath::lerp, vec2};
use std::time::Duration;

/// Time between spinner frames.
pub const TICK: Duration = Duration::from_millis(66);

/// Drop-in for `egui::Spinner` (`ui.add(Spinner::new())`).
#[must_use = "put it in a ui with `ui.add(widget)`"]
#[derive(Clone, Copy, Debug, Default)]
pub struct Spinner {
    /// The style's `interact_size` height if `None`.
    size: Option<f32>,
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Width and height.
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Paint into `rect`, asking for the next spinner frame if it is visible.
    pub fn paint_at(&self, ui: &Ui, rect: Rect) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        ui.ctx().request_repaint_after(TICK);
        let color = ui.visuals().strong_text_color();
        let radius = (rect.height().min(rect.width()) * 0.5) - 2.0;
        let n_points = (radius.round().max(0.0) as u32).clamp(8, 128);
        // Frames drawn for other reasons (input) show the same step, so it turns evenly.
        let time = step(ui.input(|i| i.time));
        let start_angle = time * std::f64::consts::TAU;
        let end_angle = start_angle + 240f64.to_radians() * time.sin();
        let points: Vec<Pos2> = (0..n_points)
            .map(|i| {
                let angle = lerp(start_angle..=end_angle, f64::from(i) / f64::from(n_points));
                let (sin, cos) = angle.sin_cos();
                rect.center() + radius * vec2(cos as f32, sin as f32)
            })
            .collect();
        ui.painter()
            .add(Shape::line(points, Stroke::new(3.0, color)));
    }
}

impl Widget for Spinner {
    fn ui(self, ui: &mut Ui) -> Response {
        let size = self
            .size
            .unwrap_or_else(|| ui.style().spacing.interact_size.y);
        let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::new(egui::Role::ProgressIndicator));
        self.paint_at(ui, rect);
        response
    }
}

/// `time` rounded down to a whole spinner frame.
fn step(time: f64) -> f64 {
    let tick = TICK.as_secs_f64();
    (time / tick).floor() * tick
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repaint_delay(add: impl Fn(&mut Ui)) -> Duration {
        let ctx = egui::Context::default();
        // A new context asks for a few frames while it settles (layout, fonts); measure after.
        let mut delay = Duration::MAX;
        for _ in 0..5 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| add(ui));
            output.textures_delta.clear();
            delay = output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map_or(Duration::MAX, |v| v.repaint_delay);
        }
        delay
    }

    #[test]
    #[allow(clippy::disallowed_types)] // the comparison is the point
    fn a_spinner_redraws_at_its_own_pace_not_every_frame() {
        assert_eq!(
            repaint_delay(|ui| {
                ui.add(egui::Spinner::new());
            }),
            Duration::ZERO,
            "egui's spinner keeps the window drawing"
        );
        assert_eq!(
            repaint_delay(|ui| {
                ui.add(Spinner::new());
            }),
            TICK
        );
    }

    #[test]
    fn frames_between_ticks_show_the_same_step() {
        let tick = TICK.as_secs_f64();
        assert_eq!(step(10.0 * tick + 0.001), step(10.0 * tick + tick * 0.9));
        assert!(step(11.0 * tick + 0.001) > step(10.0 * tick + 0.001));
    }
}
