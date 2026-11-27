//! Edit mode of a Preview pane (capability `text-editor`): a virtualized text view over
//! the engine's rope buffer. Only the visible lines are laid out; scrolling is anchored to
//! a line number (never a pixel height), so a 1 GiB file scrolls like a small one. Every
//! change goes through `editor.*` commands; the widget only draws and translates input.

use std::sync::Arc;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CursorIcon, Event, FontId, Galley, Id, Key, Modifiers, Pos2, Rect, Sense,
    Stroke, Ui, Vec2, pos2, vec2,
};
use fm_engine::Session;
use fm_engine::editor_doc::{Conflict, EditorDoc, Phase};
use fm_layout::PaneId;
use serde_json::{Value, json};

use crate::theme::Tokens;

/// Lines longer than this (in characters) are laid out as a window around the view.
const LONG_LINE: usize = 4096;
/// Characters laid out on each side of the visible part of a long line.
const LONG_MARGIN: usize = 256;
/// Bracket search distance for the match highlight.
const BRACKET_LIMIT: usize = 20_000;
const GUTTER_PAD: f32 = 10.0;
const SCROLLBAR: f32 = 10.0;

/// View state the engine does not need: horizontal offset, the sub-line part of the
/// vertical position, text of the inline prompts.
#[derive(Clone, Default)]
struct ViewUi {
    x_off: f32,
    /// Pixels scrolled into the top line (wrap mode, where lines differ in height).
    sub: f32,
    widest: f32,
    goto_text: String,
    save_as: Option<String>,
    find_focus: bool,
    goto_focus: bool,
    drag_v: Option<f32>,
    drag_h: Option<f32>,
}

fn view_id(pane: PaneId) -> Id {
    Id::NULL.with(("fm-editor-view", pane))
}

fn text_id(pane: PaneId) -> Id {
    Id::NULL.with(("fm-editor-text", pane))
}

fn keyboard_flag() -> Id {
    Id::NULL.with("fm-editor-owns-keyboard")
}

/// True when an editor had keyboard focus last frame: the app then treats keys as typing
/// (no Backspace = parent folder, no Tab = swap panes, no Delete = trash).
pub fn owns_keyboard(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(keyboard_flag()))
        .unwrap_or(false)
}

/// Records this frame's focus; several editor panes OR together within one frame.
fn note_keyboard(ctx: &egui::Context, focused: bool) {
    let frame = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let id = Id::NULL.with("fm-editor-owns-keyboard-frame");
        let same = d.get_temp::<u64>(id) == Some(frame);
        let before = same && d.get_temp::<bool>(keyboard_flag()).unwrap_or(false);
        d.insert_temp(id, frame);
        d.insert_temp(keyboard_flag(), before || focused);
    });
}

fn mono(tokens: &Tokens) -> FontId {
    FontId::monospace(tokens.font_size)
}

type Action = (&'static str, Value);

fn run(s: &mut Session, actions: Vec<Action>) {
    for (id, params) in actions {
        s.dispatch_or_notice(id, &params);
    }
}

/// Draws the editor of Preview pane `pane`. Returns false when the pane is not editing.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> bool {
    if s.editor(pane).is_none() {
        return false;
    }
    let pane_rect = ui.max_rect();
    let mut view: ViewUi = ui.data(|d| d.get_temp(view_id(pane))).unwrap_or_default();
    let mut actions: Vec<Action> = Vec::new();
    toolbar(ui, s, pane, tokens, &mut actions);
    if leave_prompt(ui, s, pane, tokens, &mut actions) {
        crate::preferences::open_editor(ui.ctx(), s);
    }
    let blocked = s.editor(pane).and_then(|d| {
        d.blocked
            .as_ref()
            .map(|b| b.message())
            .or_else(|| d.load_error.clone())
    });
    if let Some(message) = blocked {
        blocked_card(ui, pane, tokens, &message, &mut actions);
        run(s, actions);
        ui.data_mut(|d| d.insert_temp(view_id(pane), view));
        return true;
    }
    bars(ui, s, pane, tokens, &mut view, &mut actions);
    find_bar(ui, s, pane, tokens, &mut view, &mut actions);
    goto_bar(ui, s, pane, tokens, &mut view, &mut actions);
    run(s, std::mem::take(&mut actions));
    let status_h = tokens.font_size + 10.0;
    // Never wider than the pane, even if a bar above overflowed it.
    let mut avail = ui.available_rect_before_wrap();
    avail.min.x = avail.min.x.max(pane_rect.min.x);
    avail.max.x = avail.max.x.min(pane_rect.max.x);
    let text_rect = Rect::from_min_max(avail.min, pos2(avail.max.x, avail.max.y - status_h));
    // The pane's zoom (header button, Ctrl+= / Ctrl+-) scales the text, like the rendered
    // preview's content; the bars above keep their size.
    let scale = s.preview_slot(pane).map_or(1.0, |slot| slot.scale);
    let mut text_tokens = *tokens;
    if scale.is_finite() && scale > 0.0 && (scale - 1.0).abs() > f32::EPSILON {
        text_tokens.font_size = (tokens.font_size * scale).max(4.0);
    }
    text_area(ui, s, pane, &text_tokens, text_rect, &mut view);
    let status_rect = Rect::from_min_max(pos2(avail.min.x, text_rect.max.y), avail.max);
    status_line(ui, s, pane, tokens, status_rect, &mut actions);
    ui.allocate_rect(avail, Sense::hover());
    run(s, actions);
    ui.data_mut(|d| d.insert_temp(view_id(pane), view));
    true
}

fn toolbar(ui: &mut Ui, s: &Session, pane: PaneId, tokens: &Tokens, actions: &mut Vec<Action>) {
    use crate::pane_bar::{self, Common, tool};
    use crate::tool_icons::{self as icons, Icon};
    let Some(doc) = s.editor(pane) else {
        return;
    };
    let p = json!({ "pane": pane });
    let blocked = doc.blocked.is_some();
    // The same bar as the rendered preview: Wrap, Find and the Edit/Preview switch keep
    // their spots at the right; the editor's own tools sit left of them.
    let common = Common {
        editing: true,
        mode_enabled: true,
        mode_tip: "Back to the rendered preview (F4); unsaved edits are saved first",
        find_enabled: !blocked,
        find_active: doc.find.open,
        find_tip: "Find and replace (Ctrl+F)",
        wrap_enabled: !blocked,
        wrap_active: doc.wrap,
        wrap_tip: "Wrap long lines",
    };
    let mut tool_actions: Vec<Action> = Vec::new();
    let mut tools = |ui: &mut Ui, menu: bool| {
        if menu {
            ui.spacing_mut().item_spacing.x = 8.0;
        }
        let mut item = |ui: &mut Ui, icon, name, tip, id: &'static str, on| {
            if tool(ui, tokens, icon, name, tip, true, on, menu).clicked() {
                tool_actions.push((id, p.clone()));
            }
        };
        if !blocked {
            item(
                ui,
                Icon::Live,
                "Live Preview",
                "Live preview: your unsaved text rendered in another Preview pane",
                "editor.open_live_preview",
                false,
            );
            item(
                ui,
                Icon::GoToLine,
                "Go to Line",
                "Go to line (Ctrl+G)",
                "editor.goto_prompt",
                false,
            );
            item(
                ui,
                Icon::LineNumbers,
                "Line Numbers",
                "Show line numbers",
                "editor.toggle_line_numbers",
                doc.line_numbers,
            );
        }
        item(
            ui,
            Icon::External,
            "Open in External Editor",
            "Open in External Editor: the default application for this file",
            "editor.open_external",
            false,
        );
    };
    // Save never folds into the menu.
    let mut save = false;
    let mut pinned = |ui: &mut Ui| {
        if !blocked {
            save = icons::tool_button(
                ui,
                tokens,
                Icon::Save,
                "Save",
                "Save (Ctrl+S)",
                doc.dirty() && doc.savable(),
                false,
            )
            .clicked();
        }
    };
    let mut context = |ui: &mut Ui| {
        let mode = if doc.read_only.is_some() || blocked {
            "Viewing"
        } else {
            "Editing"
        };
        ui.add_space(4.0);
        ui.label(egui::RichText::new(mode).color(tokens.accent_text).strong());
        if doc.dirty() {
            icons::dirty_dot(ui, tokens);
        }
        if doc.phase == Phase::Saving {
            ui.add(crate::spinner::Spinner::new().size(tokens.font_size));
            ui.label(tokens.muted("Saving…"));
        }
    };
    let clicks = pane_bar::show(ui, tokens, &mut context, &common, &mut pinned, &mut tools);
    actions.extend(tool_actions);
    if save {
        actions.push(("editor.save", p.clone()));
    }
    if clicks.mode {
        actions.push(("editor.toggle", p.clone()));
    }
    if clicks.find {
        actions.push(("editor.find", p.clone()));
    }
    if clicks.wrap {
        actions.push(("editor.toggle_wrap", p));
    }
    ui.separator();
}

