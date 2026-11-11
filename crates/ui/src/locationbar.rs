//! Pane location bar: a clickable path that turns into an editor with directory
//! completion and history (`nav.goto`). All navigation goes through engine commands.

use egui::{Id, Key, Modifiers, RichText, Ui};
use fm_engine::Session;
use fm_engine::goto_cmds::SuggestionKind;
use fm_layout::PaneId;

use crate::theme::Tokens;

const MAX_ROWS: usize = 10;

fn popup_rect_id(pane: PaneId) -> Id {
    Id::NULL.with(("fm-locationbar-popup", pane))
}

/// Whether `pane`'s location is being edited.
pub fn is_editing(s: &Session, pane: PaneId) -> bool {
    s.path_edit.as_ref().is_some_and(|e| e.pane == pane)
}

/// The path as a header label. Clicking it starts editing.
pub fn show_label(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens, focused: bool) {
    let Some(path) = s.layout.state(pane).map(|p| {
        let loc = &p.location;
        if loc.scheme() == fm_vfs::Scheme::File {
            fm_engine::item_cmds::display_path(loc)
        } else if matches!(
            loc.scheme(),
            fm_vfs::Scheme::Network | fm_vfs::Scheme::Servers
        ) {
            loc.to_string()
        } else {
            format!("{}:{}", loc.authority(), loc.path_str())
        }
    }) else {
        return;
    };
    let text = RichText::new(&path).font(tokens.font());
    let text = if focused {
        text.color(tokens.accent_text).strong()
    } else {
        text.color(tokens.text_muted)
    };
    // The whole remaining header width is the click target, not just the text.
    let size = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.add(egui::Label::new(text).truncate()),
    );
    let response = response.on_hover_text(format!("{path}\nClick or Ctrl+L to type a path"));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if response.clicked() {
        let _ = s.layout.focus_pane(pane);
        s.dispatch_or_notice("nav.goto", &serde_json::Value::Null);
    }
}

/// The path editor with its suggestion popup. Call only while [`is_editing`].
pub fn show_editor(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    s.refresh_suggestions();
    let id = Id::NULL.with(("fm-locationbar", pane));
    let ctx = ui.ctx().clone();

    // Keys that would otherwise move the caret or the widget focus are taken first.
    let (esc, enter, tab, up, down) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Escape),
            i.consume_key(Modifiers::NONE, Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Tab),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
        )
    });
    if esc {
        s.dispatch_or_notice("nav.goto.cancel", &serde_json::Value::Null);
        return;
    }
    if let Some(edit) = s.path_edit.as_mut() {
        if down {
            edit.step(1);
        }
        if up {
            edit.step(-1);
        }
        if tab {
            complete_with_tab(edit);
        }
    }
    let Some(edit) = s.path_edit.as_mut() else {
        return;
    };

    let mut text = edit.text.clone();
    let focus_pending = edit.focus_pending;
    let caret_to_end = edit.caret_to_end;
    let frame = egui::Frame::NONE
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(
            tokens.border_width.max(1.5),
            tokens.accent,
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin::symmetric(tokens.padding as i8, 2));
    let mut edit_rect = ui.available_rect_before_wrap();
    let output = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .font(tokens.font())
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY),
        );
        if focus_pending {
            response.request_focus();
        }
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        vertical_arrows: true,
                        escape: true,
                        ..Default::default()
                    },
                );
            });
        }
        if caret_to_end {
            let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let end = egui::text::CCursor::new(text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), id);
        }
        response
    });
    let response = output.inner;
    edit_rect = edit_rect.intersect(output.response.rect);

    if let Some(edit) = s.path_edit.as_mut() {
        if edit.text != text {
            edit.text = text.clone();
            edit.selected = None;
        }
        edit.focus_pending = false;
        edit.caret_to_end = false;
    }

    if enter {
        let chosen = s
            .path_edit
            .as_ref()
            .and_then(|e| {
                e.selected
                    .and_then(|i| e.suggestions.get(i))
                    .map(|x| x.text.clone())
            })
            .unwrap_or(text);
        s.dispatch_or_notice("nav.goto.commit", &serde_json::json!({ "path": chosen }));
        return;
    }

    let clicked_row = show_popup(&ctx, s, pane, tokens, edit_rect);

    let over_popup = ctx
        .data(|d| d.get_temp::<egui::Rect>(popup_rect_id(pane)))
        .is_some_and(|r| ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| r.contains(p))));
    if !focus_pending && !response.has_focus() && !over_popup && clicked_row.is_none() {
        s.dispatch_or_notice("nav.goto.cancel", &serde_json::Value::Null);
        return;
    }

    match clicked_row {
        Some((chosen, SuggestionKind::History)) => {
            s.dispatch_or_notice("nav.goto.commit", &serde_json::json!({ "path": chosen }));
        }
        Some((chosen, SuggestionKind::Dir)) => {
            if let Some(edit) = s.path_edit.as_mut() {
                edit.set_text(chosen);
                edit.focus_pending = true;
            }
        }
        None => {}
    }
}

