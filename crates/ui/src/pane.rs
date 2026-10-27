//! Pane chrome: location bar, tree/listing split with a drag-resizable handle,
//! focus ring. Returns the uri to open (`nav.open`), if any.

use egui::{Align, Layout, Sense, Ui};
use fm_engine::Session;
use fm_layout::{PaneId, PaneKind};
use fm_vfs::VfsUri;

use crate::{listing, theme::Tokens, tree};

/// Thickness (px) of the draggable band along each pane edge.
const BORDER_GRIP: f32 = 5.0;
const MIN_TREE: f32 = 120.0;
const MAX_TREE: f32 = 420.0;

/// Render one pane. Returns the uri to open (`nav.open`), if any.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<VfsUri> {
    let area = ui.available_rect_before_wrap();
    let pressed_inside = ui.input(|i| {
        (i.pointer.button_pressed(egui::PointerButton::Primary)
            || i.pointer.button_pressed(egui::PointerButton::Secondary))
            && i.pointer.interact_pos().is_some_and(|p| area.contains(p))
    }) && ui
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| ui.ctx().layer_id_at(p))
        .is_none_or(|layer| layer == ui.layer_id());
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
    if kind.is_files() && focused && !tree_active {
        crate::locationbar::type_to_go(ui.ctx(), s, pane);
    }
    crate::dnd::note_pane(ui.ctx(), pane, area);
    if kind.is_files() {
        crate::dnd::register_zone(
            ui.ctx(),
            area,
            location.clone(),
            crate::dnd::ZONE_BACKGROUND,
        );
    }
    let frame = egui::Frame::NONE
        .fill(tokens.bg)
        .stroke(stroke)
        .inner_margin(egui::Margin::same(tokens.padding as i8))
        .show(ui, |ui| {
            // A frame only wraps its content; make it fill the slot it was given, however the
            // window was resized.
            ui.set_min_size(ui.available_size());
            // The active pane gets a solid accent header bar; inactive ones stay muted.
            if kind.is_files() && crate::locationbar::is_editing(s, pane) {
                crate::locationbar::show_editor(ui, s, pane, tokens);
            } else {
                egui::Frame::NONE
                    .fill(tokens.panel)
                    .stroke(if focused {
                        egui::Stroke::new(1.5, tokens.accent)
                    } else {
                        egui::Stroke::NONE
                    })
                    .corner_radius(tokens.corner_radius)
                    .inner_margin(egui::Margin::symmetric(tokens.padding as i8, 2))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
                        ui.allocate_ui_with_layout(
                            row,
                            Layout::right_to_left(Align::Center),
                            |ui| {
                                close_button(ui, s, pane);
                                kind_menu(ui, s, pane, kind);
                                if kind == PaneKind::Sftp && ui.button("Connect…").clicked() {
                                    s.dispatch_or_notice(
                                        "remote.connect.dialog",
                                        &serde_json::Value::Null,
                                    );
                                }
                                if kind == PaneKind::Network && ui.button("Connect…").clicked() {
                                    s.dispatch_or_notice(
                                        "network.connect.dialog",
                                        &serde_json::Value::Null,
                                    );
                                }
                                if kind.is_files() {
                                    crate::remote::badge(ui, s, pane, tokens, focused);
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    if kind.is_files() {
                                        crate::locationbar::show_label(
                                            ui, s, pane, tokens, focused,
                                        );
                                    } else {
                                        ui.label(kind.label());
                                    }
                                });
                            },
                        );
                    });
            }
            ui.separator();

            if kind.is_files() {
                crate::remote::stale_banner(ui, s, pane, tokens);
            }
            if kind == PaneKind::Terminal {
                crate::terminal::show(ui, s, pane, tokens, focused);
                return;
            }
            if kind == PaneKind::Trash {
                crate::trash::show(ui, s, pane, tokens);
                return;
            }
            if kind == PaneKind::Preview {
                open = crate::preview::show(ui, s, pane, tokens);
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
                        ui.set_min_size(ui.available_size());
                        if let Some(target) = crate::places::show(ui, s, pane, tokens) {
                            open = Some(target);
                        }
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
                        ui.set_min_size(ui.available_size());
                        if let Some(target) = listing::show(ui, s, pane, tokens, focused) {
                            open = Some(target);
                        }
                    },
                );
            });
        });
    border_grip(ui, tokens, pane, frame.response.rect);

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

/// `×` in the header; dispatches `pane.close` for this pane. Hidden on the last pane.
fn close_button(ui: &mut Ui, s: &mut Session, pane: PaneId) {
    if s.layout.panes().len() < 2 {
        return;
    }
    if ui
        .add(egui::Button::new("×").frame(false))
        .on_hover_text("Close pane")
        .clicked()
    {
        s.dispatch_or_notice("pane.close", &serde_json::json!({ "pane": pane }));
    }
}