fn blocked_card(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    message: &str,
    actions: &mut Vec<Action>,
) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(message).color(tokens.text));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Open in External Editor").clicked() {
                actions.push(("editor.open_external", json!({ "pane": pane })));
            }
            if ui.button("Back to Preview").clicked() {
                actions.push(("editor.toggle", json!({ "pane": pane })));
            }
        });
    });
}

fn bar_frame(ui: &mut Ui, tokens: &Tokens, color: Color32, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(tokens.panel)
        .stroke(Stroke::new(tokens.border_width.max(1.0), color))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal_wrapped(add);
        });
}

fn bars(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    view: &mut ViewUi,
    actions: &mut Vec<Action>,
) {
    let Some(doc) = s.editor(pane) else {
        return;
    };
    let resolve = |choice: &str| ("editor.resolve", json!({ "choice": choice, "pane": pane }));
    if doc.phase == Phase::Loading {
        let text = match doc.total {
            Some(total) if total > 0 => format!(
                "Loading… {} of {} MiB; editing starts when the file is in",
                doc.loaded >> 20,
                total >> 20
            ),
            _ => "Loading…".to_string(),
        };
        ui.horizontal(|ui| {
            ui.add(crate::spinner::Spinner::new().size(tokens.font_size));
            ui.label(tokens.muted(text));
        });
    }
    if let Some(conflict) = &doc.conflict {
        bar_frame(ui, tokens, tokens.warning, |ui| {
            ui.label(match conflict {
                Conflict::Changed => {
                    "The file changed on disk after it was opened; nothing was saved."
                }
                Conflict::Deleted => "The file was deleted or moved; nothing was saved.",
            });
            if ui
                .button("Reload")
                .on_hover_text("Discard your edits and read the file again")
                .clicked()
            {
                actions.push(resolve("reload"));
            }
            if ui
                .button("Overwrite")
                .on_hover_text("Save your text over the file")
                .clicked()
            {
                actions.push(resolve("overwrite"));
            }
            if ui.button("Save As…").clicked() {
                view.save_as = Some(copy_name(&doc.name));
            }
        });
    }
    if let Some(name) = view.save_as.as_mut() {
        let mut close = false;
        bar_frame(ui, tokens, tokens.border, |ui| {
            ui.label("Save as");
            let edit = ui.add(egui::TextEdit::singleline(name).desired_width(220.0));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (ui.button("Save").clicked() || enter)
                && let Some(parent) = doc.uri.parent()
                && let Ok(target) = parent.join(name.trim())
            {
                actions.push((
                    "editor.save_as",
                    json!({ "uri": target.to_string(), "pane": pane }),
                ));
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        if close {
            view.save_as = None;
        }
    }
    if doc.changed_on_disk && doc.conflict.is_none() {
        bar_frame(ui, tokens, tokens.warning, |ui| {
            ui.label("The file changed on disk while you were editing it.");
            if ui.button("Reload").clicked() {
                actions.push(resolve("reload"));
            }
            if ui
                .button("Keep Mine")
                .on_hover_text("Keep your text; saving replaces the file on disk")
                .clicked()
            {
                actions.push(resolve("keep"));
            }
        });
    }
    if let Some(err) = &doc.save_error {
        bar_frame(ui, tokens, tokens.error, |ui| {
            ui.label(format!("Not saved: {err}"));
            if ui.button("Try Again").clicked() {
                actions.push(("editor.save", json!({ "pane": pane })));
            }
        });
    }
    if let Some(why) = &doc.read_only {
        let text = match why.as_str() {
            "archive" => "Files inside an archive are read-only; extract the file to edit it.",
            _ => "This file is read-only.",
        };
        ui.label(tokens.muted(text));
    }
    if let Some(why) = &doc.plain_reason {
        ui.label(tokens.muted(why));
    }
    if doc.undo.truncated() && doc.undo.can_undo() && !doc.undo.can_redo() && doc.dirty() {
        // Shown only while it matters: older history is gone.
        ui.label(tokens.muted(format!(
            "Undo keeps the last {} steps; older history was dropped.",
            doc.undo.max_steps()
        )));
    }
}

fn copy_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} copy.{ext}"),
        _ => format!("{name} copy"),
    }
}

/// "Save changes?" when leaving a modified file with autosave off. Returns true when the
/// user follows the hint to Preferences › Editor.
fn leave_prompt(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(doc) = s.editor(pane).filter(|d| d.leave_prompt) else {
        return false;
    };
    let resolve = |choice: &str| ("editor.resolve", json!({ "choice": choice, "pane": pane }));
    let ctx = ui.ctx().clone();
    let id = Id::NULL.with(("fm-editor-leave", pane));
    let before = actions.len();
    let mut open = true;
    let mut to_prefs = false;
    crate::dialog_style::window(&ctx, tokens, "Save changes?", &mut open)
        .id(id)
        .order(egui::Order::Foreground)
        .show(&ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.set_max_width(440.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new(&doc.name).strong().color(tokens.text));
                ui.label(" has changes that are not saved.");
            });
            ui.label("If you don't save, they are lost.");
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let save = ui.add(
                    egui::Button::new(egui::RichText::new("Save").color(tokens.bg).strong())
                        .fill(tokens.accent),
                );
                // Save has the focus when the dialog opens: Enter saves.
                let focus_id = id.with("focused");
                if !ui.data(|d| d.get_temp::<bool>(focus_id).unwrap_or(false)) {
                    save.request_focus();
                    ui.data_mut(|d| d.insert_temp(focus_id, true));
                }
                if save.clicked() {
                    actions.push(resolve("save"));
                }
                if ui.button("Don't save").clicked() {
                    actions.push(resolve("discard"));
                }
                if ui.button("Cancel").clicked() {
                    actions.push(resolve("cancel"));
                }
            });
            ui.add_space(2.0);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                crate::dialog_style::hint(ui, tokens, "Autosave is off.");
                let link = ui.add(
                    egui::Button::new(
                        egui::RichText::new("Turn it on in Preferences")
                            .size(tokens.font_size - 1.0)
                            .color(tokens.accent_text)
                            .underline(),
                    )
                    .frame(false),
                );
                if link
                    .on_hover_text("Preferences › Editor › Saving: save automatically when you leave the editor")
                    .clicked()
                {
                    to_prefs = true;
                }
            });
        });
    if (!open || to_prefs || ctx.input(|i| i.key_pressed(Key::Escape))) && actions.len() == before {
        actions.push(resolve("cancel"));
    }
    if actions.len() > before {
        ctx.data_mut(|d| d.remove_temp::<bool>(id.with("focused")));
    }
    to_prefs
}

