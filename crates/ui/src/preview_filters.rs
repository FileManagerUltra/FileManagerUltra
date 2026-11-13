//! Preferences > Preview > File types: a list of preview types beside the filename patterns
//! of the selected type, each list with a small icon toolbar.

use egui::{
    Align2, FontId, Id, Key, Margin, Rect, RichText, Sense, Stroke, TextEdit, Ui, WidgetInfo,
};
use fm_engine::{
    Session,
    preview_types::{FilterGroup, PreviewType},
};
use serde_json::json;

use crate::{
    dialog_style::{hint, section_heading},
    focus_nav::{self, Lock, Tab},
    media_icons::{self, Icon},
    theme::Tokens,
};

/// A pattern being edited in place; `row: None` is a new pattern at the end of the list.
#[derive(Clone)]
struct Editing {
    kind: PreviewType,
    row: Option<usize>,
    /// The pattern as saved; the edit is dropped when that row changes underneath it.
    saved: String,
    text: String,
    focus: bool,
}

const ROW_LIMIT: usize = 257;
/// Tall enough to list every type without scrolling.
const VISIBLE_ROWS: f32 = PreviewType::ALL.len() as f32;
const TYPES_WIDTH: f32 = 190.0;

fn selected_id() -> Id {
    Id::NULL.with("preview-filter-selected")
}

fn pattern_id(kind: PreviewType) -> Id {
    Id::NULL.with(("preview-filter-pattern", kind))
}

fn edit_id() -> Id {
    Id::NULL.with("preview-filter-edit")
}

fn error_id() -> Id {
    Id::NULL.with("preview-filter-error")
}

fn type_widget(kind: PreviewType) -> Id {
    Id::NULL.with(("preview-filter-type-row", kind))
}

fn pattern_widget(kind: PreviewType, row: usize) -> Id {
    Id::NULL.with(("preview-filter-pattern-row", kind, row))
}

fn apply(s: &mut Session, groups: &[FilterGroup]) -> Result<(), String> {
    let value = serde_json::to_value(groups).map_err(|error| error.to_string())?;
    s.dispatch("prefs.preview.filters", &json!({"groups": value}))
        .map_err(|error| error.to_string())
}

fn row_height(tokens: &Tokens) -> f32 {
    tokens.row_height.max(tokens.font_size + 10.0)
}

pub(crate) fn show(ui: &mut Ui, s: &mut Session, tokens: &Tokens) {
    section_heading(ui, tokens, "File types");
    hint(
        ui,
        tokens,
        "Select a file type to see the filename patterns that open it in that preview.",
    );
    let groups = s.prefs.preview_filters.0.clone();
    let Some(first) = groups.first().map(|g| g.kind) else {
        return;
    };
    let selected = ui
        .ctx()
        .data_mut(|data| data.get_temp::<PreviewType>(selected_id()))
        .filter(|kind| groups.iter().any(|g| g.kind == *kind))
        .unwrap_or(first);
    let index = groups.iter().position(|g| g.kind == selected).unwrap_or(0);

    let stacked = ui.available_width() < 2.0 * TYPES_WIDTH + 40.0;
    if stacked {
        types_panel(ui, s, tokens, &groups, index, ui.available_width());
        ui.add_space(6.0);
        patterns_panel(ui, s, tokens, &groups, index);
    } else {
        ui.horizontal_top(|ui| {
            types_panel(ui, s, tokens, &groups, index, TYPES_WIDTH);
            patterns_panel(ui, s, tokens, &groups, index);
        });
    }

    if let Some(error) = ui
        .ctx()
        .data_mut(|data| data.get_temp::<String>(error_id()))
    {
        ui.label(RichText::new(error).color(tokens.error));
    }
    hint(
        ui,
        tokens,
        "Patterns match whole filenames and ignore case: * is any text, ? one character, [0-9] one character from a range; without these it is an exact name such as dmesg. Files that match no pattern are recognised by their content. HTML and CSV open as text.",
    );
    ui.add_space(4.0);
    if ui.button("Restore default file types").clicked()
        && s.dispatch_or_notice("prefs.preview.filters", &json!({"reset": true}))
    {
        ui.ctx().data_mut(|data| {
            data.remove::<Editing>(edit_id());
            data.remove::<String>(error_id());
            for kind in PreviewType::ALL {
                data.remove::<usize>(pattern_id(kind));
            }
        });
    }
}

