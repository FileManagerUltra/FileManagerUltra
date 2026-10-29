//! Places: the standard user folders and the user's bookmarks, shown above the folder tree.
//!
//! Rows are painted like the tree (full width, one row height, token colours) with a small
//! vector icon per kind, since the bundled fonts have no emoji. Clicking opens the place in
//! the pane; dropping files on a row copies/moves them there; bookmarks reorder by dragging
//! and have a menu to rename, move or remove them. Everything is dispatched as commands.

use egui::{Color32, Painter, Pos2, Rect, Sense, Stroke, Ui, vec2};
use fm_engine::Session;
use fm_engine::item_cmds::display_path;
use fm_engine::places_cmds::{Place, PlaceKind};
use fm_layout::PaneId;
use fm_vfs::{Scheme, VfsUri};
use serde_json::json;

use crate::dialogs::{Dialog, queue};
use crate::theme::Tokens;

/// A bookmark being dragged to a new position (index among the bookmarks).
#[derive(Clone, Copy, Debug)]
struct BookmarkDrag(usize);

/// Render Places for `pane`; returns the place to open, if one was clicked.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<VfsUri> {
    let location = s.layout.state(pane).map(|p| p.location.clone());
    let collapsed = s.layout.state(pane).map(|p| p.places).unwrap_or_default();
    let places = s.places();
    let (standard, bookmarks): (Vec<&Place>, Vec<&Place>) =
        places.iter().partition(|p| !p.bookmark);
    let can_add = location.as_ref().is_some_and(|l| {
        !s.is_bookmarked(l) && matches!(l.scheme(), Scheme::File | Scheme::Smb | Scheme::Sftp)
    });
    let mut open = None;
    let max_height = (ui.available_height() * 0.45).max(tokens.row_height * 4.0);
    egui::ScrollArea::vertical()
        .id_salt(("fm-places", pane))
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let devices = s.volumes.entries().to_vec();
            if !devices.is_empty() {
                let (expanded, _, toggled) =
                    section(ui, tokens, "Devices", !collapsed.devices_collapsed, None);
                if toggled && let Some(st) = s.layout.state_mut(pane) {
                    st.places.devices_collapsed = !expanded;
                }
                if expanded {
                    for device in &devices {
                        if device_row(ui, s, pane, tokens, device, location.as_ref()) {
                            open = Some(device.mount.clone());
                        }
                    }
                }
                ui.add_space(tokens.padding * 0.5);
            }
            let (expanded, _, toggled) =
                section(ui, tokens, "Places", !collapsed.places_collapsed, None);
            if toggled && let Some(st) = s.layout.state_mut(pane) {
                st.places.places_collapsed = !expanded;
            }
            if !standard.is_empty() && expanded {
                for place in &standard {
                    if row(ui, s, pane, tokens, place, location.as_ref(), None) {
                        open = Some(place.uri.clone());
                    }
                }
            }
            ui.add_space(tokens.padding * 0.5);
            let (expanded, add, toggled) = section(
                ui,
                tokens,
                "Bookmarks",
                !collapsed.bookmarks_collapsed,
                Some(can_add),
            );
            if toggled && let Some(st) = s.layout.state_mut(pane) {
                st.places.bookmarks_collapsed = !expanded;
            }
            if add {
                let _ = s.layout.focus_pane(pane);
                s.dispatch_or_notice("bookmark.add", &serde_json::Value::Null);
            }
            if expanded {
                for (i, place) in bookmarks.iter().enumerate() {
                    if row(ui, s, pane, tokens, place, location.as_ref(), Some(i)) {
                        open = Some(place.uri.clone());
                    }
                }
            }
        });
    ui.add_space(tokens.padding * 0.5);
    ui.separator();
    if open.is_some() {
        let _ = s.layout.focus_pane(pane);
    }
    open
}