fn find_bar(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    view: &mut ViewUi,
    actions: &mut Vec<Action>,
) {
    let Some(doc) = s.editor(pane).filter(|d| d.find.open) else {
        view.find_focus = false;
        return;
    };
    let find = doc.find.clone();
    let editable = doc.ensure_editable().is_ok();
    let mut query = find.query.clone();
    let mut replacement = find.replacement.clone();
    let mut set: Option<Value> = None;
    let p = json!({ "pane": pane });
    use crate::tool_icons::{self as icons, Icon};
    let field_w = (ui.available_width() * 0.35).clamp(140.0, 280.0);
    egui::Frame::new()
        .fill(tokens.panel)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 5.0);
            ui.horizontal(|ui| {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .hint_text("Find")
                        .desired_width(field_w)
                        .id(Id::NULL.with(("fm-editor-find", pane))),
                );
                if !view.find_focus {
                    edit.request_focus();
                    view.find_focus = true;
                }
                if edit.changed() {
                    set = Some(json!({ "query": query, "pane": pane }));
                }
                if edit.has_focus() || edit.lost_focus() {
                    let (enter, shift, esc) = ui.input(|i| {
                        (
                            i.key_pressed(Key::Enter),
                            i.modifiers.shift,
                            i.key_pressed(Key::Escape),
                        )
                    });
                    if enter {
                        actions.push((
                            if shift {
                                "editor.find_previous"
                            } else {
                                "editor.find_next"
                            },
                            p.clone(),
                        ));
                        edit.request_focus();
                    }
                    if esc {
                        actions.push(("editor.find_close", p.clone()));
                    }
                }
                for (label, hint, on, key) in [
                    ("Aa", "Match case", find.options.case_sensitive, "case"),
                    ("W", "Whole words", find.options.whole_word, "word"),
                    (".*", "Regular expression", find.options.regex, "regex"),
                ] {
                    let chip =
                        egui::Button::selectable(on, egui::RichText::new(label).font(mono(tokens)))
                            .min_size(icons::BUTTON * tokens.zoom);
                    if ui.add(chip).on_hover_text(hint).clicked() {
                        set = Some(json!({ key: !on, "pane": pane }));
                    }
                }
                ui.add_space(6.0);
                let count = if let Some(err) = &find.error {
                    egui::RichText::new(err).color(tokens.error)
                } else if find.busy {
                    tokens.muted("Counting…")
                } else if find.query.is_empty() {
                    tokens.muted("")
                } else if find.matches.is_empty() {
                    egui::RichText::new("No matches").color(tokens.warning)
                } else {
                    let more = if find.truncated { "+" } else { "" };
                    let n = find.matches.len();
                    tokens.muted(format!(
                        "{n}{more} {}",
                        if n == 1 { "match" } else { "matches" }
                    ))
                };
                ui.label(count);
                let any = !find.matches.is_empty();
                if icons::button(
                    ui,
                    tokens,
                    Icon::Up,
                    "Previous match (Shift+F3)",
                    any,
                    false,
                )
                .clicked()
                {
                    actions.push(("editor.find_previous", p.clone()));
                }
                if icons::button(ui, tokens, Icon::Down, "Next match (F3)", any, false).clicked() {
                    actions.push(("editor.find_next", p.clone()));
                }
                if ui
                    .add(
                        egui::Button::selectable(find.replace, "Replace")
                            .min_size(vec2(0.0, icons::BUTTON.y * tokens.zoom)),
                    )
                    .on_hover_text("Show the replace field")
                    .clicked()
                {
                    set = Some(json!({ "replace": !find.replace, "pane": pane }));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, tokens, Icon::Close, "Close (Esc)", true, false).clicked()
                    {
                        actions.push(("editor.find_close", p.clone()));
                    }
                });
            });
            if find.replace {
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut replacement)
                            .hint_text("Replace with")
                            .desired_width(field_w),
                    );
                    if edit.changed() {
                        set = Some(json!({ "replacement": replacement, "pane": pane }));
                    }
                    let enabled = editable && !find.matches.is_empty();
                    if ui
                        .add_enabled(enabled, egui::Button::new("Replace"))
                        .on_hover_text("Replace this match and go to the next")
                        .clicked()
                    {
                        actions.push(("editor.replace", p.clone()));
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Replace All"))
                        .on_hover_text("Replace every match; one undo step")
                        .clicked()
                    {
                        actions.push(("editor.replace", json!({ "all": true, "pane": pane })));
                    }
                });
            }
        });
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(tokens.border_width.max(1.0), tokens.border),
    );
    if let Some(params) = set {
        s.dispatch_or_notice("editor.find", &params);
    }
}

fn goto_bar(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    view: &mut ViewUi,
    actions: &mut Vec<Action>,
) {
    let Some(doc) = s.editor(pane).filter(|d| d.goto_open) else {
        view.goto_focus = false;
        return;
    };
    let lines = doc.buffer.len_lines();
    bar_frame(ui, tokens, tokens.border, |ui| {
        ui.label("Go to line");
        let edit = ui.add(
            egui::TextEdit::singleline(&mut view.goto_text)
                .hint_text(format!("1–{lines}, or line:column"))
                .desired_width(160.0),
        );
        if !view.goto_focus {
            edit.request_focus();
            view.goto_focus = true;
        }
        let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if (edit.lost_focus() && enter) || ui.button("Go").clicked() {
            let text = view.goto_text.trim().to_string();
            if !text.is_empty() {
                actions.push(("editor.goto_line", json!({ "text": text, "pane": pane })));
            }
            view.goto_text.clear();
        }
        let close = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                crate::tool_icons::button(
                    ui,
                    tokens,
                    crate::tool_icons::Icon::Close,
                    "Close (Esc)",
                    true,
                    false,
                )
                .clicked()
            })
            .inner;
        if esc || close {
            view.goto_text.clear();
            actions.push(("editor.goto_prompt", json!({ "pane": pane, "close": true })));
        }
    });
}

/// One laid-out visible line.
struct Laid {
    line: usize,
    /// Char index (in the buffer) of the first char of the galley.
    start: usize,
    /// Chars of the line before the galley's first char (long-line window).
    skip: usize,
    galley: Arc<Galley>,
    /// Top of the line on screen.
    top: f32,
    x0: f32,
}

