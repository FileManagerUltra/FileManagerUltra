use egui::{Id, Sense, TextEdit, Ui, WidgetInfo};
use fm_engine::{
    Session,
    preview_types::{FilterGroup, PreviewType},
};
use serde_json::json;

use crate::{
    dialog_style::{hint, section_heading},
    theme::Tokens,
};

#[derive(Clone, Default)]
struct Draft {
    saved: String,
    text: String,
    error: Option<String>,
}

fn epoch_id() -> Id {
    Id::NULL.with("preview-filter-generation")
}

fn open_id(kind: PreviewType) -> Id {
    Id::NULL.with(("preview-filter-open", kind))
}

/// Drafts are keyed by generation so any structural change starts every field afresh.
fn generation(ui: &Ui) -> u64 {
    ui.ctx()
        .data_mut(|data| data.get_temp::<u64>(epoch_id()).unwrap_or(0))
}

fn apply(ui: &Ui, s: &mut Session, groups: &[FilterGroup]) -> Result<(), String> {
    let value = serde_json::to_value(groups).map_err(|error| error.to_string())?;
    s.dispatch("prefs.preview.filters", &json!({"groups": value}))
        .map_err(|error| error.to_string())?;
    let next = generation(ui) + 1;
    ui.ctx().data_mut(|data| data.insert_temp(epoch_id(), next));
    Ok(())
}

fn labelled(response: egui::Response, label: String) -> egui::Response {
    response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    response
}

pub(crate) fn show(ui: &mut Ui, s: &mut Session, tokens: &Tokens) {
    section_heading(ui, tokens, "File type filters");
    hint(
        ui,
        tokens,
        "Each row is one case-insensitive filename pattern: * ? [a-z] or an exact name. Types are checked from top to bottom and the first match wins; use Up and Down to change priority.",
    );
    hint(
        ui,
        tokens,
        "Files matching no row use content detection. HTML and CSV use text previews; filters do not add new decoders.",
    );
    ui.add_space(4.0);
    let groups = s.prefs.preview_filters.0.clone();
    for index in 0..groups.len() {
        group(ui, s, tokens, &groups, index);
    }
    ui.add_space(6.0);
    if ui.button("Restore default file filters").clicked() {
        s.dispatch_or_notice("prefs.preview.filters", &json!({"reset": true}));
        let next = generation(ui) + 1;
        ui.ctx().data_mut(|data| data.insert_temp(epoch_id(), next));
    }
}

fn group(ui: &mut Ui, s: &mut Session, tokens: &Tokens, groups: &[FilterGroup], index: usize) {
    let Some(current) = groups.get(index) else {
        return;
    };
    let kind = current.kind;
    let label = kind.label();
    let open = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(open_id(kind)).unwrap_or(false));
    let mut toggled = false;
    let mut moved: Option<usize> = None;
    ui.horizontal(|ui| {
        let title =
            ui.add(egui::Label::new(egui::RichText::new(label).strong()).sense(Sense::click()));
        toggled |= title.clicked();
        let count = current.patterns.len();
        ui.label(
            egui::RichText::new(match count {
                0 => "no patterns".to_string(),
                1 => "1 pattern".to_string(),
                n => format!("{n} patterns"),
            })
            .color(tokens.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let shown = if open { "Hide" } else { "Show" };
            toggled |= labelled(ui.button(shown), format!("{shown} {label} patterns")).clicked();
            let down = ui.add_enabled(index + 1 < groups.len(), egui::Button::new("Down"));
            if labelled(down, format!("Move {label} down")).clicked() {
                moved = Some(index + 1);
            }
            let up = ui.add_enabled(index > 0, egui::Button::new("Up"));
            if labelled(up, format!("Move {label} up")).clicked() {
                moved = index.checked_sub(1);
            }
        });
    });
    if let Some(target) = moved {
        let mut next = groups.to_vec();
        if target < next.len() {
            next.swap(index, target);
            if let Err(error) = apply(ui, s, &next) {
                s.notice = Some(error);
            }
        }
    }
    if toggled {
        ui.ctx()
            .data_mut(|data| data.insert_temp(open_id(kind), !open));
    }
    if open {
        ui.indent(("preview-filter-body", kind), |ui| {
            rows(ui, s, tokens, groups, index)
        });
    }
    ui.add_space(4.0);
}

fn rows(ui: &mut Ui, s: &mut Session, tokens: &Tokens, groups: &[FilterGroup], index: usize) {
    let Some(current) = groups.get(index) else {
        return;
    };
    let (kind, epoch) = (current.kind, generation(ui));
    if current.patterns.is_empty() {
        hint(
            ui,
            tokens,
            "No patterns: this type is only recognised by content.",
        );
    }
    for (row, pattern) in current.patterns.iter().enumerate() {
        let id = Id::NULL.with(("preview-filter-row", kind, row, epoch));
        let mut draft = ui
            .ctx()
            .data_mut(|data| data.get_temp::<Draft>(id))
            .filter(|draft| &draft.saved == pattern)
            .unwrap_or_else(|| Draft {
                saved: pattern.clone(),
                text: pattern.clone(),
                error: None,
            });
        let mut remove = false;
        ui.horizontal(|ui| {
            let edit = ui.add(
                TextEdit::singleline(&mut draft.text)
                    .desired_width(300.0)
                    .char_limit(257),
            );
            let edit = edit.on_hover_text("Saved on Enter or when leaving the field");
            if edit.changed() {
                draft.error = None;
            }
            if edit.lost_focus() && draft.text != draft.saved {
                let mut next = groups.to_vec();
                if let Some(slot) = next.get_mut(index).and_then(|g| g.patterns.get_mut(row)) {
                    *slot = draft.text.trim().to_string();
                }
                if let Err(error) = apply(ui, s, &next) {
                    draft.error = Some(error);
                }
            }
            remove = labelled(
                ui.button("Remove"),
                format!("Remove {pattern} from {}", kind.label()),
            )
            .clicked();
        });
        if let Some(error) = &draft.error {
            ui.label(egui::RichText::new(error).color(tokens.error));
        }
        if remove {
            let mut next = groups.to_vec();
            if let Some(group) = next.get_mut(index) {
                group.patterns.remove(row);
            }
            if let Err(error) = apply(ui, s, &next) {
                draft.error = Some(error);
            }
        }
        ui.ctx().data_mut(|data| data.insert_temp(id, draft));
    }
    add_row(ui, s, tokens, groups, index, epoch);
}