/// Tab: take the highlighted suggestion, else extend the text to the common prefix of the
/// directory suggestions (like a shell), else start highlighting.
fn complete_with_tab(edit: &mut fm_engine::goto_cmds::PathEdit) {
    if let Some(i) = edit.selected
        && let Some(sug) = edit.suggestions.get(i).cloned()
    {
        edit.set_text(sug.text);
        return;
    }
    let dirs: Vec<&str> = edit
        .suggestions
        .iter()
        .filter(|x| x.kind == SuggestionKind::Dir)
        .map(|x| x.text.as_str())
        .collect();
    let Some(first) = dirs.first() else {
        return;
    };
    let mut common = first.to_string();
    for other in &dirs[1..] {
        let keep = common
            .chars()
            .zip(other.chars())
            .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
            .count();
        common = common.chars().take(keep).collect();
    }
    if common.chars().count() > edit.text.chars().count() {
        edit.set_text(common);
    } else {
        edit.step(1);
    }
}

/// The suggestion list under the editor. Returns the row clicked this frame, if any.
fn show_popup(
    ctx: &egui::Context,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    edit_rect: egui::Rect,
) -> Option<(String, SuggestionKind)> {
    let edit = s.path_edit.as_ref()?;
    if edit.suggestions.is_empty() {
        ctx.data_mut(|d| d.remove::<egui::Rect>(popup_rect_id(pane)));
        return None;
    }
    let mut clicked = None;
    let selected = edit.selected;
    // Keep the highlighted row inside the visible window of rows.
    let first = selected.map_or(0, |i| (i + 1).saturating_sub(MAX_ROWS));
    let area = egui::Area::new(popup_rect_id(pane).with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(edit_rect.left_bottom() + egui::vec2(0.0, 2.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(edit_rect.width().max(160.0));
                ui.set_max_width(edit_rect.width().max(160.0));
                for (i, sug) in edit
                    .suggestions
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(MAX_ROWS)
                {
                    let label = match sug.kind {
                        SuggestionKind::Dir => RichText::new(&sug.text).font(tokens.font()),
                        SuggestionKind::History => RichText::new(&sug.text)
                            .font(tokens.font())
                            .color(tokens.text_muted),
                    };
                    let row = ui.selectable_label(selected == Some(i), label);
                    if selected == Some(i) {
                        row.scroll_to_me(None);
                    }
                    if row.clicked() {
                        clicked = Some((sug.text.clone(), sug.kind));
                    }
                }
            });
        });
    ctx.data_mut(|d| d.insert_temp(popup_rect_id(pane), area.response.rect));
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_engine::Session;

    #[test]
    #[allow(clippy::expect_used)]
    fn editor_renders_headless_in_both_palettes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut s = Session::new().expect("session");
            s.dispatch_or_notice("nav.goto", &serde_json::Value::Null);
            let pane = s.layout.focused();
            assert!(is_editing(&s, pane));
            let ctx = egui::Context::default();
            crate::theme::apply_to(&ctx, &tokens);
            for _ in 0..3 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if is_editing(&s, pane) {
                            show_editor(ui, &mut s, pane, &tokens);
                        }
                    });
                });
                out.textures_delta.clear();
            }
        }
    }
}