#[allow(clippy::too_many_lines)]
fn text_area(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    rect: Rect,
    view: &mut ViewUi,
) {
    let id = text_id(pane);
    let font = mono(tokens);
    let (font_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
    let font_h = font_h.max(1.0);
    // Roomier lines than the font's own height (Preferences > Editor > Line spacing).
    let row_h = (font_h * s.prefs.editor.line_spacing()).round().max(font_h);
    let char_w = char_w.max(1.0);
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let focused_pane = s.layout.focused() == pane;
    // Focus: a click, or the engine asking (just opened, prompt closed).
    if response.clicked() || response.drag_started() {
        response.request_focus();
        if !focused_pane {
            let _ = s.layout.focus_pane(pane);
        }
    } else if focused_pane
        && s.editor(pane).is_some_and(|d| d.want_focus)
        && !ui.ctx().egui_wants_keyboard_input()
    {
        response.request_focus();
        if let Some(doc) = s.editor_mut(pane) {
            doc.want_focus = false;
        }
    }
    if s.layout.focused() != pane && response.has_focus() {
        response.surrender_focus();
    }
    let has_focus = response.has_focus();
    note_keyboard(ui.ctx(), has_focus);
    if has_focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }
    let Some(doc) = s.editor(pane) else {
        return;
    };
    let total = doc.buffer.len_lines().max(1);
    let fold_w = char_w * 1.8;
    let gutter_w = if doc.line_numbers {
        digits(total) as f32 * char_w + GUTTER_PAD * 2.0
    } else {
        GUTTER_PAD
    } + fold_w;
    let text_left = rect.left() + gutter_w;
    let text_w = (rect.width() - gutter_w - SCROLLBAR).max(char_w * 4.0);
    let rows_fit = ((rect.height() - SCROLLBAR) / row_h).floor().max(1.0) as usize;
    // Keyboard first, so this frame shows its result.
    if has_focus {
        keyboard(ui, s, pane, rows_fit);
    }
    // Wheel.
    if response.hovered() {
        // Same distance per wheel notch as the rendered preview (and its speed setting).
        let boost = crate::theme::WHEEL_BOOST * s.prefs.scroll_speed();
        let delta = ui.input(|i| i.smooth_scroll_delta) * boost;
        if delta != Vec2::ZERO {
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            scroll_by(s, pane, view, -delta.y, row_h, total);
            view.x_off = (view.x_off - delta.x).max(0.0);
        }
    }
    let Some(doc) = s.editor(pane) else {
        return;
    };
    let wrap = doc.wrap;
    if wrap {
        view.x_off = 0.0;
    }
    let caret_pos = doc.buffer.char_to_pos(doc.caret).unwrap_or_default();
    let folds = &doc.folds;
    let mut top = folds.visible_at(doc.scroll_line.min(total - 1));
    if doc.reveal {
        // Keep the caret line on screen (with wrap, lines may be taller: aim higher).
        let margin = if wrap {
            rows_fit / 2
        } else {
            rows_fit.saturating_sub(1)
        };
        if caret_pos.line < top {
            top = caret_pos.line;
            view.sub = 0.0;
        } else if visible_between(folds, top, caret_pos.line, margin.max(1)) >= margin.max(1) {
            // Back up from the caret by the visible lines that fit.
            let mut t = caret_pos.line;
            for _ in 1..margin.max(1) {
                t = folds.prev_visible(t);
            }
            top = t;
            view.sub = 0.0;
        }
        if !wrap {
            let caret_x = caret_pos.col as f32 * char_w;
            if caret_x < view.x_off {
                view.x_off = (caret_x - char_w * 4.0).max(0.0);
            } else if caret_x > view.x_off + text_w - char_w * 2.0 {
                view.x_off = caret_x - text_w + char_w * 8.0;
            }
        }
    }
    let code = &tokens.code;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, code.bg);
    // The visible lines (folded ones skipped), then colours per run of adjacent lines.
    let mut visible = Vec::with_capacity(rows_fit + 2);
    let mut at = top;
    while at < total && visible.len() < rows_fit + 2 {
        visible.push(at);
        at = folds.next_visible(at);
    }
    let rainbow_on = s.prefs.editor.rainbow;
    let mut spans: std::collections::HashMap<usize, Vec<Colour>> = std::collections::HashMap::new();
    if let Some(syn) = doc.syntax.as_ref() {
        for run in runs(&visible) {
            let colours = syn.highlight(&doc.buffer, run.clone());
            let brackets = if rainbow_on {
                syn.rainbow(&doc.buffer, run.clone())
            } else {
                Vec::new()
            };
            for (i, line) in run.enumerate() {
                let merged = merge_colours(
                    colours.get(i).map(Vec::as_slice).unwrap_or(&[]),
                    brackets.get(i).map(Vec::as_slice).unwrap_or(&[]),
                    tokens,
                );
                spans.insert(line, merged);
            }
        }
    }
    let mut laid: Vec<Laid> = Vec::new();
    let mut y = rect.top() - view.sub;
    let first_col = (view.x_off / char_w).floor() as usize;
    let cols = (text_w / char_w).ceil() as usize + 2;
    let mut widest = 0.0f32;
    let mut line = top;
    while line < total && y < rect.bottom() {
        let raw = doc.buffer.line_text(line).unwrap_or_default();
        let text = raw.trim_end_matches(['\n', '\r']);
        let len = text.chars().count();
        let start = doc.buffer.line_start(line).unwrap_or(0);
        let (shown, skip) = if !wrap && len > LONG_LINE {
            let from = first_col.saturating_sub(LONG_MARGIN);
            let to = (first_col + cols + LONG_MARGIN).min(len);
            let piece: String = text
                .chars()
                .skip(from)
                .take(to.saturating_sub(from))
                .collect();
            (piece, from)
        } else {
            (text.to_string(), 0)
        };
        let line_spans = if skip == 0 {
            spans.get(&line).map(Vec::as_slice).unwrap_or(&[])
        } else {
            &[]
        };
        let job = line_job(
            &shown,
            line_spans,
            &font,
            tokens,
            if wrap { text_w } else { f32::INFINITY },
            row_h,
        );
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let h = galley.size().y.max(row_h);
        widest = widest.max(galley.size().x + skip as f32 * char_w);
        let x0 = text_left - view.x_off + skip as f32 * char_w;
        laid.push(Laid {
            line,
            start: start + skip,
            skip,
            galley,
            top: y,
            x0,
        });
        y += h;
        line = folds.next_visible(line);
    }
    view.widest = view.widest.max(widest);
    let max_x = (view.widest - text_w + char_w * 4.0).max(0.0);
    view.x_off = view.x_off.min(max_x);
    let (sel_a, sel_b) = doc.selection();
    let clip = Rect::from_min_max(
        pos2(text_left - 2.0, rect.top()),
        pos2(rect.right() - SCROLLBAR, rect.bottom()),
    );
    let text_painter = ui.painter_at(clip);
    // Current line, selection, find matches, bracket pair: behind the text.
    let find_matches: &[fm_editor::Match] =
        if doc.find.open && doc.find.version == doc.buffer.version() {
            &doc.find.matches
        } else {
            &[]
        };
    let brackets = if sel_a == sel_b {
        fm_editor::match_bracket(&doc.buffer, doc.caret, BRACKET_LIMIT)
    } else {
        None
    };
    let match_bg = tokens.warning.gamma_multiply(0.35);
    let bracket_bg = tokens.accent.gamma_multiply(0.35);
    for l in &laid {
        let line_len = l.galley.job.text.chars().count();
        let end = l.start + line_len;
        let origin = pos2(l.x0, l.top);
        if sel_a == sel_b && l.line == caret_pos.line && has_focus {
            let band = Rect::from_min_size(
                pos2(clip.left(), l.top),
                vec2(clip.width(), l.galley.size().y.max(row_h)),
            );
            text_painter.rect_filled(band, 0.0, code.line_bg);
        }
        let first = find_matches.partition_point(|m| m.end <= l.start);
        for m in find_matches
            .iter()
            .skip(first)
            .take_while(|m| m.start < end + 1)
        {
            paint_range(
                &text_painter,
                &l.galley,
                origin,
                m.start,
                m.end,
                l.start,
                line_len,
                char_w,
                match_bg,
                false,
            );
        }
        if sel_a < sel_b && sel_a <= end + 1 && sel_b > l.start {
            let through = sel_b > end;
            paint_range(
                &text_painter,
                &l.galley,
                origin,
                sel_a,
                sel_b,
                l.start,
                line_len,
                char_w,
                tokens.selection_bg,
                through,
            );
        }
        if let Some((a, b)) = brackets {
            for at in [a, b] {
                paint_range(
                    &text_painter,
                    &l.galley,
                    origin,
                    at,
                    at + 1,
                    l.start,
                    line_len,
                    char_w,
                    bracket_bg,
                    false,
                );
            }
        }
        text_painter.galley(origin, l.galley.clone(), code.text);
    }
    // Fold chevrons and the "… N lines" marker after a folded line.
    let starts: std::collections::HashSet<usize> = match (laid.first(), laid.last()) {
        (Some(a), Some(b)) => doc
            .fold_ranges
            .starts_in(&doc.buffer, a.line..b.line + 1)
            .into_iter()
            .collect(),
        _ => Default::default(),
    };
    let mut fold_clicks = Vec::new();
    for l in &laid {
        let folded = folds.folded_at(l.line);
        if folded.is_none() && !starts.contains(&l.line) {
            continue;
        }
        let centre = pos2(text_left - fold_w * 0.55, l.top + row_h * 0.5);
        let r = char_w * 0.45;
        let points = if folded.is_some() {
            vec![
                centre + vec2(-r * 0.6, -r),
                centre + vec2(r * 0.8, 0.0),
                centre + vec2(-r * 0.6, r),
            ]
        } else {
            vec![
                centre + vec2(-r, -r * 0.6),
                centre + vec2(r, -r * 0.6),
                centre + vec2(0.0, r * 0.8),
            ]
        };
        let hit = Rect::from_center_size(centre, vec2(fold_w, row_h));
        let resp = ui.interact(hit, id.with(("fold", l.line)), Sense::click());
        let colour = if resp.hovered() {
            tokens.text
        } else {
            tokens.text_muted
        };
        painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
        if resp.clicked() {
            fold_clicks.push(l.line);
        }
        if let Some((start, end)) = folded {
            let label = format!(" … {} lines ", end - start);
            let x = l.x0 + l.galley.size().x + char_w;
            let galley = ui.fonts_mut(|f| f.layout_no_wrap(label, font.clone(), code.comment));
            let pill = vec2(galley.size().x, font_h + 2.0);
            let rect = Rect::from_min_size(pos2(x, l.top + (row_h - pill.y) / 2.0), pill);
            text_painter.rect_filled(rect, pill.y / 2.0, code.line_bg);
            text_painter.galley(
                pos2(x, l.top + (row_h - font_h) / 2.0),
                galley,
                code.comment,
            );
            let marker = ui.interact(rect, id.with(("fold-marker", l.line)), Sense::click());
            if marker.clicked() {
                fold_clicks.push(l.line);
            }
        }
    }
    // Gutter numbers.
    if doc.line_numbers {
        for l in &laid {
            let color = if l.line == caret_pos.line {
                code.text
            } else {
                code.gutter
            };
            painter.text(
                pos2(
                    text_left - fold_w - GUTTER_PAD * 0.5,
                    l.top + (row_h - font_h) / 2.0,
                ),
                Align2::RIGHT_TOP,
                (l.line + 1).to_string(),
                font.clone(),
                color,
            );
        }
    }
    // Caret.
    let mut caret_rect = None;
    if let Some(l) = laid.iter().find(|l| l.line == caret_pos.line) {
        let col = caret_pos.col.saturating_sub(l.skip);
        let r = l
            .galley
            .pos_from_cursor(CCursor::new(col))
            .translate(vec2(l.x0, l.top));
        // As tall as the text, not the (roomier) row.
        let h = (font_h + 2.0).min(r.height().max(font_h));
        let top = r.center().y - h / 2.0;
        let r = Rect::from_min_max(pos2(r.min.x, top), pos2(r.min.x + 2.0, top + h));
        caret_rect = Some(r);
        if has_focus && doc.ensure_editable().is_ok() {
            text_painter.rect_filled(r, 1.0, code.text);
        } else if has_focus {
            text_painter.rect_filled(r, 1.0, code.gutter);
        }
    }
    if let (true, Some(r)) = (has_focus, caret_rect) {
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: Default::default(),
                rect,
                cursor_rect: r,
                should_interrupt_composition: false,
            });
        });
    }
    // Scrollbars.
    let v_track = Rect::from_min_max(
        pos2(rect.right() - SCROLLBAR, rect.top()),
        pos2(rect.right(), rect.bottom() - SCROLLBAR),
    );
    let frac = |n: usize| n as f32 / total.max(1) as f32;
    let thumb_h = (v_track.height() * (rows_fit as f32 / total as f32).min(1.0))
        .max(24.0)
        .min(v_track.height());
    let thumb_y = v_track.top()
        + (v_track.height() - thumb_h)
            * frac(top).min(1.0)
            * (total as f32 / (total.saturating_sub(rows_fit).max(1)) as f32).min(1.0);
    let v_thumb = Rect::from_min_size(
        pos2(v_track.left() + 2.0, thumb_y),
        vec2(SCROLLBAR - 4.0, thumb_h),
    );
    let v_resp = ui.interact(v_track, id.with("vbar"), Sense::click_and_drag());
    let mut new_top = None;
    if total > rows_fit {
        scroll_thumb(
            &painter,
            v_thumb,
            true,
            v_resp.hovered(),
            v_resp.dragged(),
            tokens,
        );
        if let Some(p) = v_resp.interact_pointer_pos() {
            if v_resp.drag_started() || v_resp.clicked() {
                view.drag_v = Some(if v_thumb.contains(p) {
                    p.y - v_thumb.top()
                } else {
                    thumb_h / 2.0
                });
            }
            if let Some(grab) = view.drag_v {
                let span = (v_track.height() - thumb_h).max(1.0);
                let t = ((p.y - grab - v_track.top()) / span).clamp(0.0, 1.0);
                new_top = Some((t * total.saturating_sub(rows_fit) as f32).round() as usize);
            }
        }
        if !v_resp.dragged() && !v_resp.clicked() {
            view.drag_v = None;
        }
    }
    if !wrap && view.widest > text_w {
        let h_track = Rect::from_min_max(
            pos2(text_left, rect.bottom() - SCROLLBAR),
            pos2(rect.right() - SCROLLBAR, rect.bottom()),
        );
        let thumb_w = (h_track.width() * text_w / view.widest)
            .max(24.0)
            .min(h_track.width());
        let max_off = (view.widest - text_w).max(1.0);
        let thumb_x =
            h_track.left() + (h_track.width() - thumb_w) * (view.x_off / max_off).min(1.0);
        let h_thumb = Rect::from_min_size(
            pos2(thumb_x, h_track.top() + 2.0),
            vec2(thumb_w, SCROLLBAR - 4.0),
        );
        let h_resp = ui.interact(h_track, id.with("hbar"), Sense::click_and_drag());
        scroll_thumb(
            &painter,
            h_thumb,
            false,
            h_resp.hovered(),
            h_resp.dragged(),
            tokens,
        );
        if let Some(p) = h_resp.interact_pointer_pos() {
            if h_resp.drag_started() || h_resp.clicked() {
                view.drag_h = Some(if h_thumb.contains(p) {
                    p.x - h_thumb.left()
                } else {
                    thumb_w / 2.0
                });
            }
            if let Some(grab) = view.drag_h {
                let span = (h_track.width() - thumb_w).max(1.0);
                view.x_off = ((p.x - grab - h_track.left()) / span).clamp(0.0, 1.0) * max_off;
            }
        }
        if !h_resp.dragged() && !h_resp.clicked() {
            view.drag_h = None;
        }
    }
    // Pointer: place the caret, drag to select, double-click selects a word.
    let mut pointer_action: Option<Value> = None;
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    let text_hit = Rect::from_min_max(
        pos2(rect.left(), rect.top()),
        pos2(rect.right() - SCROLLBAR, rect.bottom()),
    );
    if let Some(p) = response
        .interact_pointer_pos()
        .filter(|p| text_hit.contains(*p) || response.dragged())
    {
        if let Some((line, col)) = hit(&laid, p, row_h) {
            let shift = ui.input(|i| i.modifiers.shift);
            if response.double_clicked() {
                pointer_action =
                    Some(json!({"op": "select_word", "line": line, "col": col, "pane": pane}));
            } else if response.drag_started() || response.clicked() {
                pointer_action = Some(
                    json!({"op": "click", "line": line, "col": col, "extend": shift, "pane": pane}),
                );
            } else if response.dragged() {
                pointer_action = Some(
                    json!({"op": "click", "line": line, "col": col, "extend": true, "pane": pane}),
                );
            }
        }
        if response.dragged() {
            // Past the edges: keep scrolling while selecting.
            if p.y < rect.top() {
                scroll_by(s, pane, view, -row_h, row_h, total);
            } else if p.y > rect.bottom() - SCROLLBAR {
                scroll_by(s, pane, view, row_h, row_h, total);
            }
        }
    }
    response.context_menu(|ui| context_menu(ui, s, pane));
    if let Some(params) = pointer_action {
        s.dispatch_or_notice("editor.input", &params);
    }
    for line in fold_clicks {
        s.dispatch_or_notice(
            "editor.toggle_fold",
            &json!({ "line": line + 1, "pane": pane }),
        );
    }
    if let Some(doc) = s.editor_mut(pane) {
        if let Some(t) = new_top {
            doc.scroll_line = t.min(total.saturating_sub(1));
            view.sub = 0.0;
        } else {
            doc.scroll_line = top;
        }
        doc.reveal = false;
    }
}