fn device_row(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    device: &fm_engine::devices::Device,
    location: Option<&VfsUri>,
) -> bool {
    let name = fm_engine::devices::display_label(&device.name);
    let path = fm_engine::devices::display_label(&display_path(&device.mount));
    let space = match (device.free, device.total) {
        (Some(free), Some(total)) => format!(
            "{} free of {}",
            crate::listing::human_size(free),
            crate::listing::human_size(total),
        ),
        _ => "Capacity unavailable".to_string(),
    };
    let tooltip = format!("{name}\n{path}\n{}\n{space}", device.filesystem);
    let current = location.is_some_and(|uri| device.mount.is_same_or_descendant_of(uri));
    let mut clicked = false;
    let group = ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let (icon, _) =
                ui.allocate_exact_size(vec2(tokens.font_size, tokens.row_height), Sense::hover());
            paint_icon(
                ui.painter(),
                PlaceKind::Computer,
                icon.center(),
                tokens.font_size * 0.45,
                tokens.text_muted,
            );
            let response = ui
                .add_sized(
                    [
                        (ui.available_width() - tokens.font_size * 6.5).max(20.0),
                        tokens.row_height,
                    ],
                    egui::Button::selectable(current, "")
                        .left_text(&name)
                        .truncate(),
                )
                .on_hover_text(&tooltip);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            clicked = response.clicked();
            response.context_menu(|ui| {
                if ui.button("Open").clicked() {
                    clicked = true;
                    ui.close();
                }
                if device_removal(ui, s, device) {
                    ui.close();
                }
            });
            device_removal(ui, s, device);
        });
    });
    crate::dnd::register_zone(
        ui.ctx(),
        group.response.rect,
        device.mount.clone(),
        crate::dnd::ZONE_ROW,
    );
    if clicked {
        let _ = s.layout.focus_pane(pane);
    }
    clicked
}

/// Shared by the sidebar and Go menu. Unsupported volume managers remain browseable.
pub fn device_removal(ui: &mut Ui, s: &mut Session, device: &fm_engine::devices::Device) -> bool {
    use fm_engine::devices::Removal;
    let removing = s
        .volumes
        .removal_target()
        .is_some_and(|target| target.mount == device.mount && target.identity == device.identity);
    let enabled = !s.volumes.removing() && !matches!(device.removal, Removal::Unsupported(_));
    let tip = match &device.removal {
        _ if removing => "Removal requested for this volume; do not unplug it until confirmed",
        _ if s.volumes.removing() => "Wait for the other volume's removal to finish",
        Removal::Unsupported(reason) => reason.as_str(),
        Removal::Eject => "Unmount and safely eject this drive",
        Removal::Dismount => "Unmount this volume without powering off its host disk",
    };
    let label = if removing {
        match device.removal {
            Removal::Eject => "Ejecting...",
            _ => "Dismounting...",
        }
    } else {
        device.removal.label()
    };
    if ui
        .push_id(("volume-removal", device.mount.as_str()), |ui| {
            ui.add_enabled(enabled, egui::Button::new(label))
        })
        .inner
        .on_disabled_hover_text(tip)
        .on_hover_text(tip)
        .clicked()
    {
        s.dispatch_or_notice(
            device.removal.command(),
            &json!({"uri": device.mount.to_string()}),
        );
        return true;
    }
    false
}

/// Paint one line of `text` at `x` inside `rect`, truncated with `…` to the row width and
/// clipped to it, so long names never spill into the neighbouring column.
fn label(
    painter: &Painter,
    rect: Rect,
    x: f32,
    text: &str,
    font: egui::FontId,
    tokens: &Tokens,
    color: Color32,
) {
    let width = (rect.width() - x - tokens.padding).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let galley = painter.layout_job(job);
    let pos = Pos2::new(rect.left() + x, rect.center().y - galley.size().y * 0.5);
    painter
        .with_clip_rect(rect.intersect(painter.clip_rect()))
        .galley(pos, galley, color);
}

fn indent(tokens: &Tokens) -> f32 {
    tokens.padding * 2.0 + tokens.font_size
}

