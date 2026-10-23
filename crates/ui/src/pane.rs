//! Pane chrome: location bar, tree/listing split with a drag-resizable handle,
//! focus ring. Returns the uri to open (`nav.open`), if any.

use egui::{Align, Layout, Sense, Ui};
use fm_engine::Session;
use fm_layout::{PaneId, PaneKind};
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
    let kind = state.kind;
    let tree_active = state.tree.active;

    let stroke = if focused {
        egui::Stroke::new(tokens.border_width.max(2.5), tokens.accent)
    } else {
        egui::Stroke::new(tokens.border_width, tokens.border)
    };
    let mut open = None;
    if kind == PaneKind::Files && focused && !tree_active {
        crate::locationbar::type_to_go(ui.ctx(), s, pane);
    }
    crate::dnd::note_pane(ui.ctx(), pane, area);
    if kind == PaneKind::Files {
        crate::dnd::register_zone(
            ui.ctx(),
            area,
            location.clone(),
            crate::dnd::ZONE_BACKGROUND,
        );
    }
    egui::Frame::NONE
        .fill(tokens.bg)
        .stroke(stroke)
        .inner_margin(egui::Margin::same(tokens.padding as i8))
        .show(ui, |ui| {
            // The active pane gets a solid accent header bar; inactive ones stay muted.
            let mut grip = None;
            if kind == PaneKind::Files && crate::locationbar::is_editing(s, pane) {
                crate::locationbar::show_editor(ui, s, pane, tokens);
            } else {
                let header = egui::Frame::NONE
                    .fill(if focused { tokens.accent } else { tokens.panel })
                    .corner_radius(tokens.corner_radius)
                    .inner_margin(egui::Margin::symmetric(tokens.padding as i8, 2));
                // The scope registers its drag sense before the contents, so the kind menu
                // and breadcrumb (added later) win hit-testing over the grip.
                let scope = ui.scope_builder(egui::UiBuilder::new().sense(Sense::drag()), |ui| {
                    header.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                        ui.allocate_ui_with_layout(
                            row,
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                kind_menu(ui, s, pane, kind);
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    if kind == PaneKind::Files {
                                        crate::locationbar::show_label(
                                            ui, s, pane, tokens, focused,
                                        );
                                    } else {
                                        ui.label(kind.label());
                                    }
                                });
                            },
                        );
                    })
                });
                grip = Some(scope.response);
            }
            ui.separator();
            if let Some(g) = grip {
                crate::arrange::source(ui.ctx(), &g, pane);
            }

            if kind == PaneKind::Terminal {
                crate::terminal::show(ui, s, pane, tokens, focused);
                return;
            }

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
                let active = handle.hovered() || handle.dragged();
                if active {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                ui.painter().rect_filled(
                    handle_rect,
                    0.0,
                    if active { tokens.accent } else { tokens.border },
                );
                if handle.double_clicked()
                    && let Some(state) = s.layout.state_mut(pane)
                {
                    state.tree_width = fm_layout::DEFAULT_TREE_WIDTH;
                } else if handle.dragged()
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
    let dragging = egui::DragAndDrop::has_payload_of_type::<crate::dnd::DragPayload>(ui.ctx());
    if kind == PaneKind::Terminal {
        crate::dnd::finish_terminal(ui.ctx(), s, pane, area, tokens);
    } else {
        crate::dnd::finish(ui.ctx(), s, area, tokens);
    }
    crate::arrange::finish(ui.ctx(), s, pane, area, tokens);
    // A drop hands the keyboard to the pane it landed on.
    let landed = ui.input(|i| {
        i.pointer.any_released() && i.pointer.interact_pos().is_some_and(|p| area.contains(p))
    });
    if dragging
        && landed
        && !egui::DragAndDrop::has_payload_of_type::<crate::dnd::DragPayload>(ui.ctx())
    {
        let _ = s.layout.focus_pane(pane);
    }
    open
}

/// Pane-type switcher; dispatches `pane.kind.set`. Unsupported kinds are greyed out.
fn kind_menu(ui: &mut Ui, s: &mut Session, pane: PaneId, current: PaneKind) {
    // Trailing spaces reserve room for the painted dropdown triangle (the bundled fonts lack ▾).
    let button = ui.menu_button(format!("{}   ", current.label()), |ui| {
        for kind in PaneKind::ALL {
            let supported = matches!(kind, PaneKind::Files | PaneKind::Terminal);
            let label = if supported {
                kind.label().to_string()
            } else {
                format!("{} (soon)", kind.label())
            };
            let resp = ui.add_enabled(supported, egui::Button::selectable(kind == current, label));
            if resp.clicked() {
                s.dispatch_or_notice(
                    "pane.kind.set",
                    &serde_json::json!({ "kind": kind.as_str(), "pane": pane }),
                );
                ui.close();
            }
        }
    });
    let r = button.response.rect;
    let color = ui.style().interact(&button.response).fg_stroke.color;
    let size = r.height() * 0.14;
    let c = egui::pos2(
        r.right() - ui.spacing().button_padding.x - size,
        r.center().y,
    );
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            c + egui::vec2(-size, -size * 0.5),
            c + egui::vec2(size, -size * 0.5),
            c + egui::vec2(0.0, size * 0.7),
        ],
        color,
        egui::Stroke::NONE,
    ));
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