/// The line and column under `p`.
fn hit(laid: &[Laid], p: Pos2, row_h: f32) -> Option<(usize, usize)> {
    let l = laid
        .iter()
        .find(|l| p.y < l.top + l.galley.size().y.max(row_h))
        .or_else(|| laid.last())?;
    let c = l.galley.cursor_from_pos(p - pos2(l.x0, l.top));
    Some((l.line, c.index.0 + l.skip))
}

/// Paints the part of char range `a..b` (buffer indices) that falls in one laid-out line.
#[allow(clippy::too_many_arguments)]
fn paint_range(
    painter: &egui::Painter,
    galley: &Galley,
    origin: Pos2,
    a: usize,
    b: usize,
    line_start: usize,
    line_len: usize,
    char_w: f32,
    color: Color32,
    through_newline: bool,
) {
    let from = a.saturating_sub(line_start).min(line_len);
    let to = b.saturating_sub(line_start).min(line_len);
    if from >= to && !through_newline {
        return;
    }
    let ra = galley.pos_from_cursor(CCursor::new(from));
    let rb = galley.pos_from_cursor(CCursor::new(to));
    if (ra.top() - rb.top()).abs() < 0.5 {
        let mut right = rb.left();
        if through_newline {
            right += char_w * 0.6;
        }
        let r = Rect::from_min_max(pos2(ra.left(), ra.top()), pos2(right, ra.bottom()));
        painter.rect_filled(r.translate(origin.to_vec2()), 0.0, color);
        return;
    }
    // Wrapped: first row to its end, full rows between, last row from its start.
    for row in &galley.rows {
        let top = row.min_y();
        if top + 0.5 < ra.top() || top > rb.top() + 0.5 {
            continue;
        }
        let left = if (top - ra.top()).abs() < 0.5 {
            ra.left()
        } else {
            row.rect().left()
        };
        let right = if (top - rb.top()).abs() < 0.5 {
            rb.left() + if through_newline { char_w * 0.6 } else { 0.0 }
        } else {
            row.rect().right() + char_w * 0.3
        };
        let r = Rect::from_min_max(pos2(left, top), pos2(right, row.max_y()));
        painter.rect_filled(r.translate(origin.to_vec2()), 0.0, color);
    }
}