/// Caption, framed toolbar and a fixed-height list; returns the list closure's result.
fn panel<R>(
    ui: &mut Ui,
    tokens: &Tokens,
    width: f32,
    caption: &str,
    toolbar: impl FnOnce(&mut Ui) -> R,
    list: impl FnOnce(&mut Ui),
) -> R {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(caption).color(tokens.text));
        egui::Frame::new()
            .fill(tokens.bg)
            .stroke(Stroke::new(tokens.border_width, tokens.border))
            .corner_radius(tokens.corner_radius)
            .show(ui, |ui| {
                ui.set_width(width - 2.0 * tokens.border_width);
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                let out = egui::Frame::new()
                    .inner_margin(Margin::symmetric(4, 3))
                    .show(ui, |ui| ui.horizontal(toolbar).inner)
                    .inner;
                let (line, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), tokens.border_width),
                    Sense::hover(),
                );
                ui.painter().rect_filled(line, 0.0, tokens.border);
                let height = VISIBLE_ROWS * row_height(tokens);
                egui::ScrollArea::vertical()
                    .id_salt(("preview-filter-list", caption.split(' ').next()))
                    .auto_shrink([false, false])
                    .max_height(height)
                    .min_scrolled_height(height)
                    .show(ui, |ui| {
                        ui.set_min_height(height);
                        list(ui);
                    });
                out
            })
            .inner
    })
    .inner
}

/// One full-width list row. Draws `text` (and an optional muted `detail` at the right).
/// Only the list's current row is `focusable`: each list is a single Tab stop.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    tokens: &Tokens,
    id: Id,
    focusable: bool,
    text: &str,
    detail: Option<&str>,
    selected: bool,
    accessible: String,
) -> egui::Response {
    let size = egui::vec2(ui.available_width(), row_height(tokens));
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let sense = if focusable {
        Sense::click()
    } else {
        Sense::CLICK
    };
    let response = ui.interact(rect, id, sense);
    // Clicking a row gives it the keyboard, so the arrows, Delete and F2 work right away.
    if response.clicked() {
        focus_nav::focus_later(ui.ctx(), id);
    }
    response.widget_info(|| {
        WidgetInfo::selected(egui::Role::ListBoxOption, true, selected, &accessible)
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 0.0, tokens.selection_bg);
        } else if response.hovered() {
            painter.rect_filled(rect, 0.0, tokens.hover_bg);
        }
        let color = if selected {
            tokens.selection_text
        } else {
            tokens.text
        };
        let font = FontId::proportional(tokens.font_size);
        let mut right = rect.right() - 10.0;
        if let Some(detail) = detail {
            let muted = if selected {
                tokens.selection_text
            } else {
                tokens.text_muted
            };
            let shown = painter.text(
                egui::pos2(right, rect.center().y),
                Align2::RIGHT_CENTER,
                detail,
                FontId::proportional(tokens.font_size - 1.0),
                muted,
            );
            right = shown.left() - 8.0;
        }
        let clip = Rect::from_min_max(rect.min, egui::pos2(right, rect.max.y));
        painter.with_clip_rect(clip).text(
            egui::pos2(rect.left() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            font,
            color,
        );
    }
    response
}