/// Pane-type switcher; dispatches `pane.kind.set`. Unsupported kinds are greyed out.
fn kind_menu(ui: &mut Ui, s: &mut Session, pane: PaneId, current: PaneKind) {
    // Trailing spaces reserve room for the painted dropdown triangle (the bundled fonts lack ▾).
    let button = ui.menu_button(format!("{}   ", current.label()), |ui| {
        for kind in PaneKind::ALL {
            let supported = matches!(
                kind,
                PaneKind::Files
                    | PaneKind::Terminal
                    | PaneKind::Preview
                    | PaneKind::Sftp
                    | PaneKind::Network
                    | PaneKind::Trash
            );
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

/// The pane's outer border drags the pane: four thin hit bands just inside its frame,
/// clear of the padding that holds the content.
fn border_grip(ui: &mut Ui, tokens: &Tokens, pane: PaneId, rect: egui::Rect) {
    let t = BORDER_GRIP;
    let bands = [
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + t)),
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - t), rect.max),
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + t, rect.bottom())),
        egui::Rect::from_min_max(egui::pos2(rect.right() - t, rect.top()), rect.max),
    ];
    let mut active = false;
    for (i, band) in bands.into_iter().enumerate() {
        let id = egui::Id::NULL.with(("pane-border", pane, i));
        let resp = ui.interact(band, id, Sense::drag());
        active |= resp.hovered() || resp.dragged();
        crate::arrange::source(ui.ctx(), &resp, pane);
    }
    if active {
        ui.painter().rect_stroke(
            rect,
            tokens.corner_radius,
            egui::Stroke::new(tokens.border_width.max(2.5), tokens.accent),
            egui::StrokeKind::Inside,
        );
    }
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
    fn a_press_on_a_popup_over_a_pane_does_not_focus_that_pane() {
        let tokens = Tokens::dark();
        let mut s = ready_session();
        let second = s
            .layout
            .split(1, fm_layout::Axis::Horizontal, 0.5)
            .expect("split");
        s.layout.focus_pane(1).expect("focus");
        let ctx = egui::Context::default();
        let at = egui::pos2(100.0, 100.0);
        let frame = |s: &mut Session, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = show(ui, s, second, &tokens);
                    });
                    egui::Area::new(egui::Id::NULL.with("menu"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(egui::pos2(0.0, 0.0))
                        .show(ctx, |ui| {
                            ui.set_min_size(egui::vec2(400.0, 400.0));
                            let _ = ui.button("Paste");
                        });
                },
            );
            out.textures_delta.clear();
        };
        frame(&mut s, vec![]);
        frame(&mut s, vec![egui::Event::PointerMoved(at)]);
        frame(
            &mut s,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(s.layout.focused(), 1, "the popup swallowed the press");
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

    #[test]
    fn arrows_in_the_listing_do_not_move_the_tree_cursor() {
        let tokens = Tokens::dark();
        let mut s = ready_session();
        let d = VfsUri::parse("/d").expect("uri");
        if let Some(p) = s.layout.state_mut(1) {
            p.tree.focused = Some(d.clone());
            p.tree.active = false;
        }
        let ctx = egui::Context::default();
        let frame = |s: &mut Session, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = show(ui, s, 1, &tokens);
                    });
                },
            );
            out.textures_delta.clear();
        };
        frame(&mut s, vec![]);
        frame(&mut s, vec![]);
        assert!(
            ctx.memory(|m| m.focused()).is_none(),
            "the tree grabbed focus"
        );
        let down = egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut s, vec![down]);
        let p = s.layout.state(1).expect("pane");
        assert_eq!(p.tree.focused.as_ref(), Some(&d));
        assert!(p.listing.focused_row > 0, "the listing cursor moved");
    }

    #[test]
    fn up_walks_from_the_first_entry_through_parent_to_the_current_row() {
        let tokens = Tokens::dark();
        let mut s = ready_session();
        let ctx = egui::Context::default();
        let key = |k| egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let frame = |s: &mut Session, events: Vec<egui::Event>| {
            let mut opened = None;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        opened = show(ui, s, 1, &tokens);
                    });
                },
            );
            out.textures_delta.clear();
            opened
        };
        frame(&mut s, vec![]);
        assert!(frame(&mut s, vec![key(egui::Key::ArrowUp)]).is_none());
        assert!(s.layout.state(1).is_some_and(|p| p.listing.on_parent));
        let opened = frame(&mut s, vec![key(egui::Key::Enter)]);
        assert_eq!(opened, VfsUri::parse("/d").expect("uri").parent());
        // Up from `..` reaches `.`; activating `.` does not navigate.
        assert!(frame(&mut s, vec![key(egui::Key::ArrowUp)]).is_none());
        let p = s.layout.state(1).expect("pane");
        assert!(p.listing.on_current && !p.listing.on_parent);
        assert_eq!(frame(&mut s, vec![key(egui::Key::Enter)]), None);
        // Down returns to `..`, and Home comes back to `.`.
        frame(&mut s, vec![key(egui::Key::ArrowDown)]);
        assert!(s.layout.state(1).is_some_and(|p| p.listing.on_parent));
        frame(&mut s, vec![key(egui::Key::Home)]);
        assert!(s.layout.state(1).is_some_and(|p| p.listing.on_current));
    }

    #[test]
    fn an_empty_directory_starts_with_the_cursor_on_the_current_row() {
        let tokens = Tokens::dark();
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/e").expect("mkdirs");
        s.backends.register(Arc::new(mem));
        let uri = VfsUri::parse("/e").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !s.dirs.state(&uri).is_ready() {
            s.poll();
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = show(ui, &mut s, 1, &tokens);
            });
        });
        out.textures_delta.clear();
        assert!(s.layout.state(1).is_some_and(|p| p.listing.on_current));
    }
}