fn line_job(
    text: &str,
    spans: &[Colour],
    font: &FontId,
    tokens: &Tokens,
    wrap: f32,
    row_h: f32,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    job.break_on_newline = false;
    let styled = |color: Color32, italics: bool| TextFormat {
        font_id: font.clone(),
        color,
        italics,
        line_height: Some(row_h),
        valign: egui::Align::Center,
        ..Default::default()
    };
    let fmt = |color: Color32| styled(color, false);
    let mut at = 0usize;
    for &(span_start, span_end, colour, italics) in spans {
        let (start, end) = (span_start.min(text.len()), span_end.min(text.len()));
        if start < at
            || start >= end
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        if let Some(gap) = text.get(at..start).filter(|g| !g.is_empty()) {
            job.append(gap, 0.0, fmt(tokens.code.text));
        }
        if let Some(piece) = text.get(start..end) {
            job.append(piece, 0.0, styled(colour, italics));
        }
        at = end;
    }
    if let Some(rest) = text.get(at..).filter(|r| !r.is_empty()) {
        job.append(rest, 0.0, fmt(tokens.code.text));
    }
    if job.text.is_empty() {
        job.append("", 0.0, fmt(tokens.code.text));
    }
    job
}

/// A scroll handle in the preview's style (`theme::scroll_style`): slim and faint at rest,
/// wide and stronger under the pointer or while dragged; `full` is its widest rectangle.
fn scroll_thumb(
    painter: &egui::Painter,
    full: Rect,
    vertical: bool,
    hovered: bool,
    dragged: bool,
    tokens: &Tokens,
) {
    use crate::theme::{SCROLL_BAR_SLIM, SCROLL_BAR_WIDE, SCROLL_HANDLE_REST};
    let hot = hovered || dragged;
    let width = if hot {
        SCROLL_BAR_WIDE - 4.0
    } else {
        SCROLL_BAR_SLIM
    };
    let opacity = if dragged {
        0.85
    } else if hovered {
        0.6
    } else {
        SCROLL_HANDLE_REST
    };
    // Slim handles hug the outer edge, as the preview's floating bars do.
    let rect = if vertical {
        Rect::from_min_max(pos2(full.right() - width, full.top()), full.max)
    } else {
        Rect::from_min_max(pos2(full.left(), full.bottom() - width), full.max)
    };
    painter.rect_filled(rect, width / 2.0, tokens.code.text.gamma_multiply(opacity));
}

/// Visible lines from `from` up to `to`, counting at most `limit`.
fn visible_between(folds: &fm_editor::FoldIndex, from: usize, to: usize, limit: usize) -> usize {
    let mut n = 0;
    let mut at = from;
    while at < to && n < limit {
        at = folds.next_visible(at);
        n += 1;
    }
    n
}

/// Runs of adjacent line numbers in an ascending list.
fn runs(lines: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for &l in lines {
        match out.last_mut() {
            Some(r) if r.end == l => r.end = l + 1,
            _ => out.push(l..l + 1),
        }
    }
    out
}

/// A coloured byte range of one line: start, end, colour, italics (comments).
type Colour = (usize, usize, Color32, bool);