fn types_panel(
    ui: &mut Ui,
    s: &mut Session,
    tokens: &Tokens,
    groups: &[FilterGroup],
    index: usize,
    width: f32,
) {
    let label = groups.get(index).map_or("", |g| g.kind.label());
    let mut moved: Option<usize> = None;
    let mut select: Option<PreviewType> = None;
    panel(
        ui,
        tokens,
        width,
        "Types, checked top to bottom",
        |ui| {
            let up = media_icons::button(
                ui,
                tokens,
                Icon::MoveUp,
                &format!("Move {label} up: check it earlier"),
                index > 0,
                false,
            );
            if up.clicked() {
                moved = index.checked_sub(1);
            }
            let down = media_icons::button(
                ui,
                tokens,
                Icon::MoveDown,
                &format!("Move {label} down: check it later"),
                index + 1 < groups.len(),
                false,
            );
            if down.clicked() {
                moved = Some(index + 1);
            }
        },
        |ui| {
            let ctx = ui.ctx().clone();
            for (i, group) in groups.iter().enumerate() {
                let id = type_widget(group.kind);
                let focusable = i == index;
                if focusable {
                    // Into the patterns, at the remembered selection or the first one.
                    let enter_patterns = |select: &mut Option<PreviewType>| {
                        let remembered = pattern_id(group.kind);
                        let len = group.patterns.len();
                        let row = ctx
                            .data(|data| data.get_temp::<usize>(remembered))
                            .filter(|row| *row < len)
                            .unwrap_or(0);
                        ctx.data_mut(|data| data.insert_temp(remembered, row));
                        *select = Some(group.kind);
                        focus_nav::focus_later(&ctx, pattern_widget(group.kind, row));
                    };
                    let keys = focus_nav::keys(
                        ui,
                        id,
                        Lock {
                            tab: true,
                            arrows: true,
                        },
                    );
                    match keys.tab {
                        Some(Tab::Forward) if !group.patterns.is_empty() => {
                            enter_patterns(&mut select);
                        }
                        Some(tab) => focus_nav::pass_tab(ui, tab),
                        None => {}
                    }
                    let step = match keys.arrow {
                        Some(Key::ArrowDown) => Some(i + 1),
                        Some(Key::ArrowUp) => i.checked_sub(1),
                        Some(Key::Home) => Some(0),
                        Some(Key::End) => Some(groups.len().saturating_sub(1)),
                        Some(Key::ArrowRight) if !group.patterns.is_empty() => {
                            enter_patterns(&mut select);
                            None
                        }
                        _ => None,
                    };
                    if let Some(next) = step.filter(|n| *n != i).and_then(|n| groups.get(n)) {
                        select = Some(next.kind);
                        focus_nav::focus_later(&ctx, type_widget(next.kind));
                    }
                }
                let count = group.patterns.len().to_string();
                let response = row(
                    ui,
                    tokens,
                    id,
                    focusable,
                    group.kind.label(),
                    Some(&count),
                    i == index,
                    group.kind.label().to_string(),
                );
                let response = response.on_hover_text(match group.patterns.len() {
                    0 => "No filename patterns".to_string(),
                    1 => "1 filename pattern".to_string(),
                    n => format!("{n} filename patterns"),
                });
                focus_nav::claim(ui, &response);
                if response.clicked() {
                    select = Some(group.kind);
                }
            }
        },
    );
    if let Some(kind) = select {
        ui.ctx()
            .data_mut(|data| data.insert_temp(selected_id(), kind));
    }
    if let Some(target) = moved {
        let mut next = groups.to_vec();
        if target < next.len() {
            next.swap(index, target);
            if let Err(error) = apply(s, &next) {
                s.notice = Some(error);
            }
        }
    }
}

