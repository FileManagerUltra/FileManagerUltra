//! Pane chrome: location bar, tree/listing split with a drag-resizable handle,
//! focus ring. Returns the uri to open (`nav.open`), if any.

use egui::{Align, Layout, Sense, Ui};
use fm_engine::Session;
use fm_layout::PaneId;
use fm_vfs::VfsUri;

use crate::{listing, theme::Tokens, tree};

const MIN_TREE: f32 = 120.0;
const MAX_TREE: f32 = 420.0;

/// Render one pane. Returns the uri to open (`nav.open`), if any.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<VfsUri> {
    let area = ui.available_rect_before_wrap();
    let pressed_inside = ui.input(|i| {
        i.pointer.button_pressed(egui::PointerButton::Primary)
            && i.pointer.interact_pos().is_some_and(|p| area.contains(p))
    });
    let focused = s.layout.focused() == pane;
    let state = s.layout.state(pane)?;
    let location = state.location.clone();
    let tree_width = state.tree_width;

    let stroke = if focused {
        egui::Stroke::new(tokens.border_width.max(1.5), tokens.accent)
    } else {
        egui::Stroke::new(tokens.border_width, tokens.border)
    };
    let mut open = None;
    crate::dnd::note_pane(ui.ctx(), pane, area);
    crate::dnd::register_zone(
        ui.ctx(),
        area,
        location.clone(),
        crate::dnd::ZONE_BACKGROUND,
    );
    egui::Frame::NONE
        .fill(tokens.bg)
        .stroke(stroke)
        .inner_margin(egui::Margin::same(tokens.padding as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if focused {
                    ui.label(
                        egui::RichText::new("*")
                            .color(tokens.accent)
                            .font(tokens.font()),
                    );
                }
                ui.label(tokens.muted(location.path_str()));
            });
            ui.separator();

            let height = ui.available_height();
            let max_tree = (ui.available_width() - 160.0).clamp(MIN_TREE, MAX_TREE);
            let width = tree_width.clamp(MIN_TREE, max_tree);
            let handle_w = (tokens.border_width * 2.0).max(6.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        if let Some(target) = tree::show(ui, s, pane, tokens) {
                            open = Some(target);
                        }
                    },
                );
                let (handle_rect, handle) =
                    ui.allocate_exact_size(egui::vec2(handle_w, height), Sense::click_and_drag());
                ui.painter().rect_filled(handle_rect, 0.0, tokens.border);
                if handle.dragged()
                    && let Some(state) = s.layout.state_mut(pane)
                {
                    state.tree_width =
                        (state.tree_width + handle.drag_delta().x).clamp(MIN_TREE, MAX_TREE);
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        if let Some(target) = listing::show(ui, s, pane, tokens, focused) {
                            open = Some(target);
                        }
                    },
                );
            });
        });

    if pressed_inside {
        let _ = s.layout.focus_pane(pane);
    }
    crate::dnd::finish(ui.ctx(), s, area, tokens);
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_testkit::MemFs;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[allow(clippy::expect_used)]
    fn ready_session() -> Session {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/d/sub").expect("mkdirs");
        mem.put_str("/d/f.txt", b"f").expect("put");
        s.backends.register(Arc::new(mem));
        let uri = VfsUri::parse("/d").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs.state(&uri).is_ready() {
                break;
            }
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        s
    }

    #[test]
    fn pane_renders_headless_in_both_palettes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut s = ready_session();
            let ctx = egui::Context::default();
            crate::theme::apply_to(&ctx, &tokens);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let open = show(ui, &mut s, 1, &tokens);
                    assert!(open.is_none());
                });
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn split_survives_a_narrow_frame() {
        let tokens = Tokens::dark();
        let mut s = ready_session();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_max_width(200.0);
                let open = show(ui, &mut s, 1, &tokens);
                assert!(open.is_none());
            });
        });
        output.textures_delta.clear();
    }
}
