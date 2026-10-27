//! The Open With window: the installed applications, those registered for the file's type
//! first (the default one marked), filterable by name. Opening dispatches `file.open_with`.

use egui::{Context, Id, RichText};
use fm_engine::Session;
use serde_json::json;

use crate::theme::Tokens;

#[derive(Clone, Default)]
struct Draft {
    target: String,
    filter: String,
    selected: Option<String>,
    focus: bool,
}

fn draft_id() -> Id {
    Id::NULL.with("fm-open-with-draft")
}

/// Render the window while `s.open_with` is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(dialog) = s.open_with.as_ref() else {
        ctx.data_mut(|d| d.remove_temp::<Draft>(draft_id()));
        return;
    };
    let target = dialog.target.to_string();
    let name = dialog.target.file_name().unwrap_or_default().to_string();
    let choices = dialog.choices();
    let mut draft = ctx
        .data_mut(|d| d.get_temp::<Draft>(draft_id()))
        .filter(|d| d.target == target)
        .unwrap_or_else(|| Draft {
            target: target.clone(),
            focus: true,
            ..Draft::default()
        });
    if draft.selected.is_none()
        && let Some((_, list)) = &choices
    {
        draft.selected = list
            .iter()
            .find(|c| c.default)
            .or_else(|| list.iter().find(|c| c.recommended))
            .map(|c| c.id.clone());
    }
    let mut open = true;
    let mut launch: Option<String> = None;
    let mut cancel = false;
    egui::Window::new("Open With")
        .id(Id::NULL.with("fm-open-with"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([380.0, 440.0])
        .default_pos(ctx.content_rect().center() - egui::Vec2::from([380.0, 440.0]) / 2.0)
        .frame(crate::dialogs::frame(tokens))
        .show(ctx, |ui| {
            ui.label(RichText::new(format!("Open \"{name}\" with:")).color(tokens.text));
            let Some((mime, list)) = &choices else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tokens.muted("Looking for applications…"));
                });
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
                return;
            };
            if let Some(mime) = mime {
                ui.label(tokens.muted(mime.as_str()));
            }
            let caption = ui.label(tokens.muted("Applications"));
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut draft.filter)
                        .hint_text("Filter applications")
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(caption.id)
                .on_hover_text("Show only applications whose name contains this text");
            if draft.focus {
                response.request_focus();
                draft.focus = false;
            }
            let needle = draft.filter.to_lowercase();
            let shown: Vec<_> = list
                .iter()
                .filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle))
                .collect();
            let footer = tokens.row_height * 2.5;
            egui::ScrollArea::vertical()
                .id_salt("fm-open-with-list")
                .max_height((ui.available_height() - footer).max(tokens.row_height * 4.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut heading = None;
                    for choice in &shown {
                        let section = if choice.recommended {
                            "Recommended"
                        } else {
                            "Other applications"
                        };
                        if heading != Some(section) {
                            heading = Some(section);
                            ui.label(tokens.muted(section));
                        }
                        let label = if choice.default {
                            format!("{}  (default)", choice.name)
                        } else {
                            choice.name.clone()
                        };
                        let selected = draft.selected.as_deref() == Some(choice.id.as_str());
                        let r = ui.add(egui::Button::selectable(selected, label).truncate());
                        if r.clicked() {
                            draft.selected = Some(choice.id.clone());
                        }
                        if r.double_clicked() {
                            launch = Some(choice.id.clone());
                        }
                    }
                    if shown.is_empty() {
                        ui.label(tokens.muted("No applications found"));
                    }
                });
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.separator();
            ui.horizontal(|ui| {
                let some = draft.selected.is_some();
                if ui
                    .add_enabled(some, egui::Button::new("Open"))
                    .on_hover_text("Open the file with the selected application (Enter)")
                    .clicked()
                    || (enter && some)
                {
                    launch = draft.selected.clone();
                }
                if ui.button("Cancel").on_hover_text("Close (Esc)").clicked() {
                    cancel = true;
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    ctx.data_mut(|d| d.insert_temp(draft_id(), draft));
    if let Some(app) = launch {
        s.dispatch_or_notice("file.open_with", &json!({ "app": app, "uri": target }));
    } else if cancel || !open {
        s.dispatch_or_notice("file.open_with.close", &serde_json::Value::Null);
    }
}
