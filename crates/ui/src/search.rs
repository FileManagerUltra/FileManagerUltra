//! The Find Files window: a form (folder, name pattern, text inside files, hidden files), the
//! live status of the background walk and its virtualized results. Every action is a command:
//! `search.start`/`stop`/`close`, `file.open`, `file.reveal`, `file.copy_path`.

use egui::{Context, Id, RichText};
use fm_engine::Session;
use fm_engine::item_cmds::display_path;
use fm_vfs::EntryKind;
use serde_json::json;

use crate::listing::human_size;
use crate::theme::Tokens;

/// The form as typed; the engine only sees it when a search starts.
#[derive(Clone, Default)]
struct Draft {
    root: String,
    name: String,
    content: String,
    hidden: bool,
    /// The engine root this draft last copied, so `search.open` on another folder refreshes it.
    synced_root: Option<String>,
    selected: Option<usize>,
    focus: bool,
}

fn draft_id() -> Id {
    Id::NULL.with("fm-search-draft")
}

/// Render the window while `s.search` is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(search) = s.search.as_ref() else {
        ctx.data_mut(|d| d.remove_temp::<Draft>(draft_id()));
        return;
    };
    // Minimised: the Activity panel shows it; the form and its draft stay as they were.
    if search.minimized {
        return;
    }
    let status = search.status();
    let engine_root = search.query.root.as_ref().map(|u| u.to_string());
    let mut draft = ctx
        .data_mut(|d| d.get_temp::<Draft>(draft_id()))
        .unwrap_or_else(|| Draft {
            name: search.query.name.clone(),
            content: search.query.content.clone(),
            hidden: search.query.hidden,
            focus: true,
            ..Draft::default()
        });
    if draft.synced_root != engine_root {
        draft.root = engine_root.clone().unwrap_or_default();
        draft.synced_root = engine_root;
        draft.focus = true;
    }
    if status.running {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut open = true;
    let mut command: Option<(&'static str, serde_json::Value)> = None;
    let mut start = false;
    let mut close_requested = false;
    crate::dialog_style::window(ctx, tokens, "Find Files", &mut open)
        .id(Id::NULL.with("fm-search"))
        .resizable(true)
        .default_width(680.0)
        .title_bar(false)
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            // Own title bar so it can carry Minimize next to Close; dragging the window works anywhere.
            ui.horizontal(|ui| {
                let width = ui.available_width();
                let (minimize, close) = ui
                    .allocate_ui_with_layout(
                        egui::vec2(width, 24.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let close = title_button(ui, tokens, "Close window", true);
                            let minimize = title_button(ui, tokens, "Minimize window", false);
                            (minimize, close)
                        },
                    )
                    .inner;
                let bar =
                    egui::Rect::from_min_size(ui.min_rect().left_top(), egui::vec2(width, 24.0));
                ui.painter().text(
                    bar.center(),
                    egui::Align2::CENTER_CENTER,
                    "Find Files",
                    egui::FontId::proportional(tokens.font_size + 2.0),
                    tokens.text,
                );
                if minimize.clicked() {
                    command = Some(("search.minimize", serde_json::Value::Null));
                }
                if close.clicked() {
                    close_requested = true;
                }
            });
            ui.separator();
            let mut submit = false;
            let field =
                |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str, tip: &str| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let caption = ui.label(
                            RichText::new(label)
                                .font(egui::FontId::proportional(tokens.font_size - 1.0))
                                .color(tokens.text_muted),
                        );
                        ui.add(
                            egui::TextEdit::singleline(text)
                                .hint_text(hint)
                                .margin(egui::vec2(8.0, 6.0))
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(caption.id)
                        .on_hover_text(tip)
                    })
                    .inner
                };
            ui.spacing_mut().item_spacing.y = 8.0;
            let r = field(
                ui,
                "Name",
                &mut draft.name,
                "*.txt; report*",
                "Names containing this text, or matching * ? [a-z]; separate alternatives with ;",
            );
            if draft.focus {
                r.request_focus();
                draft.focus = false;
            }
            submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.columns(2, |cols| {
                let r = field(
                    &mut cols[0],
                    "Containing",
                    &mut draft.content,
                    "text inside files",
                    "Only files containing this text (case-insensitive; binary files are skipped)",
                );
                submit |= r.lost_focus() && cols[0].input(|i| i.key_pressed(egui::Key::Enter));
                let r = field(
                    &mut cols[1],
                    "Look in",
                    &mut draft.root,
                    "folder",
                    "The folder to search below",
                );
                submit |= r.lost_focus() && cols[0].input(|i| i.key_pressed(egui::Key::Enter));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut draft.hidden, "Include hidden")
                    .on_hover_text("Also search hidden files and folders");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ready = !draft.name.trim().is_empty() || !draft.content.is_empty();
                    if status.running {
                        if ui.button("Stop").on_hover_text("Stop searching").clicked() {
                            command = Some(("search.stop", serde_json::Value::Null));
                        }
                    } else if {
                        // The theme forces one text colour; the filled button needs its own.
                        ui.visuals_mut().override_text_color = Some(tokens.bg);
                        let clicked = ui
                            .add_enabled(
                                ready,
                                egui::Button::new(RichText::new("Search").strong())
                                    .fill(tokens.accent)
                                    .min_size(egui::vec2(88.0, 0.0)),
                            )
                            .on_hover_text("Search (Enter)")
                            .clicked();
                        ui.visuals_mut().override_text_color = Some(tokens.text);
                        clicked
                    } || (submit && ready)
                    {
                        start = true;
                    }
                    if ui
                        .button("Minimize")
                        .on_hover_text(
                            "Hide this window; a running search continues in the Activity panel",
                        )
                        .clicked()
                    {
                        command = Some(("search.minimize", serde_json::Value::Null));
                    }
                    let recent = s.recent_searches();
                    ui.add_enabled_ui(!recent.is_empty() && !status.running, |ui| {
                        ui.menu_button("Recent", |ui| {
                            for (index, saved) in recent.iter().enumerate() {
                                if ui.button(saved.label()).clicked() {
                                    draft.root = saved.root.clone();
                                    draft.synced_root = Some(saved.root.clone());
                                    draft.name = saved.name.clone();
                                    draft.content = saved.content.clone();
                                    draft.hidden = saved.hidden;
                                    draft.selected = None;
                                    command = Some(("search.history.run", json!({"index": index})));
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("Clear recent searches").clicked() {
                                command = Some(("search.history.clear", serde_json::Value::Null));
                                ui.close();
                            }
                        })
                        .response
                        .on_hover_text("Run one of your last 20 searches again");
                    });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                if status.running {
                    ui.add(crate::spinner::Spinner::new());
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(status_line(&status))
                            .font(egui::FontId::proportional(tokens.font_size - 1.0))
                            .color(tokens.text_muted),
                    )
                    .truncate(),
                );
            });
            if let Some(err) = &status.error {
                ui.label(RichText::new(err).color(tokens.error));
            }
            // Sized from the screen, not the free space: a window that fills its free space grows forever.
            let list_height = (ui.ctx().content_rect().height() - 460.0).clamp(160.0, 480.0);
            let row_height = tokens.row_height * 2.0 + 6.0;
            egui::Frame::new()
                .fill(tokens.bg)
                .stroke(egui::Stroke::new(tokens.border_width, tokens.border))
                .corner_radius(tokens.corner_radius)
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(ui.available_width(), list_height));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if status.found == 0 {
                        ui.allocate_ui(egui::vec2(ui.available_width(), list_height), |ui| {
                            ui.centered_and_justified(|ui| {
                                crate::dialog_style::hint(
                                    ui,
                                    tokens,
                                    if status.running {
                                        "Searching…"
                                    } else {
                                        "No results"
                                    },
                                );
                            });
                        });
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("fm-search-results")
                        .auto_shrink([false, false])
                        .max_height(list_height)
                        .show_rows(ui, row_height, status.found, |ui, range| {
                            for i in range {
                                let Some(entry) = search_result(s, i) else {
                                    continue;
                                };
                                let dir = entry.kind == EntryKind::Dir;
                                let (response, action) = result_row(
                                    ui,
                                    tokens,
                                    &entry,
                                    row_height,
                                    draft.selected == Some(i),
                                );
                                let uri = entry.uri.to_string();
                                match action {
                                    Some(RowAction::Open) => {
                                        command = Some(if dir {
                                            ("file.reveal", json!({ "uri": uri }))
                                        } else {
                                            ("file.open", json!({ "uri": uri }))
                                        });
                                    }
                                    Some(RowAction::Reveal) => {
                                        command = Some(("file.reveal", json!({ "uri": uri })));
                                    }
                                    Some(RowAction::CopyPath) => {
                                        command =
                                            Some(("file.copy_path", json!({ "sources": [uri] })));
                                    }
                                    None => {}
                                }
                                if response.clicked() || response.secondary_clicked() {
                                    draft.selected = Some(i);
                                }
                                let uri = entry.uri.to_string();
                                response.context_menu(|ui| {
                                    let sources = json!({ "sources": [&uri] });
                                    let items: [(&str, &str, serde_json::Value); 6] = [
                                        ("Open", "file.open", json!({ "uri": &uri })),
                                        (
                                            "Open With…",
                                            "file.open_with.dialog",
                                            json!({ "uri": &uri }),
                                        ),
                                        ("Show in Folder", "file.reveal", json!({ "uri": &uri })),
                                        ("Copy Path", "file.copy_path", sources.clone()),
                                        ("Copy Name", "file.copy_name", sources.clone()),
                                        ("Properties", "file.properties", sources),
                                    ];
                                    for (n, (label, id, params)) in items.into_iter().enumerate() {
                                        if matches!(n, 3 | 5) {
                                            ui.separator();
                                        }
                                        if ui.button(label).clicked() {
                                            command = Some((id, params));
                                            ui.close();
                                        }
                                    }
                                });
                                if response.double_clicked() {
                                    let uri = entry.uri.to_string();
                                    command = Some(if dir {
                                        ("file.reveal", json!({ "uri": uri }))
                                    } else {
                                        ("file.open", json!({ "uri": uri }))
                                    });
                                }
                            }
                        });
                });
        });
    if close_requested {
        open = false;
    }
    if start && open {
        draft.selected = None;
        let mut params = json!({
            "name": draft.name.trim(),
            "content": draft.content,
            "hidden": draft.hidden,
        });
        if !draft.root.trim().is_empty() {
            params["root"] = json!(draft.root.trim());
        }
        s.dispatch_or_notice("search.start", &params);
        draft.synced_root = s
            .search
            .as_ref()
            .and_then(|x| x.query.root.as_ref())
            .map(|u| u.to_string());
    }
    if let Some((id, params)) = command.filter(|_| open) {
        s.dispatch_or_notice(id, &params);
    }
    ctx.data_mut(|d| d.insert_temp(draft_id(), draft));
    if !open {
        s.dispatch_or_notice("search.close", &serde_json::Value::Null);
    }
}