/// A collapsible section header. With `add`, a `+` button (enabled when `Some(true)`) is
/// drawn on the right. Returns `(expanded, add_clicked, toggled)`; the caller stores the
/// collapse state in the pane's layout state so it persists with the workspace.
fn section(
    ui: &mut Ui,
    tokens: &Tokens,
    title: &str,
    mut expanded: bool,
    add: Option<bool>,
) -> (bool, bool, bool) {
    let height = tokens.row_height;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    let plus = add.map(|_| {
        Rect::from_center_size(
            Pos2::new(
                rect.right() - height * 0.5 - tokens.padding,
                rect.center().y,
            ),
            vec2(height * 0.8, height * 0.8),
        )
    });
    let pointer = ui.ctx().pointer_hover_pos();
    let on_plus = plus.zip(pointer).is_some_and(|(r, p)| r.contains(p));
    let mut added = false;
    let mut toggled = false;
    if response.clicked() {
        if on_plus {
            added = add == Some(true);
        } else {
            expanded = !expanded;
            toggled = true;
        }
    }
    let painter = ui.painter();
    let color = tokens.accent;
    chevron(
        painter,
        Pos2::new(
            rect.left() + tokens.padding + tokens.font_size * 0.35,
            rect.center().y,
        ),
        tokens.font_size * 0.35,
        expanded,
        color,
    );
    let galley = egui::WidgetText::from(egui::RichText::new(title).color(color).strong())
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        );
    let pos = egui::pos2(
        rect.left() + tokens.padding * 2.0 + tokens.font_size * 0.7,
        rect.center().y - galley.size().y * 0.5,
    );
    // The bundled monospace font has no bold face; a small overstrike adds weight.
    painter.galley(pos, galley.clone(), color);
    painter.galley(pos + vec2(0.6, 0.0), galley, color);
    if let (Some(r), Some(enabled)) = (plus, add) {
        let color = match (enabled, on_plus) {
            (false, _) => tokens.border,
            (true, true) => tokens.accent_text,
            (true, false) => tokens.text_muted,
        };
        if enabled && on_plus {
            painter.rect_filled(r, tokens.corner_radius, tokens.hover_bg);
        }
        let c = r.center();
        let a = r.width() * 0.28;
        let stroke = Stroke::new(1.5, color);
        painter.line_segment([c - vec2(a, 0.0), c + vec2(a, 0.0)], stroke);
        painter.line_segment([c - vec2(0.0, a), c + vec2(0.0, a)], stroke);
    }
    let tip = match add {
        Some(true) if on_plus => "Bookmark the current folder (Ctrl+D)",
        Some(false) if on_plus => "The current folder is already bookmarked",
        _ if expanded => "Collapse",
        _ => "Expand",
    };
    response.on_hover_text(tip);
    (expanded, added, toggled)
}