fn patterns_panel(
    ui: &mut Ui,
    s: &mut Session,
    tokens: &Tokens,
    groups: &[FilterGroup],
    index: usize,
) {
    let Some(current) = groups.get(index) else {
        return;
    };
    let kind = current.kind;
    let label = kind.label();
    let ctx = ui.ctx().clone();
    let mut chosen = ctx
        .data_mut(|data| data.get_temp::<usize>(pattern_id(kind)))
        .filter(|row| *row < current.patterns.len());
    let mut editing = ctx
        .data_mut(|data| data.get_temp::<Editing>(edit_id()))
        .filter(|e| {
            e.kind == kind
                && e.row
                    .is_none_or(|row| current.patterns.get(row) == Some(&e.saved))
        });
    let mut start: Option<Option<usize>> = None;
    let mut remove: Option<usize> = None;
    let mut commit: Option<String> = None;
    let mut cancel = false;
    // Enter or Escape ended an edit: the keyboard goes back to the list afterwards.
    let mut by_key = false;
    let mut removed_by_key = false;
    let width = ui.available_width();
    let toolbar_chosen = chosen;
    let (add, gone, edit) = panel(
        ui,
        tokens,
        width,
        &format!("Filename patterns for {label}"),
        |ui| {
            let has = toolbar_chosen.is_some();
            let add = media_icons::button(
                ui,
                tokens,
                Icon::Add,
                &format!("Add {label} pattern"),
                true,
                false,
            );
            let gone = media_icons::button(
                ui,
                tokens,
                Icon::Remove,
                "Remove selected pattern",
                has,
                false,
            );
            let edit =
                media_icons::button(ui, tokens, Icon::Edit, "Edit selected pattern", has, false);
            if !current.patterns.is_empty() {
                // The list has keys for these (Insert, Delete, F2): keep Tab moving between
                // the lists instead of stopping on every toolbar button.
                ui.memory_mut(|m| {
                    for response in [&add, &gone, &edit] {
                        m.ignore_focus(response.id);
                    }
                });
            }
            (add.clicked(), gone.clicked(), edit.clicked())
        },
        |ui| {
            let rows = current.patterns.len()
                + usize::from(editing.as_ref().is_some_and(|e| e.row.is_none()));
            if rows == 0 {
                ui.vertical_centered(|ui| {
                    ui.add_space((VISIBLE_ROWS / 2.0 - 0.5) * row_height(tokens));
                    ui.label(
                        RichText::new("No patterns: recognised by content only")
                            .color(tokens.text_muted),
                    );
                });
            }
            for row_index in 0..rows {
                let pattern = current.patterns.get(row_index);
                let is_edit = editing.as_ref().is_some_and(|e| match e.row {
                    Some(r) => r == row_index,
                    None => pattern.is_none(),
                });
                if is_edit {
                    if let Some(edit) = editing.as_mut() {
                        let (done, keep) = edit_row(ui, tokens, edit);
                        if done {
                            by_key = ui
                                .input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape));
                            match keep {
                                Some(text) if text != edit.saved => commit = Some(text),
                                _ => cancel = true,
                            }
                        }
                    }
                    continue;
                }
                let Some(pattern) = pattern else { continue };
                let id = pattern_widget(kind, row_index);
                // The list is one Tab stop: the selected row, or the first one.
                let focusable = row_index == chosen.unwrap_or(0);
                if focusable {
                    let keys = focus_nav::keys(
                        ui,
                        id,
                        Lock {
                            tab: true,
                            arrows: true,
                        },
                    );
                    match keys.tab {
                        Some(Tab::Back) => focus_nav::focus_later(ui.ctx(), type_widget(kind)),
                        Some(Tab::Forward) => focus_nav::pass_tab(ui, Tab::Forward),
                        None => {}
                    }
                    let step = match keys.arrow {
                        Some(Key::ArrowDown) => Some(row_index + 1),
                        Some(Key::ArrowUp) => row_index.checked_sub(1),
                        Some(Key::Home) => Some(0),
                        Some(Key::End) => Some(current.patterns.len().saturating_sub(1)),
                        Some(Key::ArrowLeft) => {
                            focus_nav::focus_later(ui.ctx(), type_widget(kind));
                            None
                        }
                        _ => None,
                    };
                    if let Some(next) =
                        step.filter(|n| *n < current.patterns.len() && *n != row_index)
                    {
                        chosen = Some(next);
                        focus_nav::focus_later(ui.ctx(), pattern_widget(kind, next));
                    }
                }
                let selected = chosen == Some(row_index);
                let response = row(
                    ui,
                    tokens,
                    id,
                    focusable,
                    pattern,
                    None,
                    selected,
                    pattern.clone(),
                )
                .on_hover_text(
                    "Click to select; click again, Enter or F2 to edit; Delete to remove",
                );
                focus_nav::claim(ui, &response);
                if response.clicked() || response.double_clicked() {
                    if selected {
                        start = Some(Some(row_index));
                    }
                    chosen = Some(row_index);
                }
                if response.has_focus() {
                    // Arriving by Shift+Tab focuses the row without selecting it.
                    chosen = chosen.or(Some(row_index));
                    ui.input(|input| {
                        if input.key_pressed(Key::Delete) {
                            remove = Some(row_index);
                            removed_by_key = true;
                        }
                        if input.key_pressed(Key::F2) {
                            start = Some(Some(row_index));
                        }
                        if input.key_pressed(Key::Insert) {
                            start = Some(None);
                        }
                    });
                }
            }
        },
    );

    if add {
        start = Some(None);
    }
    if gone {
        remove = toolbar_chosen;
    }
    if edit {
        start = toolbar_chosen.map(Some);
    }
    let mut error: Option<String> = None;
    if let (Some(text), Some(edit)) = (commit, editing.as_ref()) {
        let mut next = groups.to_vec();
        if let Some(group) = next.get_mut(index) {
            match edit.row {
                Some(row) => {
                    if let Some(slot) = group.patterns.get_mut(row) {
                        *slot = text;
                    }
                }
                None => group.patterns.push(text),
            }
        }
        match apply(s, &next) {
            Ok(()) => {
                chosen = Some(edit.row.unwrap_or(current.patterns.len()));
                cancel = true;
            }
            Err(message) => {
                error = Some(message);
                if let Some(edit) = editing.as_mut() {
                    edit.focus = true;
                }
            }
        }
    }
    if cancel {
        let edited = editing.as_ref().and_then(|e| e.row);
        editing = None;
        if by_key {
            focus_nav::focus_later(
                ui.ctx(),
                match chosen.or(edited) {
                    Some(row) => pattern_widget(kind, row),
                    None => type_widget(kind),
                },
            );
        }
    }
    if let Some(row) = remove {
        let mut next = groups.to_vec();
        if let Some(group) = next.get_mut(index)
            && row < group.patterns.len()
        {
            group.patterns.remove(row);
            let left = group.patterns.len();
            match apply(s, &next) {
                Ok(()) => {
                    chosen = (left > 0).then(|| row.min(left - 1));
                    if removed_by_key {
                        focus_nav::focus_later(
                            ui.ctx(),
                            match chosen {
                                Some(row) => pattern_widget(kind, row),
                                None => type_widget(kind),
                            },
                        );
                    }
                }
                Err(message) => error = Some(message),
            }
        }
    }
    if let Some(row) = start {
        let saved = row
            .and_then(|r| current.patterns.get(r))
            .cloned()
            .unwrap_or_default();
        editing = Some(Editing {
            kind,
            row,
            text: saved.clone(),
            saved,
            focus: true,
        });
    }
    ctx.data_mut(|data| {
        match editing {
            Some(edit) => {
                data.insert_temp(edit_id(), edit);
            }
            None => data.remove::<Editing>(edit_id()),
        }
        match chosen {
            Some(row) => {
                data.insert_temp(pattern_id(kind), row);
            }
            None => data.remove::<usize>(pattern_id(kind)),
        }
        if let Some(message) = error {
            data.insert_temp(error_id(), message);
        } else if start.is_some() || remove.is_some() || cancel {
            data.remove::<String>(error_id());
        }
    });
}