/// A borderless glyph button for the title bar: a cross (`close`) or a dash.
fn title_button(ui: &mut egui::Ui, tokens: &Tokens, label: &str, close: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, tokens.corner_radius, tokens.hover_bg);
    }
    let stroke = egui::Stroke::new(1.5, tokens.text);
    let c = rect.center();
    let r = 5.0;
    if close {
        ui.painter()
            .line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], stroke);
        ui.painter()
            .line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], stroke);
    } else {
        ui.painter()
            .line_segment([c + egui::vec2(-r, 0.0), c + egui::vec2(r, 0.0)], stroke);
    }
    response.on_hover_text(label)
}

#[derive(Clone, Copy)]
enum RowAction {
    Open,
    Reveal,
    CopyPath,
}

fn paint_row_icon(painter: &egui::Painter, c: egui::Pos2, action: RowAction, stroke: egui::Stroke) {
    use egui::{Rect, Shape, Vec2, pos2, vec2};
    let r = 6.0;
    match action {
        // Box with an arrow leaving its top-right corner.
        RowAction::Open => {
            let line = |a: egui::Pos2, b: egui::Pos2| painter.line_segment([a, b], stroke);
            line(c + vec2(-r, -r), c + vec2(-r, r));
            line(c + vec2(-r, r), c + vec2(r, r));
            line(c + vec2(r, r), c + vec2(r, 1.0));
            line(c + vec2(-r, -r), c + vec2(-1.0, -r));
            line(c + vec2(-1.0, 1.0), c + vec2(r, -r));
            line(c + vec2(1.0, -r), c + vec2(r, -r));
            line(c + vec2(r, -r), c + vec2(r, -1.0));
        }
        // Folder: tab plus body.
        RowAction::Reveal => {
            painter.add(Shape::closed_line(
                vec![
                    c + vec2(-r, -r + 1.0),
                    c + vec2(-1.5, -r + 1.0),
                    c + vec2(0.0, -r + 3.0),
                    c + vec2(r, -r + 3.0),
                    c + vec2(r, r - 1.0),
                    c + vec2(-r, r - 1.0),
                ],
                stroke,
            ));
        }
        // Two overlapping sheets.
        RowAction::CopyPath => {
            let size = Vec2::splat(8.0);
            painter.rect_stroke(
                Rect::from_min_size(pos2(c.x - r, c.y - r), size),
                1.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_size(pos2(c.x - r + 4.0, c.y - r + 4.0), size),
                1.5,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Two lines per hit: the name with its size, and the folder it lives in. Long text is clipped.
fn result_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    entry: &fm_vfs::Entry,
    height: f32,
    selected: bool,
) -> (egui::Response, Option<RowAction>) {
    let dir = entry.kind == EntryKind::Dir;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    let name = if dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
    if !ui.is_rect_visible(rect) {
        return (response, None);
    }
    let fill = if selected {
        Some(tokens.selection_bg)
    } else if response.hovered() {
        Some(tokens.hover_bg)
    } else {
        None
    };
    let painter = ui.painter_at(rect);
    if let Some(fill) = fill {
        painter.rect_filled(
            rect.shrink2(egui::vec2(3.0, 1.0)),
            tokens.corner_radius,
            fill,
        );
    }
    let (name_color, muted) = if selected {
        (tokens.selection_text, tokens.selection_text)
    } else {
        (tokens.text, tokens.text_muted)
    };
    let pad = 10.0;
    let mut action = None;
    let mut left = rect.left() + 6.0;
    for (act, label) in [
        (RowAction::Open, "Open"),
        (RowAction::Reveal, "Show in Folder"),
        (RowAction::CopyPath, "Copy Path"),
    ] {
        let slot = egui::Rect::from_center_size(
            egui::pos2(left + 11.0, rect.center().y),
            egui::vec2(22.0, 22.0),
        );
        let r = ui.interact(slot, response.id.with(label), egui::Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if r.hovered() {
            painter.rect_filled(slot, tokens.corner_radius, tokens.border);
        }
        let color = if selected {
            tokens.selection_text
        } else {
            tokens.text_muted
        };
        paint_row_icon(&painter, slot.center(), act, egui::Stroke::new(1.3, color));
        if r.clicked() {
            action = Some(act);
        }
        r.on_hover_text(label);
        left += 24.0;
    }
    let mut right = rect.right() - pad;
    if !dir && let Some(size) = entry.size {
        let galley = painter.layout_no_wrap(
            human_size(size),
            egui::FontId::proportional(tokens.font_size - 1.0),
            muted,
        );
        right -= galley.size().x;
        painter.galley(egui::pos2(right, rect.top() + 7.0), galley, muted);
        right -= 12.0;
    }
    let folder = entry
        .uri
        .parent()
        .map(|p| display_path(&p))
        .unwrap_or_default();
    let left = left + 6.0;
    painter
        .with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right.max(left), rect.bottom()),
        ))
        .text(
            egui::pos2(left, rect.top() + 6.0),
            egui::Align2::LEFT_TOP,
            name,
            egui::FontId::proportional(tokens.font_size),
            name_color,
        );
    painter
        .with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(rect.right() - pad, rect.bottom()),
        ))
        .text(
            egui::pos2(left, rect.top() + 6.0 + tokens.font_size + 4.0),
            egui::Align2::LEFT_TOP,
            folder,
            egui::FontId::proportional(tokens.font_size - 1.0),
            muted,
        );
    (response, action)
}

fn search_result(s: &Session, i: usize) -> Option<fm_vfs::Entry> {
    s.search.as_ref().and_then(|x| x.result(i))
}

pub(crate) fn status_line(status: &fm_engine::search_cmds::SearchStatus) -> String {
    let found = match status.found {
        1 => "1 match".to_string(),
        n => format!("{n} matches"),
    };
    let scanned = format!("{} folders, {} files searched", status.dirs, status.files);
    if status.running {
        format!("{found} · {scanned} · {}", status.current)
    } else if status.truncated {
        format!("{found} · stopped at the result limit")
    } else if status.stopped {
        format!("{found} · stopped · {scanned}")
    } else if status.dirs == 0 && status.files == 0 {
        "Enter a name pattern and/or text, then Search.".to_string()
    } else {
        format!("{found} · {scanned}")
    }
}