/// One place. `bookmark` is the index among the bookmarks (reorderable). Returns `true` when
/// clicked.
fn row(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    place: &Place,
    location: Option<&VfsUri>,
    bookmark: Option<usize>,
) -> bool {
    let sense = if bookmark.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), tokens.row_height), sense);
    let current = location == Some(&place.uri);
    let dragging_bookmark = egui::DragAndDrop::has_payload_of_type::<BookmarkDrag>(ui.ctx());
    let painter = ui.painter();
    let bg_rect = rect.shrink2(vec2(tokens.padding * 0.5, 1.0));
    if current {
        painter.rect_filled(bg_rect, tokens.corner_radius, tokens.selection_bg);
    } else if response.hovered() && !dragging_bookmark {
        painter.rect_filled(bg_rect, tokens.corner_radius, tokens.hover_bg);
    }
    let fg = if current {
        tokens.selection_text
    } else {
        tokens.text
    };
    let icon_color = if current {
        tokens.selection_text
    } else if place.bookmark {
        tokens.accent_text
    } else {
        tokens.text_muted
    };
    let icon_c = Pos2::new(
        rect.left() + tokens.padding + tokens.font_size * 0.5,
        rect.center().y,
    );
    paint_icon(
        painter,
        place.kind,
        icon_c,
        tokens.font_size * 0.5,
        icon_color,
    );
    let font = egui::TextStyle::Button.resolve(ui.style());
    label(painter, rect, indent(tokens), &place.name, font, tokens, fg);
    let response = response.on_hover_text(display_path(&place.uri));

    // Files dropped on a place go there, like on a tree row.
    crate::dnd::register_zone(ui.ctx(), rect, place.uri.clone(), crate::dnd::ZONE_ROW);
    if let Some(index) = bookmark {
        if response.drag_started() {
            response.dnd_set_drag_payload(BookmarkDrag(index));
        }
        if let Some(pos) = ui.ctx().pointer_interact_pos()
            && response.dnd_hover_payload::<BookmarkDrag>().is_some()
        {
            let below = pos.y > rect.center().y;
            let y = if below { rect.bottom() } else { rect.top() };
            ui.painter().line_segment(
                [
                    Pos2::new(rect.left() + tokens.padding, y),
                    Pos2::new(rect.right() - tokens.padding, y),
                ],
                Stroke::new(2.0, tokens.accent),
            );
            if let Some(dragged) = response.dnd_release_payload::<BookmarkDrag>() {
                let mut to = index + usize::from(below);
                if dragged.0 < to {
                    to -= 1;
                }
                if let Some(uri) = s.prefs.bookmarks.get(dragged.0).map(|b| b.uri.clone())
                    && to != dragged.0
                {
                    s.dispatch_or_notice("bookmark.move", &json!({"uri": uri, "to": to}));
                }
            }
        }
        bookmark_menu(&response, s, place, index);
    } else {
        let bookmarked = s.is_bookmarked(&place.uri);
        response.context_menu(|ui| {
            if ui.button("Open").clicked() {
                let _ = s.layout.focus_pane(pane);
                s.dispatch_or_notice("nav.open", &json!({ "uri": place.uri.to_string() }));
                ui.close();
            }
            if ui
                .add_enabled(!bookmarked, egui::Button::new("Add to bookmarks"))
                .clicked()
            {
                s.dispatch_or_notice(
                    "bookmark.add",
                    &json!({ "uri": place.uri.to_string(), "name": place.name }),
                );
                ui.close();
            }
            if ui.button("Copy path").clicked() {
                s.dispatch_or_notice(
                    "file.copy_path",
                    &json!({ "sources": [place.uri.to_string()] }),
                );
                ui.close();
            }
        });
    }
    response.clicked()
}

fn bookmark_menu(response: &egui::Response, s: &mut Session, place: &Place, index: usize) {
    let uri = place.uri.to_string();
    let count = s.prefs.bookmarks.len();
    response.context_menu(|ui| {
        if ui.button("Rename…").clicked() {
            queue(
                ui.ctx(),
                Dialog::RenameBookmark {
                    uri: uri.clone(),
                    name: place.name.clone(),
                },
            );
            ui.close();
        }
        if ui
            .add_enabled(index > 0, egui::Button::new("Move up"))
            .clicked()
        {
            s.dispatch_or_notice("bookmark.move", &json!({"uri": uri, "by": -1}));
            ui.close();
        }
        if ui
            .add_enabled(index + 1 < count, egui::Button::new("Move down"))
            .clicked()
        {
            s.dispatch_or_notice("bookmark.move", &json!({"uri": uri, "by": 1}));
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            s.dispatch_or_notice("file.copy_path", &json!({ "sources": [uri] }));
            ui.close();
        }
        ui.separator();
        if ui.button("Remove bookmark").clicked() {
            s.dispatch_or_notice("bookmark.remove", &json!({ "uri": uri }));
            ui.close();
        }
    });
}