/// The in-place editor. Returns `(finished, Some(text) to save | None to cancel)`.
fn edit_row(ui: &mut Ui, tokens: &Tokens, edit: &mut Editing) -> (bool, Option<String>) {
    ui.spacing_mut().interact_size.y = row_height(tokens);
    let response = ui.add_sized(
        [ui.available_width(), row_height(tokens)],
        TextEdit::singleline(&mut edit.text)
            .hint_text("Pattern, for example *.trace")
            .char_limit(ROW_LIMIT)
            .margin(Margin::symmetric(9, 2)),
    );
    let response = response.on_hover_text("Enter saves, Escape cancels");
    if std::mem::take(&mut edit.focus) {
        response.request_focus();
        response.scroll_to_me(None);
    }
    if !response.lost_focus() {
        return (false, None);
    }
    let text = edit.text.trim().to_string();
    if ui.input(|input| input.key_pressed(Key::Escape)) || (edit.row.is_none() && text.is_empty()) {
        return (true, None);
    }
    (true, Some(text))
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
            .with_size(egui::vec2(850.0, 900.0))
            .build_ui_state(
                move |ui, s| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    crate::dialog_style::apply(ui, &tokens);
                    show(ui, s, &tokens);
                },
                Session::new().expect("session"),
            )
    }

    fn type_text(h: &mut Harness<'_, Session>, text: &str, enter: bool) {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.key_press(egui::Key::Backspace);
        h.run();
        h.get_by_role(Role::TextInput).type_text(text);
        if enter {
            h.key_press(egui::Key::Enter);
        }
        h.run();
    }

    fn patterns(h: &Harness<'_, Session>, kind: PreviewType) -> Vec<String> {
        h.state().prefs.preview_filters.patterns(kind).to_vec()
    }

    fn position(h: &Harness<'_, Session>, kind: PreviewType) -> Option<usize> {
        h.state()
            .prefs
            .preview_filters
            .0
            .iter()
            .position(|g| g.kind == kind)
    }

    #[test]
    fn arrow_keys_move_the_pattern_selection() {
        let mut h = harness(Tokens::dark());
        let selected = |h: &Harness<'_, Session>, label: &str| {
            h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
        };
        h.get_by_label("*.log").click();
        h.run();
        assert!(selected(&h, "*.log"));
        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert!(selected(&h, "*.log.gz"));
        assert!(!selected(&h, "*.log"));
        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert!(selected(&h, "*.log.[0-9]*"));
        h.key_press(egui::Key::ArrowUp);
        h.run();
        assert!(selected(&h, "*.log.gz"));
        h.get_by_label("Remove selected pattern").click();
        h.run();
        assert!(
            !h.state()
                .prefs
                .preview_filters
                .patterns(PreviewType::Logs)
                .iter()
                .any(|p| p == "*.log.gz"),
            "the keyboard selection is what Remove acts on"
        );
        // Left into the types, Down to the next type, Right back into its patterns.
        h.get_by_label("*.log").click();
        h.run();
        h.key_press(egui::Key::ArrowLeft);
        h.run();
        h.key_press(egui::Key::ArrowDown);
        h.run();
        h.get_by_label("Filename patterns for Images");
        h.key_press(egui::Key::ArrowRight);
        h.run();
        assert!(selected(&h, "*.png"), "Right selects the first pattern");
        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert!(selected(&h, "*.jpg"));
        assert!(!selected(&h, "*.jpeg"), "one step per key press");
        // Shift+Tab goes back to the types, Tab forward into the patterns and then on.
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        h.run();
        h.key_press(egui::Key::ArrowDown);
        h.run();
        h.get_by_label("Filename patterns for Video");
        h.key_press(egui::Key::Tab);
        h.run();
        assert!(
            selected(&h, "*.mp4"),
            "Tab enters the patterns at the first one"
        );
        h.key_press(egui::Key::ArrowDown);
        h.run();
        assert!(selected(&h, "*.m4v"));
        h.key_press(egui::Key::Tab);
        h.run();
        assert!(
            h.get_by_label("Restore default file types")
                .accesskit_node()
                .is_focused(),
            "Tab leaves the lists instead of bouncing between them"
        );
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        h.run();
        assert!(
            h.get_by_label("*.m4v").accesskit_node().is_focused(),
            "Shift+Tab comes back to the selected pattern"
        );
        h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        h.run();
        h.key_press(egui::Key::ArrowUp);
        h.run();
        h.get_by_label("Filename patterns for Images");
        // Keys for the toolbar actions: Insert adds, F2 edits, Delete removes.
        h.key_press(egui::Key::Tab);
        h.run();
        h.key_press(egui::Key::Insert);
        h.run();
        h.get_by_role(Role::TextInput).type_text("*.heic");
        h.key_press(egui::Key::Enter);
        h.run();
        assert!(patterns(&h, PreviewType::Images).contains(&"*.heic".to_string()));
        assert!(
            h.get_by_label("*.heic").accesskit_node().is_focused(),
            "after Enter the keyboard is back on the list, on the new pattern"
        );
        h.key_press(egui::Key::Delete);
        h.run();
        assert!(!patterns(&h, PreviewType::Images).contains(&"*.heic".to_string()));
        assert!(
            h.get_by_label("*.ico").accesskit_node().is_focused(),
            "after Delete the neighbouring pattern has the keyboard"
        );
    }

    #[test]
    fn select_type_then_add_edit_remove_reorder_and_restore() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut h = harness(tokens);
            let original = h.state().prefs.preview_filters.clone();
            // The first type is selected and its patterns are listed.
            h.get_by_label("Filename patterns for Logs");
            h.get_by_label("syslog");
            assert!(
                h.get_by_label("Remove selected pattern")
                    .accesskit_node()
                    .is_disabled()
            );

            h.get_by_label("PDF").click();
            h.run();
            h.get_by_label("Filename patterns for PDF");

            // Select, then click again to edit in place.
            h.get_by_label("*.pdf").click();
            h.run();
            assert!(
                !h.get_by_label("Remove selected pattern")
                    .accesskit_node()
                    .is_disabled()
            );
            h.get_by_label("*.pdf").click();
            h.run();
            type_text(&mut h, "*.pdfx", true);
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx"]);
            assert_eq!(h.query_all_by_role(Role::TextInput).count(), 0);

            // Empty edits are rejected and the field stays open with the error.
            h.get_by_label("Edit selected pattern").click();
            h.run();
            type_text(&mut h, "", true);
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx"]);
            h.get_by_label_contains("cannot be empty");
            // Escape abandons the edit.
            type_text(&mut h, "*.nope", false);
            h.key_press(egui::Key::Escape);
            h.run();
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx"]);
            assert_eq!(h.query_all_by_role(Role::TextInput).count(), 0);

            // + opens an empty row at the end.
            h.get_by_label("Add PDF pattern").click();
            h.run();
            type_text(&mut h, "*.doc", true);
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.pdfx", "*.doc"]);

            let before = h.state().prefs.preview_filters.clone();
            h.get_by_label("Add PDF pattern").click();
            h.run();
            type_text(&mut h, "a/b", true);
            assert_eq!(h.state().prefs.preview_filters, before);
            h.get_by_label_contains("without paths");
            h.key_press(egui::Key::Escape);
            h.run();

            h.get_by_label("*.pdfx").click();
            h.run();
            h.get_by_label("Remove selected pattern").click();
            h.run();
            assert_eq!(patterns(&h, PreviewType::Pdf), ["*.doc"]);
            h.get_by_label("Remove selected pattern").click();
            h.run();
            assert!(patterns(&h, PreviewType::Pdf).is_empty());
            h.get_by_label("No patterns: recognised by content only");

            let pdf = position(&h, PreviewType::Pdf).expect("pdf");
            h.get_by_label("Move PDF up: check it earlier").click();
            h.run();
            assert_eq!(position(&h, PreviewType::Pdf), Some(pdf - 1));
            h.get_by_label("Move PDF down: check it later").click();
            h.run();
            assert_eq!(position(&h, PreviewType::Pdf), Some(pdf));

            h.get_by_label("Logs").click();
            h.run();
            assert!(
                h.get_by_label("Move Logs up: check it earlier")
                    .accesskit_node()
                    .is_disabled()
            );

            h.get_by_label("Restore default file types").click();
            h.run();
            assert_eq!(h.state().prefs.preview_filters, original);
        }
    }
}