/// Syntax colours with rainbow brackets cut in: sorted, non-overlapping byte ranges.
fn merge_colours(
    syntax: &[fm_editor::TokenSpan],
    brackets: &[fm_editor::BracketSpan],
    tokens: &Tokens,
) -> Vec<Colour> {
    let mut out = Vec::with_capacity(syntax.len() + brackets.len());
    for span in syntax {
        let colour = tokens.editor_syntax(span.kind);
        let italics = span.kind == fm_editor::Kind::Comment;
        let mut from = span.start;
        for b in brackets
            .iter()
            .filter(|b| b.start < span.end && b.end > span.start)
        {
            if b.start > from {
                out.push((from, b.start, colour, italics));
            }
            from = from.max(b.end);
        }
        if from < span.end {
            out.push((from, span.end, colour, italics));
        }
    }
    for b in brackets {
        out.push((b.start, b.end, tokens.bracket(b.depth, b.unmatched), false));
    }
    out.sort_unstable_by_key(|c| c.0);
    out
}

fn digits(n: usize) -> usize {
    n.max(1).ilog10() as usize + 1
}

/// Moves the view by `px` pixels (positive: down), in whole lines without wrap.
fn scroll_by(s: &mut Session, pane: PaneId, view: &mut ViewUi, px: f32, row_h: f32, total: usize) {
    let Some(doc) = s.editor_mut(pane) else {
        return;
    };
    let mut sub = view.sub + px;
    let max_top = total.saturating_sub(1);
    let mut top = doc.folds.visible_at(doc.scroll_line.min(max_top));
    while sub >= row_h {
        sub -= row_h;
        let next = doc.folds.next_visible(top);
        if next > max_top {
            sub = 0.0;
            break;
        }
        top = next;
    }
    while sub < 0.0 {
        if top == 0 {
            sub = 0.0;
            break;
        }
        sub += row_h;
        top = doc.folds.prev_visible(top);
    }
    doc.scroll_line = top;
    // Keep the part of a line scrolled so far: scrolling is smooth, not line by line.
    view.sub = sub;
}