fn add_row(
    ui: &mut Ui,
    s: &mut Session,
    tokens: &Tokens,
    groups: &[FilterGroup],
    index: usize,
    epoch: u64,
) {
    let Some(current) = groups.get(index) else {
        return;
    };
    let kind = current.kind;
    let id = Id::NULL.with(("preview-filter-new", kind, epoch));
    let mut draft = ui
        .ctx()
        .data_mut(|data| data.get_temp::<Draft>(id))
        .unwrap_or_default();
    let mut submit = false;
    ui.horizontal(|ui| {
        let edit = ui.add(
            TextEdit::singleline(&mut draft.text)
                .hint_text("New pattern, for example *.trace")
                .desired_width(300.0)
                .char_limit(257),
        );
        if edit.changed() {
            draft.error = None;
        }
        let enter = edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let add = labelled(ui.button("Add"), format!("Add {} pattern", kind.label()));
        submit = (add.clicked() || enter) && !draft.text.trim().is_empty();
    });
    if submit {
        let mut next = groups.to_vec();
        if let Some(group) = next.get_mut(index) {
            group.patterns.push(draft.text.trim().to_string());
        }
        match apply(ui, s, &next) {
            Ok(()) => {
                ui.ctx().data_mut(|data| data.remove::<Draft>(id));
                return;
            }
            Err(error) => draft.error = Some(error),
        }
    }
    if let Some(error) = &draft.error {
        ui.label(egui::RichText::new(error).color(tokens.error));
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, draft));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    fn harness(tokens: Tokens) -> Harness<'static, Session> {
        Harness::builder()
            .with_size(egui::vec2(850.0, 1400.0))
            .build_ui_state(
                move |ui, s| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    crate::dialog_style::apply(ui, &tokens);
                    show(ui, s, &tokens);
                },
                Session::new().expect("session"),
            )
    }

    fn open(h: &mut Harness<'_, Session>, kind: PreviewType) {
        h.ctx.data_mut(|data| data.insert_temp(open_id(kind), true));
        h.run();
    }

    fn type_in(h: &mut Harness<'_, Session>, field: usize, text: &str, enter: bool) {
        h.get_all_by_role(Role::TextInput)
            .nth(field)
            .expect("field")
            .click();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.key_press(egui::Key::Backspace);
        h.run();
        h.get_all_by_role(Role::TextInput)
            .nth(field)
            .expect("field")
            .type_text(text);
        if enter {
            h.key_press(egui::Key::Enter);
        }
        h.run();
    }

    fn patterns(h: &Harness<'_, Session>, kind: PreviewType) -> Vec<String> {
        h.state().prefs.preview_filters.patterns(kind).to_vec()
    }

    #[test]
    fn rows_can_be_edited_added_removed_reordered_and_restored() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut h = harness(tokens);
            let original = h.state().prefs.preview_filters.clone();
            assert!(
                h.query_by_label("Remove *.pdf from PDF").is_none(),
                "groups start collapsed"
            );
            open(&mut h, PreviewType::Pdf);
            h.get_by_label("Remove *.pdf from PDF");

            type_in(&mut h, 0, "*.pdfx", true);
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx"]);

            type_in(&mut h, 0, "", true);
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx"]);
            h.get_by_label_contains("cannot be empty");

            type_in(&mut h, 1, "*.doc", true);
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx", "*.doc"]);

            let before = h.state().prefs.preview_filters.clone();
            type_in(&mut h, 2, "a/b", false);
            h.get_by_label("Add PDF pattern").click();
            h.run();
            assert_eq!(h.state().prefs.preview_filters, before);
            h.get_by_label_contains("without paths");
            type_in(&mut h, 2, "a;b", true);
            assert_eq!(h.state().prefs.preview_filters, before);

            h.get_by_label("Remove *.pdfx from PDF").click();
            h.run();
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.doc"]);
            h.get_by_label("Remove *.doc from PDF").click();
            h.run();
            assert!(patterns(&h, PreviewType::Pdf).is_empty());
            h.get_by_label("No patterns: this type is only recognised by content.");

            assert!(
                h.get_by_label("Move Logs up")
                    .accesskit_node()
                    .is_disabled()
            );
            let position = |h: &Harness<'_, Session>, kind| {
                h.state()
                    .prefs
                    .preview_filters
                    .0
                    .iter()
                    .position(|g| g.kind == kind)
            };
            let pdf = position(&h, PreviewType::Pdf).expect("pdf");
            h.get_by_label("Move PDF up").click();
            h.run();
            assert_eq!(position(&h, PreviewType::Pdf), Some(pdf - 1));
            h.get_by_label("Move PDF down").click();
            h.run();
            assert_eq!(position(&h, PreviewType::Pdf), Some(pdf));

            h.get_by_label("Restore default file filters").click();
            h.run();
            assert_eq!(h.state().prefs.preview_filters, original);
            assert_eq!(h.get_all_by_role(Role::TextInput).count(), 2);
            h.get_by_label("Remove *.pdf from PDF");
        }
    }
}