fn chevron(painter: &Painter, c: Pos2, size: f32, expanded: bool, color: Color32) {
    let pts = if expanded {
        vec![
            c + vec2(-size, -size * 0.5),
            c + vec2(size, -size * 0.5),
            c + vec2(0.0, size * 0.7),
        ]
    } else {
        vec![
            c + vec2(-size * 0.5, -size),
            c + vec2(-size * 0.5, size),
            c + vec2(size * 0.7, 0.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// A small line icon of radius `r` centred on `c`.
fn paint_icon(painter: &Painter, kind: PlaceKind, c: Pos2, r: f32, color: Color32) {
    let stroke = Stroke::new(1.3, color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
    let p = |x: f32, y: f32| c + vec2(x * r, y * r);
    match kind {
        PlaceKind::Home => {
            painter.add(egui::Shape::closed_line(
                vec![
                    p(0.0, -0.9),
                    p(0.9, -0.05),
                    p(0.65, -0.05),
                    p(0.65, 0.85),
                    p(-0.65, 0.85),
                    p(-0.65, -0.05),
                    p(-0.9, -0.05),
                ],
                stroke,
            ));
            painter.rect_stroke(
                Rect::from_min_max(p(-0.2, 0.3), p(0.2, 0.85)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }

        PlaceKind::Desktop => {
            painter.rect_stroke(
                Rect::from_min_max(p(-0.9, -0.75), p(0.9, 0.4)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            line(p(0.0, 0.4), p(0.0, 0.8));
            line(p(-0.45, 0.85), p(0.45, 0.85));
        }
        PlaceKind::Documents => {
            painter.add(egui::Shape::closed_line(
                vec![
                    p(-0.65, -0.9),
                    p(0.3, -0.9),
                    p(0.7, -0.5),
                    p(0.7, 0.9),
                    p(-0.65, 0.9),
                ],
                stroke,
            ));
            for y in [-0.25, 0.15, 0.5] {
                line(p(-0.35, y), p(0.4, y));
            }
        }
        PlaceKind::Downloads => {
            line(p(0.0, -0.9), p(0.0, 0.35));
            line(p(-0.45, -0.1), p(0.0, 0.35));
            line(p(0.45, -0.1), p(0.0, 0.35));
            painter.add(egui::Shape::line(
                vec![p(-0.85, 0.35), p(-0.85, 0.85), p(0.85, 0.85), p(0.85, 0.35)],
                stroke,
            ));
        }
        PlaceKind::Music => {
            line(p(-0.3, 0.55), p(-0.3, -0.75));
            line(p(-0.3, -0.75), p(0.65, -0.95));
            line(p(0.65, -0.95), p(0.65, 0.35));
            painter.circle_filled(p(-0.5, 0.6), r * 0.28, color);
            painter.circle_filled(p(0.45, 0.4), r * 0.28, color);
        }
        PlaceKind::Pictures => {
            painter.rect_stroke(
                Rect::from_min_max(p(-0.9, -0.75), p(0.9, 0.75)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.add(egui::Shape::line(
                vec![
                    p(-0.9, 0.6),
                    p(-0.3, -0.05),
                    p(0.15, 0.4),
                    p(0.45, 0.1),
                    p(0.9, 0.55),
                ],
                stroke,
            ));
            painter.circle_filled(p(0.4, -0.35), r * 0.14, color);
        }
        PlaceKind::Videos => {
            painter.rect_stroke(
                Rect::from_min_max(p(-0.9, -0.7), p(0.9, 0.7)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![p(-0.25, -0.35), p(0.4, 0.0), p(-0.25, 0.35)],
                color,
                Stroke::NONE,
            ));
        }
        PlaceKind::Computer => {
            // A drive: a flat box with an activity light.
            painter.rect_stroke(
                Rect::from_min_max(p(-0.9, -0.45), p(0.9, 0.45)),
                1.5,
                stroke,
                egui::StrokeKind::Middle,
            );
            line(p(-0.9, 0.05), p(0.9, 0.05));
            painter.circle_filled(p(0.55, 0.27), r * 0.1, color);
        }
        PlaceKind::Bookmark => {
            painter.add(egui::Shape::closed_line(
                vec![
                    p(-0.55, -0.9),
                    p(0.55, -0.9),
                    p(0.55, 0.9),
                    p(0.0, 0.4),
                    p(-0.55, 0.9),
                ],
                stroke,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use fm_engine::devices::{Device, Removal};

    fn device() -> Device {
        Device {
            name: "Encrypted files".into(),
            mount: VfsUri::parse("/media/veracrypt1").expect("uri"),
            filesystem: "exfat".into(),
            total: Some(64 << 30),
            free: Some(32 << 30),
            identity: "virtual-42".into(),
            removal: Removal::Unsupported("Dismount this volume in VeraCrypt".into()),
        }
    }

    #[test]
    fn two_eject_buttons_dispatch_only_the_clicked_volume() {
        let mut s = Session::new().expect("session");
        s.standard_places.0.clear();
        let mem = fm_testkit::MemFs::new();
        mem.mkdirs_str("/media/first").expect("first");
        mem.mkdirs_str("/media/second").expect("second");
        s.backends.register(std::sync::Arc::new(mem));
        let mut first = device();
        first.name = "First USB".into();
        first.mount = VfsUri::parse("/media/first").expect("uri");
        first.removal = Removal::Eject;
        let mut second = first.clone();
        second.name = "Second USB".into();
        second.mount = VfsUri::parse("/media/second").expect("uri");
        second.identity = "second".into();
        s.volumes.replace(vec![first.clone(), second]);
        s.submit_op(fm_engine::OpRequest::Mkdir {
            parent: first.mount,
            name: "test".into(),
        })
        .expect("memory-only operation");
        let mut h = Harness::builder()
            .with_size(vec2(450.0, 800.0))
            .build_ui_state(
                |ui, s| {
                    let _ = show(ui, s, 1, &Tokens::dark());
                },
                s,
            );
        assert_eq!(h.query_all_by_label("Eject").count(), 2);
        h.query_all_by_label("Eject").next().expect("first").click();
        h.run();
        assert!(
            h.state()
                .notice
                .as_deref()
                .is_some_and(|text| text.contains("file operation"))
        );
        h.query_all_by_label("Eject")
            .nth(1)
            .expect("second")
            .click();
        h.run();
        assert!(
            h.state()
                .notice
                .as_deref()
                .is_some_and(|text| text.contains("Home is unavailable"))
        );
        assert!(!h.state().volumes.removing(), "never execute OS removal");
    }

    #[test]
    fn devices_are_conditional_and_collapse_independently_in_both_themes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut s = Session::new().expect("session");
            s.standard_places.0.clear();
            let mem = fm_testkit::MemFs::new();
            mem.mkdirs_str("/media/veracrypt1").expect("mount");
            s.backends.register(std::sync::Arc::new(mem));
            let mut h = Harness::builder()
                .with_size(vec2(450.0, 800.0))
                .build_ui_state(
                    move |ui, s| {
                        crate::theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        if let Some(uri) = show(ui, s, 1, &tokens) {
                            s.dispatch_or_notice("nav.open", &json!({"uri": uri.to_string()}));
                        }
                    },
                    s,
                );
            assert!(h.query_by_label("Devices").is_none());
            h.state_mut().volumes.replace(vec![device()]);
            h.run();
            h.get_by_label("Devices");
            h.get_by_label("Encrypted files");
            assert!(h.query_by_label("32.0 GiB free of 64.0 GiB").is_none());
            assert!(h.query_by_label("/media/veracrypt1").is_none());
            assert!(h.get_by_label("Dismount").accesskit_node().is_disabled());
            h.get_by_label("Encrypted files").click();
            h.run();
            assert_eq!(
                h.state().layout.state(1).expect("pane").location,
                device().mount
            );
            let mut unknown = device();
            unknown.total = None;
            unknown.free = None;
            h.state_mut().volumes.replace(vec![unknown]);
            h.run();
            assert!(h.query_by_label("Capacity unavailable").is_none());
            h.get_by_label("Devices").click();
            h.run();
            assert!(
                h.state()
                    .layout
                    .state(1)
                    .expect("pane")
                    .places
                    .devices_collapsed
            );
            assert!(h.query_by_label("Encrypted files").is_none());
            h.get_by_label("Devices");
            h.get_by_label("Places");
            h.state_mut().volumes.replace(Vec::new());
            h.run();
            assert!(h.query_by_label("Devices").is_none());
        }
    }
}