/// Keys and text while the editor has focus, as `editor.*` commands.
fn keyboard(ui: &mut Ui, s: &mut Session, pane: PaneId, rows_fit: usize) {
    let events = ui.input(|i| i.events.clone());
    let p = |extra: Value| {
        let mut v = extra;
        if let Some(m) = v.as_object_mut() {
            m.insert("pane".into(), json!(pane));
        }
        v
    };
    let mut handled = Vec::new();
    for (ix, event) in events.iter().enumerate() {
        let action: Option<(&str, Value)> = match event {
            Event::Text(t) if !t.chars().any(char::is_control) => {
                if let Err(e) = s.editor_input_text(pane, t) {
                    s.notice = Some(e.to_string());
                }
                None
            }
            Event::Ime(egui::ImeEvent::Commit(t)) => {
                if let Err(e) = s.editor_input_text(pane, t) {
                    s.notice = Some(e.to_string());
                }
                None
            }
            Event::Paste(t) => {
                if let Err(e) = s.editor_input_text(pane, t) {
                    s.notice = Some(e.to_string());
                }
                None
            }
            Event::Copy | Event::Cut => {
                if let Some(text) = s.editor(pane).and_then(EditorDoc::selected_text) {
                    ui.ctx().copy_text(text);
                }
                if matches!(event, Event::Cut) {
                    Some(("editor.input", p(json!({"op": "cut"}))))
                } else {
                    None
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => key_action(*key, *modifiers, rows_fit).map(|(id, v)| (id, p(v))),
            _ => continue,
        };
        handled.push(ix);
        if let Some((id, params)) = action {
            s.dispatch_or_notice(id, &params);
        }
    }
    if !handled.is_empty() {
        ui.input_mut(|i| {
            let mut ix = 0usize;
            i.events.retain(|e| {
                let keep = !handled.contains(&ix) || !matches!(e, Event::Key { .. });
                ix += 1;
                keep
            });
        });
    }
}

fn key_action(key: Key, m: Modifiers, rows_fit: usize) -> Option<(&'static str, Value)> {
    let input = |v: Value| Some(("editor.input", v));
    let mv = |to: &str| {
        input(
            json!({"op": "move", "to": to, "extend": m.shift, "page": rows_fit.saturating_sub(1).max(1)}),
        )
    };
    let word = m.command || m.alt && cfg!(target_os = "macos");
    match key {
        Key::ArrowLeft if word => mv("word_left"),
        Key::ArrowRight if word => mv("word_right"),
        Key::ArrowLeft if m.command && cfg!(target_os = "macos") => mv("line_start"),
        Key::ArrowRight if m.command && cfg!(target_os = "macos") => mv("line_end"),
        Key::ArrowLeft => mv("left"),
        Key::ArrowRight => mv("right"),
        Key::ArrowUp if m.alt && m.shift => input(json!({"op": "duplicate_lines"})),
        Key::ArrowDown if m.alt && m.shift => input(json!({"op": "duplicate_lines"})),
        Key::ArrowUp if m.alt => input(json!({"op": "move_lines", "down": false})),
        Key::ArrowDown if m.alt => input(json!({"op": "move_lines", "down": true})),
        Key::ArrowUp if m.command => mv("doc_start"),
        Key::ArrowDown if m.command => mv("doc_end"),
        Key::ArrowUp => mv("up"),
        Key::ArrowDown => mv("down"),
        Key::Home if m.command => mv("doc_start"),
        Key::End if m.command => mv("doc_end"),
        Key::Home => mv("line_start"),
        Key::End => mv("line_end"),
        Key::PageUp => mv("page_up"),
        Key::PageDown => mv("page_down"),
        Key::Backspace => input(json!({"op": "backspace", "word": word})),
        Key::Delete => input(json!({"op": "delete", "word": word})),
        Key::Enter if !m.command && !m.alt => input(json!({"op": "newline"})),
        Key::Tab if m.shift => input(json!({"op": "outdent"})),
        Key::Tab if !m.command && !m.alt => input(json!({"op": "indent"})),
        Key::A if m.command && !m.shift => input(json!({"op": "select_all"})),
        Key::Z if m.command && m.shift => Some(("editor.redo", json!({}))),
        Key::Z if m.command => Some(("editor.undo", json!({}))),
        Key::Y if m.command => Some(("editor.redo", json!({}))),
        Key::K if m.command && m.shift => input(json!({"op": "delete_lines"})),
        Key::Slash if m.command => input(json!({"op": "toggle_comment"})),
        Key::F if m.command && !m.shift => Some(("editor.find", json!({}))),
        Key::F3 if m.shift => Some(("editor.find_previous", json!({}))),
        Key::F3 => Some(("editor.find_next", json!({}))),
        Key::F4 if m.is_none() => Some(("editor.toggle", json!({}))),
        // The pane's zoom, as in the rendered preview (Ctrl+Shift stays the app's zoom).
        Key::Plus | Key::Equals if m.command && !m.shift && !m.alt => {
            Some(("preview.scale.in", json!({})))
        }
        Key::Minus if m.command && !m.shift && !m.alt => Some(("preview.scale.out", json!({}))),
        Key::Num0 if m.command && !m.shift && !m.alt => Some(("preview.scale.reset", json!({}))),
        Key::OpenBracket | Key::OpenCurlyBracket if m.command && m.shift => {
            Some(("editor.fold", json!({})))
        }
        Key::CloseBracket | Key::CloseCurlyBracket if m.command && m.shift => {
            Some(("editor.unfold", json!({})))
        }
        Key::Escape => Some(("editor.find_close", json!({}))),
        _ => None,
    }
}

fn context_menu(ui: &mut Ui, s: &mut Session, pane: PaneId) {
    let Some(doc) = s.editor(pane) else {
        return;
    };
    let editable = doc.ensure_editable().is_ok();
    let has_sel = doc.has_selection();
    let comment = doc.lang.and_then(fm_editor::Lang::line_comment).is_some();
    let mut action: Option<(&str, Value)> = None;
    let p = json!({ "pane": pane });
    if ui
        .add_enabled(has_sel && editable, egui::Button::new("Cut"))
        .clicked()
    {
        if let Some(text) = doc.selected_text() {
            ui.ctx().copy_text(text);
        }
        action = Some(("editor.input", json!({"op": "cut", "pane": pane})));
    }
    if ui.add_enabled(has_sel, egui::Button::new("Copy")).clicked()
        && let Some(text) = doc.selected_text()
    {
        ui.ctx().copy_text(text);
    }
    if ui
        .add_enabled(editable, egui::Button::new("Paste"))
        .clicked()
    {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
    if ui.button("Select All").clicked() {
        action = Some(("editor.input", json!({"op": "select_all", "pane": pane})));
    }
    ui.separator();
    if ui
        .add_enabled(
            editable && comment,
            egui::Button::new("Toggle Comment").shortcut_text("Ctrl+/"),
        )
        .clicked()
    {
        action = Some((
            "editor.input",
            json!({"op": "toggle_comment", "pane": pane}),
        ));
    }
    if ui
        .add_enabled(
            editable,
            egui::Button::new("Duplicate Line").shortcut_text("Alt+Shift+Down"),
        )
        .clicked()
    {
        action = Some((
            "editor.input",
            json!({"op": "duplicate_lines", "pane": pane}),
        ));
    }
    if ui
        .add_enabled(
            editable,
            egui::Button::new("Delete Line").shortcut_text("Ctrl+Shift+K"),
        )
        .clicked()
    {
        action = Some(("editor.input", json!({"op": "delete_lines", "pane": pane})));
    }
    ui.separator();
    for (label, id, keys) in [
        ("Fold", "editor.fold", "Ctrl+Shift+["),
        ("Unfold", "editor.unfold", "Ctrl+Shift+]"),
        ("Fold All", "editor.fold_all", ""),
        ("Unfold All", "editor.unfold_all", ""),
    ] {
        if ui
            .add(egui::Button::new(label).shortcut_text(keys))
            .clicked()
        {
            action = Some((id, p.clone()));
        }
    }
    ui.separator();
    if ui
        .add_enabled(
            doc.dirty() && doc.savable(),
            egui::Button::new("Save").shortcut_text("Ctrl+S"),
        )
        .clicked()
    {
        action = Some(("editor.save", p.clone()));
    }
    if ui.button("Open in External Editor").clicked() {
        action = Some(("editor.open_external", p));
    }
    if let Some((id, params)) = action {
        s.dispatch_or_notice(id, &params);
        ui.close();
    }
}

fn status_line(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    let Some(doc) = s.editor(pane) else {
        return;
    };
    let pos = doc.buffer.char_to_pos(doc.caret).unwrap_or_default();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.painter().rect_filled(rect, 0.0, tokens.panel);
    child.add_space(8.0);
    let mut where_ = format!("Ln {}, Col {}", pos.line + 1, pos.col + 1);
    let (a, b) = doc.selection();
    if a != b {
        where_.push_str(&format!(" ({} selected)", b - a));
    }
    child.label(tokens.muted(where_));
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(8.0);
        ui.label(tokens.muted(fm_engine::editor_hub::lang_label(doc.lang)));
        let editable = doc.ensure_editable().is_ok();
        let eol = if doc.mixed_eol {
            format!("{} (mixed)", doc.eol.label())
        } else {
            doc.eol.label().to_string()
        };
        ui.add_enabled_ui(editable, |ui| {
            ui.menu_button(eol, |ui| {
                for (label, key) in [("LF (Unix, macOS)", "lf"), ("CRLF (Windows)", "crlf")] {
                    if ui.button(label).clicked() {
                        actions.push(("editor.set_eol", json!({ "eol": key, "pane": pane })));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Line endings written on save");
            ui.menu_button(doc.encoding.label(), |ui| {
                for (label, key) in [
                    ("UTF-8", "utf-8"),
                    ("UTF-8 with BOM", "utf-8-bom"),
                    ("UTF-16 LE", "utf-16le"),
                    ("UTF-16 BE", "utf-16be"),
                    ("Latin-1", "latin-1"),
                ] {
                    if ui.button(label).clicked() {
                        actions.push((
                            "editor.set_encoding",
                            json!({ "encoding": key, "pane": pane }),
                        ));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Encoding written on save");
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_editor_commands() {
        let none = Modifiers::NONE;
        assert_eq!(
            key_action(Key::Enter, none, 10).map(|a| a.0),
            Some("editor.input")
        );
        assert_eq!(
            key_action(Key::Z, Modifiers::COMMAND, 10).map(|a| a.0),
            Some("editor.undo")
        );
        assert_eq!(
            key_action(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT, 10).map(|a| a.0),
            Some("editor.redo")
        );
        assert_eq!(
            key_action(Key::F4, none, 10).map(|a| a.0),
            Some("editor.toggle")
        );
        assert_eq!(
            key_action(Key::Equals, Modifiers::COMMAND, 10).map(|a| a.0),
            Some("preview.scale.in")
        );
        assert_eq!(
            key_action(Key::Equals, Modifiers::COMMAND | Modifiers::SHIFT, 10).map(|a| a.0),
            None,
            "Ctrl+Shift+= is the app's zoom"
        );
        assert_eq!(
            key_action(Key::Num0, Modifiers::COMMAND, 10).map(|a| a.0),
            Some("preview.scale.reset")
        );
        let tab = key_action(Key::Tab, Modifiers::SHIFT, 10).map(|a| a.1);
        assert_eq!(tab, Some(json!({"op": "outdent"})));
        assert!(
            key_action(Key::Q, none, 10).is_none(),
            "plain letters arrive as text"
        );
    }

    #[test]
    fn line_jobs_skip_bad_spans_and_keep_all_text() {
        let tokens = Tokens::dark();
        let spans = [
            fm_editor::TokenSpan {
                start: 0,
                end: 2,
                kind: fm_editor::Kind::Keyword,
            },
            fm_editor::TokenSpan {
                start: 1,
                end: 3,
                kind: fm_editor::Kind::String,
            },
            fm_editor::TokenSpan {
                start: 5,
                end: 99,
                kind: fm_editor::Kind::Comment,
            },
        ];
        let brackets = [fm_editor::BracketSpan {
            start: 1,
            end: 2,
            depth: 1,
            unmatched: false,
        }];
        let merged = merge_colours(&spans, &brackets, &tokens);
        assert!(
            merged.contains(&(1, 2, tokens.rainbow[1], false)),
            "{merged:?}"
        );
        assert!(merged.contains(&(0, 1, tokens.editor_syntax(fm_editor::Kind::Keyword), false)));
        assert!(
            merged.contains(&(5, 99, tokens.code.comment, true)),
            "comments are italic: {merged:?}"
        );
        let job = line_job(
            "fn é // x",
            &merged,
            &mono(&tokens),
            &tokens,
            f32::INFINITY,
            20.0,
        );
        assert_eq!(job.text, "fn é // x");
        assert_eq!(runs(&[1, 2, 3, 7, 8, 20]), vec![1..4, 7..9, 20..21]);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(copy_name("a.rs"), "a copy.rs");
    }
}
