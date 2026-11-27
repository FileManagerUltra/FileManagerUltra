//! The Preferences dialog. Pure view over `Session::prefs`: every change is a command
//! (`view.theme.*`, `prefs.links.retarget`), so the UI never edits settings itself.

use egui::{Context, Id, Key, RichText};
use serde_json::json;

use fm_engine::{DateFormat, Density, Params, Session, SizeUnits, TerminalShell, ThemePreference};

use crate::dialog_style::{hint, section_heading};
use crate::focus_nav::{self, Face, Lock, Tab};
use crate::theme::Tokens;

#[path = "preferences_shortcuts.rs"]
mod shortcuts;
#[path = "preferences_subtitles.rs"]
mod subtitles;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Topic {
    #[default]
    General,
    Appearance,
    Files,
    FileOperations,
    Preview,
    Editor,
    Media,
    Terminal,
    Connections,
    Workspaces,
    Shortcuts,
    Advanced,
}

impl Topic {
    const ALL: [Self; 12] = [
        Self::General,
        Self::Appearance,
        Self::Files,
        Self::FileOperations,
        Self::Preview,
        Self::Editor,
        Self::Media,
        Self::Terminal,
        Self::Connections,
        Self::Workspaces,
        Self::Shortcuts,
        Self::Advanced,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Files => "Files",
            Self::FileOperations => "Copy and move",
            Self::Preview => "Preview",
            Self::Editor => "Editor",
            Self::Media => "Media",
            Self::Terminal => "Terminal",
            Self::Connections => "Connections",
            Self::Workspaces => "Workspaces",
            Self::Shortcuts => "Keyboard shortcuts",
            Self::Advanced => "Advanced",
        }
    }
}

/// Render the dialog while `Session::preferences_open`; Esc or the button closes it.
/// Open Preferences on the Media page (e.g. from the player's Subtitle appearance item).
pub fn open_media(ctx: &Context, s: &mut Session) {
    if s.dispatch_or_notice("app.preferences", &serde_json::Value::Null) {
        ctx.data_mut(|data| data.insert_temp(Id::NULL.with("fm-preferences-topic"), Topic::Media));
    }
}

/// Open Preferences on the Editor page (e.g. from the editor's "Save changes?" hint).
pub fn open_editor(ctx: &Context, s: &mut Session) {
    if s.dispatch_or_notice("app.preferences", &serde_json::Value::Null) {
        ctx.data_mut(|data| data.insert_temp(Id::NULL.with("fm-preferences-topic"), Topic::Editor));
    }
}

/// Open Preferences on the Files page (e.g. from the "RAR archives need 7-Zip" pane).
pub fn open_files(ctx: &Context, s: &mut Session) {
    if s.dispatch_or_notice("app.preferences", &serde_json::Value::Null) {
        ctx.data_mut(|data| data.insert_temp(Id::NULL.with("fm-preferences-topic"), Topic::Files));
    }
}

fn topic_widget(topic: Topic) -> Id {
    Id::NULL.with(("fm-preferences-topic-row", topic.label()))
}

fn close_widget_key() -> Id {
    Id::NULL.with("fm-preferences-close-id")
}

/// Keyboard model (see `focus_nav`): Tab cycles topic list → the topic's controls → Close →
/// topic list and never leaves the dialog; the topic list is one Tab stop driven by the
/// arrow keys; Ctrl+Tab and Ctrl+PageUp/PageDown switch topics from anywhere.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    if !s.preferences_open {
        return;
    }
    focus_nav::track(ctx);
    // Escape closes the innermost thing first: an open popup, then a text field, then the
    // dialog. egui drops focus and popups on Esc before widgets run, so use the last frame.
    let typing_id = Id::NULL.with("fm-preferences-typing");
    let popup_id = Id::NULL.with("fm-preferences-popup");
    let was_typing = ctx.data(|data| data.get_temp::<bool>(typing_id).unwrap_or(false));
    let had_popup = ctx.data(|data| data.get_temp::<bool>(popup_id).unwrap_or(false));
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape)) && !was_typing && !had_popup;
    let topic_id = Id::NULL.with("fm-preferences-topic");
    let mut topic = ctx.data_mut(|data| data.get_temp::<Topic>(topic_id).unwrap_or_default());
    let step = ctx.input_mut(|i| {
        use egui::Modifiers;
        let shift_ctrl = Modifiers::CTRL | Modifiers::SHIFT;
        if i.consume_key(shift_ctrl, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageUp) {
            Some(Topic::ALL.len() - 1)
        } else if i.consume_key(Modifiers::CTRL, Key::Tab)
            || i.consume_key(Modifiers::CTRL, Key::PageDown)
        {
            Some(1)
        } else {
            None
        }
    });
    if let Some(step) = step {
        let at = Topic::ALL.iter().position(|t| *t == topic).unwrap_or(0);
        topic = Topic::ALL
            .get((at + step) % Topic::ALL.len())
            .copied()
            .unwrap_or_default();
        focus_nav::focus_later(ctx, topic_widget(topic));
    }
    // With nothing focused (just opened, or after Esc left a text field), the keyboard
    // starts on the topic list instead of in the panes behind the dialog.
    if ctx.memory(|m| m.focused()).is_none() {
        ctx.memory_mut(|m| m.request_focus(topic_widget(topic)));
    }
    let close_widget = ctx.data(|data| data.get_temp::<Id>(close_widget_key()));
    let available = ctx.content_rect().size() - egui::vec2(32.0, 32.0);
    let width = available.x.clamp(260.0, 820.0);
    let height = (available.y - 110.0).clamp(120.0, 440.0);
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Preferences", &mut open)
        .id(Id::NULL.with("fm-preferences"))
        .default_width(width)
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.set_width(width);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(160.0);
                    egui::ScrollArea::vertical()
                        .id_salt("preferences-topics")
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            topic = topic_list(ui, tokens, topic, close_widget);
                            focus_nav::scroll_to_focus(ui);
                        });
                });
                let (divider, _) =
                    ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
                ui.painter().line_segment(
                    [divider.center_top(), divider.center_bottom()],
                    egui::Stroke::new(tokens.border_width, tokens.border),
                );
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(topic.label())
                            .color(tokens.text)
                            .strong()
                            .size(tokens.font_size + 4.0),
                    );
                    ui.add_space(14.0);
                    egui::ScrollArea::both()
                        .id_salt(("preferences-settings", topic.label()))
                        .max_height(height - 48.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            settings(ui, s, tokens, topic);
                            focus_nav::scroll_to_focus(ui);
                        });
                });
            });
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(width, 32.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Close is the last stop: Tab goes round to the topic list.
                        if let Some(id) = close_widget {
                            match focus_nav::keys(
                                ui,
                                id,
                                Lock {
                                    tab: true,
                                    arrows: false,
                                },
                            )
                            .tab
                            {
                                Some(Tab::Forward) => {
                                    focus_nav::focus_later(ui.ctx(), topic_widget(topic));
                                }
                                Some(Tab::Back) => focus_nav::pass_tab(ui, Tab::Back),
                                None => {}
                            }
                        }
                        let response = ui.add_sized([88.0, 30.0], egui::Button::new("Close"));
                        focus_nav::claim(ui, &response);
                        ui.data_mut(|data| data.insert_temp(close_widget_key(), response.id));
                        if response.clicked() {
                            close = true;
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            hint(
                                ui,
                                tokens,
                                if topic == Topic::Shortcuts {
                                    "Keyboard shortcuts are read-only."
                                } else {
                                    "Changes apply immediately. Saved on exit."
                                },
                            );
                        });
                    });
                },
            );
            focus_nav::paint_focus_ring(ui, tokens);
        });
    let typing = ctx.text_edit_focused() && !close && open;
    let popup = ctx.any_popup_open() && !close && open;
    focus_nav::end_frame(ctx);
    ctx.data_mut(|data| {
        data.insert_temp(topic_id, topic);
        data.insert_temp(typing_id, typing);
        data.insert_temp(popup_id, popup);
    });
    if close || !open {
        s.preferences_open = false;
        // The panes behind take the keyboard back.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
}

/// The topics, as one Tab stop (the selected topic): Up/Down/Home/End select, Shift+Tab goes
/// round to Close. Returns the selected topic.
fn topic_list(ui: &mut egui::Ui, tokens: &Tokens, mut topic: Topic, close: Option<Id>) -> Topic {
    let count = Topic::ALL.len();
    let at = Topic::ALL.iter().position(|t| *t == topic).unwrap_or(0);
    let mut chosen = None;
    for (index, item) in Topic::ALL.into_iter().enumerate() {
        let id = topic_widget(item);
        let selected = item == topic;
        if selected {
            let keys = focus_nav::keys(
                ui,
                id,
                Lock {
                    tab: true,
                    arrows: true,
                },
            );
            match keys.tab {
                Some(Tab::Back) => {
                    if let Some(close) = close {
                        focus_nav::focus_later(ui.ctx(), close);
                    }
                }
                Some(Tab::Forward) => focus_nav::pass_tab(ui, Tab::Forward),
                None => {}
            }
            let next = match keys.arrow {
                Some(Key::ArrowUp | Key::ArrowLeft) => Some(at.saturating_sub(1)),
                Some(Key::ArrowDown | Key::ArrowRight) => Some((at + 1).min(count - 1)),
                Some(Key::Home) => Some(0),
                Some(Key::End) => Some(count - 1),
                _ => None,
            };
            if let Some(next) = next.filter(|n| *n != at) {
                chosen = Some(next);
            }
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 32.0), egui::Sense::hover());
        let sense = if selected {
            egui::Sense::click()
        } else {
            egui::Sense::CLICK
        };
        let response = ui.interact(rect, id, sense);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, true, selected, item.label())
        });
        focus_nav::claim(ui, &response);
        if response.clicked() && !selected {
            chosen = Some(index);
        }
        if ui.is_rect_visible(rect) {
            let fill = if selected {
                Some(tokens.selection_bg)
            } else if response.hovered() {
                Some(tokens.hover_bg)
            } else {
                None
            };
            if let Some(fill) = fill {
                ui.painter().rect_filled(rect, 2.0, fill);
            }
            ui.painter().text(
                egui::pos2(rect.left() + 6.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                item.label(),
                egui::TextStyle::Button.resolve(ui.style()),
                if selected {
                    tokens.selection_text
                } else {
                    tokens.text
                },
            );
        }
    }
    if let Some(next) = chosen.and_then(|i| Topic::ALL.get(i)) {
        topic = *next;
        focus_nav::focus_later(ui.ctx(), topic_widget(topic));
    }
    topic
}

/// A choice row for preference values: returns the newly chosen value.
fn choose<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    key: &str,
    label: &str,
    options: &[(&str, T)],
    current: T,
) -> Option<T> {
    let faces: Vec<Face<'_>> = options.iter().map(|(text, _)| Face::Text(text)).collect();
    let selected = options.iter().position(|(_, value)| *value == current);
    focus_nav::choice_row(
        ui,
        tokens,
        Id::NULL.with(("fm-preferences-choice", key)),
        Some(label),
        &faces,
        selected,
    )
    .and_then(|index| options.get(index))
    .map(|(_, value)| *value)
}

/// "What opens what": one row per kind of file (its name, with examples in grey below),
/// and the same segmented choice lined up in a column. Returns the row and the new choice.
fn open_table<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    key: &str,
    rows: &[(&str, &str, T)],
    options: &[(&str, T)],
) -> Option<(usize, T)> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = options
        .iter()
        .map(|(text, _)| {
            ui.fonts_mut(|f| f.layout_no_wrap((*text).to_string(), font.clone(), tokens.text))
                .size()
                .x
        })
        .fold(0.0f32, f32::max)
        + 28.0;
    let last = options.len().saturating_sub(1);
    let mut picked = None;
    egui::Grid::new(Id::NULL.with(("fm-preferences-open-table", key)))
        .num_columns(2)
        .spacing(egui::vec2(32.0, 12.0))
        .min_col_width(180.0)
        .show(ui, |ui| {
            for (row, (title, detail, current)) in rows.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(*title);
                    ui.label(
                        RichText::new(*detail)
                            .size((tokens.font_size - 2.0).max(9.0))
                            .color(tokens.text_muted),
                    );
                });
                let names: Vec<String> = options
                    .iter()
                    .map(|(text, _)| format!("{title}: {text}"))
                    .collect();
                let faces: Vec<Face<'_>> = options
                    .iter()
                    .zip(&names)
                    .enumerate()
                    .map(|(i, ((text, _), name))| Face::Segment {
                        text,
                        name,
                        width,
                        first: i == 0,
                        last: i == last,
                    })
                    .collect();
                let selected = options.iter().position(|(_, value)| value == current);
                let choice = focus_nav::choice_row(
                    ui,
                    tokens,
                    Id::NULL.with(("fm-preferences-open", key, row)),
                    None,
                    &faces,
                    selected,
                )
                .and_then(|index| options.get(index));
                if let Some((_, value)) = choice {
                    picked = Some((row, *value));
                }
                ui.end_row();
            }
        });
    picked
}

fn settings(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens, topic: Topic) {
    match topic {
        Topic::Shortcuts => shortcuts::show(ui, s, tokens),
        Topic::General => {
            section_heading(ui, tokens, "Updates");
            let mut check_updates = s.prefs.check_updates;
            if ui
                .checkbox(&mut check_updates, "Check for updates at startup")
                .changed()
            {
                s.dispatch_or_notice("prefs.updates", &json!({"value": check_updates}));
            }
        }
        Topic::Appearance => {
            section_heading(ui, tokens, "Interface");
            if let Some(id) = choose(
                ui,
                tokens,
                "theme",
                "Theme",
                &[
                    ("System", "view.theme.system"),
                    ("Dark", "view.theme.dark"),
                    ("Light", "view.theme.light"),
                ],
                match s.prefs.theme {
                    ThemePreference::System => "view.theme.system",
                    ThemePreference::Dark => "view.theme.dark",
                    ThemePreference::Light => "view.theme.light",
                },
            ) {
                s.dispatch_or_notice(id, &Params::Null);
            }
            ui.add_space(8.0);
            accent_row(ui, s, tokens);
            ui.add_space(8.0);
            if let Some(value) = choose(
                ui,
                tokens,
                "density",
                "Density",
                &[
                    ("Compact", Density::Compact),
                    ("Normal", Density::Normal),
                    ("Comfortable", Density::Comfortable),
                ],
                s.prefs.density,
            ) {
                let value = match value {
                    Density::Compact => "compact",
                    Density::Normal => "normal",
                    Density::Comfortable => "comfortable",
                };
                set(s, "density", json!(value));
            }
            ui.add_space(8.0);
            size_row(ui, s, "Text size", "font_size", s.prefs.ui_font_size());
            hint(
                ui,
                tokens,
                "Text size and row spacing of the whole interface. Use View > Zoom to scale everything.",
            );
        }
        Topic::Files => {
            section(ui, tokens, "Browsing", None, |ui| {
                let mut hidden = s.prefs.show_hidden;
                if ui
                    .checkbox(&mut hidden, "Show hidden files at startup")
                    .changed()
                {
                    set(s, "show_hidden", json!(hidden));
                }
                let mut single = s.prefs.single_click_open;
                if ui
                    .checkbox(&mut single, "Open with a single click")
                    .on_hover_text("Otherwise double-click or Enter opens an item.")
                    .changed()
                {
                    set(s, "single_click_open", json!(single));
                }
            });
            section(ui, tokens, "Archives", None, |ui| {
                let mut browse = s.prefs.archives_browse;
                let response = ui
                    .checkbox(&mut browse, "Enter browses inside archives")
                    .on_hover_text(
                        "Enter or a double click on a zip, tar, 7z or RAR file shows its contents \
                         like a folder. Turn this off to open archives with their \
                         application; the right-click menu always offers both.",
                    );
                if response.changed() {
                    s.dispatch_or_notice("prefs.archives.browse", &json!({"value": browse}));
                }
                let mut in_tree = s.prefs.archives_in_tree;
                let response = ui
                    .add_enabled(
                        s.prefs.archives_browse,
                        egui::Checkbox::new(
                            &mut in_tree,
                            "Show all archives as folders in the tree",
                        ),
                    )
                    .on_hover_text(
                        "Off: the tree shows the archives you opened, until you close them \
                         (right-click > Close Archive). On: every archive file in the tree \
                         can be expanded like a folder.",
                    );
                if response.changed() {
                    s.dispatch_or_notice("prefs.archives.tree", &json!({"value": in_tree}));
                }
                rar_tool_row(ui, s, tokens);
                patterns_row(ui, s, tokens);
                spool_row(ui, s, tokens);
            });
            section(ui, tokens, "Display", None, |ui| {
                let units = [
                    ("KiB (1024)", SizeUnits::Binary),
                    ("kB (1000)", SizeUnits::Decimal),
                    ("Bytes", SizeUnits::Bytes),
                ];
                if let Some(value) =
                    choose(ui, tokens, "sizes", "Sizes", &units, s.prefs.size_units)
                {
                    let value = match value {
                        SizeUnits::Binary => "binary",
                        SizeUnits::Decimal => "decimal",
                        SizeUnits::Bytes => "bytes",
                    };
                    set(s, "size_units", json!(value));
                }
                let formats = [
                    ("2026-10-28 13:40", DateFormat::Iso),
                    ("2026-10-28 13:40:05", DateFormat::IsoSeconds),
                    ("28/10/2026", DateFormat::Eu),
                    ("10/28/2026", DateFormat::Us),
                ];
                if let Some(value) =
                    choose(ui, tokens, "dates", "Dates", &formats, s.prefs.date_format)
                {
                    let value = match value {
                        DateFormat::Iso => "iso",
                        DateFormat::IsoSeconds => "isoseconds",
                        DateFormat::Eu => "eu",
                        DateFormat::Us => "us",
                    };
                    set(s, "date_format", json!(value));
                }
                hint(
                    ui,
                    tokens,
                    "File times use your operating system's timezone, detected at startup, including daylight saving. Log time filters remain UTC.",
                );
            });
            section(
                ui,
                tokens,
                "Confirmations",
                Some("Turn a prompt off only if you are sure; deleted files cannot be restored."),
                |ui| {
                    let mut delete = s.prefs.confirm_delete;
                    if ui
                        .checkbox(&mut delete, "Ask before deleting permanently")
                        .changed()
                    {
                        set(s, "confirm_delete", json!(delete));
                    }
                    let mut run = s.prefs.confirm_run;
                    if ui
                        .checkbox(&mut run, "Ask before running a program")
                        .changed()
                    {
                        set(s, "confirm_run", json!(run));
                    }
                },
            );
            section(ui, tokens, "Git", None, |ui| {
                for (key, label, value) in [
                    ("git_enabled", "Show git status", s.prefs.git_enabled),
                    (
                        "git_gutter",
                        "Status letter beside names",
                        s.prefs.git_gutter,
                    ),
                    ("git_tint", "Tint names by status", s.prefs.git_tint),
                ] {
                    let mut on = value;
                    if ui.checkbox(&mut on, label).changed() {
                        set(s, key, json!(on));
                    }
                }
            });
        }
        Topic::Advanced => {
            section(
                ui,
                tokens,
                "Thumbnails",
                Some("Memory applies at once; the disk cache is trimmed at the next start."),
                |ui| {
                    cache_location(ui, s, tokens);
                    ui.add_space(8.0);
                    let mut memory = s.prefs.thumb_memory_mib;
                    let mut disk = s.prefs.thumb_disk_mib;
                    egui::Grid::new("fm-prefs-thumbs")
                        .num_columns(2)
                        .spacing([ui.spacing().item_spacing.x * 2.0, tokens.padding])
                        .show(ui, |ui| {
                            for (label, key, value, range) in [
                                (
                                    "Memory cache",
                                    "thumb_memory_mib",
                                    &mut memory,
                                    fm_engine::prefs::THUMB_MEMORY_RANGE,
                                ),
                                (
                                    "Disk cache",
                                    "thumb_disk_mib",
                                    &mut disk,
                                    fm_engine::prefs::THUMB_DISK_RANGE,
                                ),
                            ] {
                                let caption = ui.label(label);
                                let response = ui
                                    .add_sized(
                                        [110.0, ui.spacing().interact_size.y],
                                        egui::DragValue::new(value)
                                            .range(range)
                                            .speed(8)
                                            .suffix(" MiB"),
                                    )
                                    .labelled_by(caption.id);
                                if response.changed() {
                                    set(s, key, json!(*value));
                                }
                                ui.end_row();
                            }
                        });
                },
            );
            section(
                ui,
                tokens,
                "Diagrams",
                Some("Drawn Mermaid diagrams are kept in memory while the app runs."),
                |ui| {
                    if ui
                        .button("Clear diagram cache")
                        .on_hover_text(
                            "Forget every drawn Mermaid diagram; open documents draw them again.",
                        )
                        .clicked()
                    {
                        s.dispatch_or_notice("preview.md_diagrams.clear_cache", &Params::Null);
                    }
                },
            );
            section(
                ui,
                tokens,
                "Preview",
                Some("A preview that takes longer is abandoned with an error card."),
                |ui| {
                    let mut secs = s.prefs.preview_time_secs;
                    ui.horizontal(|ui| {
                        let caption = ui.label("Time limit");
                        let response = ui
                            .add_sized(
                                [110.0, ui.spacing().interact_size.y],
                                egui::DragValue::new(&mut secs)
                                    .range(fm_engine::prefs::PREVIEW_TIME_RANGE)
                                    .suffix(" s"),
                            )
                            .labelled_by(caption.id);
                        if response.changed() {
                            set(s, "preview_time_secs", json!(secs));
                        }
                    });
                },
            );
            section(
                ui,
                tokens,
                "Troubleshooting",
                Some(
                    "The log records memory use and what the app does, never file names or passwords. Send it with a bug report.",
                ),
                |ui| {
                    let mut log = s.prefs.diagnostics_log;
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut log, "Write diagnostic log")
                            .on_hover_text(
                                "One sample per second plus events, in the app's data folder. Up to 64 MiB per session, the last 20 sessions are kept",
                            )
                            .changed()
                        {
                            set(s, "diagnostics_log", json!(log));
                        }
                        let folder = s.command_enabled("app.diagnostics.folder");
                        if ui
                            .add_enabled(folder, egui::Button::new("Open log folder"))
                            .on_hover_text("Show the diagnostic logs in a Files pane")
                            .on_disabled_hover_text("No diagnostic log has been written yet")
                            .clicked()
                        {
                            s.dispatch_or_notice("app.diagnostics.folder", &serde_json::Value::Null);
                        }
                    });
                    if cfg!(windows) {
                        renderer_row(ui, s, tokens);
                    }
                },
            );
        }
        Topic::Editor => editor_topic(ui, s, tokens),
        Topic::Media => {
            section(
                ui,
                tokens,
                "Opening files",
                Some(
                    "What Enter or a double-click does with audio and video files on this computer. Open With is always in the right-click menu.",
                ),
                |ui| media_open_choices(ui, s, tokens),
            );
            section(
                ui,
                tokens,
                "Playback",
                Some("Applies to the Media pane and its status bar controls."),
                |ui| {
                    let mut gapless = s.prefs.media_gapless;
                    if ui
                        .checkbox(&mut gapless, "Gapless playback")
                        .on_hover_text("Join consecutive local tracks without a pause")
                        .changed()
                    {
                        set(s, "media_gapless", json!(gapless));
                    }
                    let mut background = s.prefs.media_background_audio;
                    if ui
                        .checkbox(&mut background, "Keep playing in the background")
                        .on_hover_text(
                            "Audio and video continue when the Media pane is closed or you switch workspace",
                        )
                        .changed()
                    {
                        set(s, "media_background_audio", json!(background));
                    }
                    let mut hardware = s.prefs.media_hardware_decoding;
                    if ui
                        .checkbox(&mut hardware, "Hardware video decoding")
                        .on_hover_text(format!(
                            "Decode video on the graphics card ({}). If it cannot, playback continues in software. The player's menu shows which is used",
                            fm_preview::player::HwAccel::for_platform().label()
                        ))
                        .changed()
                    {
                        set(s, "media_hardware_decoding", json!(hardware));
                    }
                    let mut autoplay = s.prefs.media_autoplay_on_add;
                    if ui
                        .checkbox(&mut autoplay, "Start playing when items are added")
                        .on_hover_text(
                            "Adding to the playlist plays the first new item if nothing is playing. Never interrupts playback",
                        )
                        .changed()
                    {
                        set(s, "media_autoplay_on_add", json!(autoplay));
                    }
                },
            );
            section(
                ui,
                tokens,
                "ffmpeg",
                Some(
                    "Needed for video playback, chapters and poster frames. Audio does not need it.",
                ),
                |ui| ffmpeg_section(ui, s, tokens),
            );
            section(
                ui,
                tokens,
                "Audiobooks",
                Some("Skip buttons and remembered positions in the Media pane."),
                |ui| {
                    for (label, key, current) in [
                        (
                            "Skip back",
                            "media_skip_back_secs",
                            s.prefs.media_skip_back_secs,
                        ),
                        (
                            "Skip forward",
                            "media_skip_forward_secs",
                            s.prefs.media_skip_forward_secs,
                        ),
                    ] {
                        let mut secs = current;
                        ui.horizontal(|ui| {
                            let caption = ui.label(label);
                            let response = ui
                                .add_sized(
                                    [110.0, ui.spacing().interact_size.y],
                                    egui::DragValue::new(&mut secs)
                                        .range(fm_engine::prefs::SKIP_RANGE)
                                        .suffix(" s"),
                                )
                                .labelled_by(caption.id);
                            if response.changed() {
                                set(s, key, json!(secs));
                            }
                        });
                    }
                    if ui
                        .button("Forget remembered positions")
                        .on_hover_text(
                            "Clears saved positions, bookmarks and finished marks of audiobooks and videos",
                        )
                        .clicked()
                    {
                        s.dispatch_or_notice("media.resume_clear", &serde_json::Value::Null);
                    }
                },
            );
            subtitles::show(ui, s, tokens);
        }
        Topic::FileOperations => {
            section_heading(ui, tokens, "Symbolic links");
            let mut retarget = s.prefs.retarget_links;
            let response = ui
                .checkbox(&mut retarget, "Re-target links when copying")
                .on_hover_text(
                    "Absolute symbolic links that point inside the copied folder are \
                     re-pointed at the copy. Relative links and links to other places \
                     are copied as they are. Links are never followed.",
                );
            if response.changed() {
                s.dispatch_or_notice("prefs.links.retarget", &json!({"value": retarget}));
            }
            hint(
                ui,
                tokens,
                "Keep links inside a copied folder pointing to the copy. Links are never followed.",
            );
        }
        Topic::Preview => {
            section_heading(ui, tokens, "Scrolling");
            let mut pct = s.prefs.scroll_speed_pct.clamp(
                *fm_engine::prefs::SCROLL_SPEED_RANGE.start(),
                *fm_engine::prefs::SCROLL_SPEED_RANGE.end(),
            );
            ui.horizontal(|ui| {
                let caption = ui.label("Scroll speed");
                let response = ui
                    .add(
                        egui::Slider::new(&mut pct, fm_engine::prefs::SCROLL_SPEED_RANGE)
                            .step_by(25.0)
                            .suffix(" %"),
                    )
                    .labelled_by(caption.id)
                    .on_hover_text(
                        "How far one turn of the mouse wheel scrolls previews and the editor",
                    );
                if response.changed() {
                    set(s, "scroll_speed_pct", json!(pct));
                }
            });
            hint(ui, tokens, "Applies to previews and the editor.");
            ui.add_space(18.0);
            section_heading(ui, tokens, "Playback");
            let mut autoplay = s.prefs.preview_audio_autoplay;
            if ui
                .checkbox(&mut autoplay, "Autoplay audio previews")
                .changed()
            {
                set(s, "preview_audio_autoplay", json!(autoplay));
            }
            hint(
                ui,
                tokens,
                "Off by default: select a music file and press Play to listen.",
            );
            let mut video_autoplay = s.prefs.preview_video_autoplay;
            if ui
                .checkbox(&mut video_autoplay, "Autoplay video previews")
                .changed()
            {
                set(s, "preview_video_autoplay", json!(video_autoplay));
            }
            ui.add_space(18.0);
            section_heading(ui, tokens, "Folders");
            let mut recursive = s.prefs.recursive_preview;
            let response = ui
                .checkbox(&mut recursive, "Recursive folder preview")
                .on_hover_text(
                    "A folder in the preview pane counts, and in thumbnail mode lists, \
                     every file below it. Bounded by entry, depth and time limits; \
                     links are never followed.",
                );
            if response.changed() {
                s.dispatch_or_notice("prefs.preview.recursive", &json!({"value": recursive}));
            }
            ui.add_space(18.0);
            section_heading(ui, tokens, "Markdown");
            let mut remote = s.prefs.preview_remote_images;
            let response = ui
                .checkbox(&mut remote, "Load remote images in Markdown")
                .on_hover_text(
                    "Fetch http(s) images referenced by a Markdown file. The image host \
                     learns your IP address and that you opened the file. Private and \
                     local addresses are never contacted. Relative images load regardless.",
                );
            if response.changed() {
                s.dispatch_or_notice("prefs.preview.remote_images", &json!({"value": remote}));
            }
            let mut mermaid = s.prefs.preview_mermaid;
            let response = ui
                .checkbox(&mut mermaid, "Draw Mermaid diagrams")
                .on_hover_text(
                    "Show ```mermaid code blocks as diagrams in the app's colours. \
                     Drawn offline; a diagram that cannot be drawn stays a code block \
                     with the reason. Turn off to always see the code.",
                );
            if response.changed() {
                s.dispatch_or_notice("prefs.preview.mermaid", &json!({"value": mermaid}));
            }
            ui.add_space(18.0);
            section_heading(ui, tokens, "Log previews");
            let mut poll = s.prefs.log_poll_ms.clamp(
                *fm_engine::prefs::LOG_POLL_RANGE.start(),
                *fm_engine::prefs::LOG_POLL_RANGE.end(),
            );
            let mut tail = s.prefs.log_tail_mib.min(64);
            let mut unlimited = tail == 0;
            let mut changed = false;
            egui::Grid::new("fm-prefs-log")
                .spacing([24.0, 12.0])
                .show(ui, |ui| {
                    let caption = ui.label("Live tail interval (ms)");
                    changed |= ui.add_sized([88.0, 28.0], egui::DragValue::new(&mut poll).range(fm_engine::prefs::LOG_POLL_RANGE).speed(50))
                        .labelled_by(caption.id)
                        .on_hover_text("Background size-check interval for local, SFTP and SMB logs. Unchanged files transfer no content; growth reads only appended bytes.")
                        .changed();
                    ui.end_row();
                    let caption = ui.label("Initial tail size (MiB)");
                    let mut shown_tail = tail.max(1);
                    changed |= ui.add_enabled_ui(!unlimited, |ui| {
                        ui.add_sized([88.0, 28.0], egui::DragValue::new(&mut shown_tail).range(fm_engine::prefs::LOG_TAIL_RANGE))
                            .labelled_by(caption.id)
                    }).inner.changed();
                    if !unlimited {
                        tail = shown_tail;
                    }
                    ui.end_row();
                });
            changed |= ui
                .checkbox(&mut unlimited, "Unlimited initial tail")
                .changed();
            if unlimited {
                tail = 0;
            } else {
                tail = tail.max(1);
            }
            hint(
                ui,
                tokens,
                "Initial size applies when opening logs. Unlimited removes this limit, not safety caps: up to 64 MiB for a single log or 32 MiB shared across selected files. Larger files still open at the tail; loading time and parser limits remain.",
            );
            if changed {
                s.dispatch_or_notice(
                    "prefs.preview.log",
                    &json!({"poll_ms": poll, "tail_mib": tail}),
                );
            }
            ui.add_space(18.0);
            crate::preview_filters::show(ui, s, tokens);
        }
        Topic::Terminal => {
            section_heading(ui, tokens, "New terminals");
            let shells: &[(&str, &str)] = if cfg!(windows) {
                &[("PowerShell", "default"), ("cmd", "cmd")]
            } else {
                &[("Login shell", "default"), ("PowerShell", "powershell")]
            };
            let current = match s.prefs.terminal_shell {
                TerminalShell::Cmd => "cmd",
                TerminalShell::PowerShell if !cfg!(windows) => "powershell",
                _ => "default",
            };
            if let Some(value) = choose(ui, tokens, "shell", "Shell", shells, current) {
                s.dispatch_or_notice("prefs.terminal.shell", &json!({"value": value}));
            }
            hint(
                ui,
                tokens,
                "Running terminals keep their current shell. Restart a terminal to use this setting.",
            );
            ui.horizontal(|ui| {
                let caption = ui.label("Scrollback rows");
                let mut rows = s.prefs.terminal_scrollback_rows() as u64;
                if ui
                    .add(
                        egui::DragValue::new(&mut rows)
                            .range(5_000..=1_000_000)
                            .speed(1_000),
                    )
                    .labelled_by(caption.id)
                    .changed()
                {
                    set(s, "terminal_scrollback_rows", json!(rows));
                }
            });
            hint(
                ui,
                tokens,
                "5,000-1,000,000 visual rows per terminal. Applies to new or restarted terminals. Large histories can use substantial memory; rows allocate as output arrives.",
            );
            ui.add_space(18.0);
            section_heading(ui, tokens, "Text");
            size_row(
                ui,
                s,
                "Font size",
                "terminal_font_size",
                s.prefs.term_font_size(),
            );
        }
        Topic::Connections => {
            section(ui, tokens, "SMB", None, |ui| {
                let mut encrypt = s.prefs.smb_require_encryption;
                if ui
                    .checkbox(&mut encrypt, "Require encrypted connections")
                    .on_hover_text("Refuse SMB connections whose session is not encrypted.")
                    .changed()
                {
                    s.dispatch_or_notice("prefs.smb.encryption", &json!({"value": encrypt}));
                }
            });

            let mut poll = s.prefs.remote_poll_secs;
            let mut timeout = s.prefs.remote_timeout_secs;
            let mut keepalive = s.prefs.remote_keepalive_secs;
            let mut changed = false;
            section(ui, tokens, "Remote connections", None, |ui| {
                egui::Grid::new("fm-prefs-sftp")
                        .num_columns(2)
                        .spacing([ui.spacing().item_spacing.x * 2.0, tokens.padding])
                        .show(ui, |ui| {
                            for (label, value, range, tip) in [
                                (
                                    "Refresh interval",
                                    &mut poll,
                                    fm_engine::REMOTE_POLL_RANGE,
                                    "How often connected remote panes re-read their folder; a dead link \
                                 is noticed at the next refresh.",
                                ),
                                (
                                    "Stalled call timeout",
                                    &mut timeout,
                                    fm_engine::REMOTE_TIMEOUT_RANGE,
                                    "A remote call that makes no progress for this long marks the \
                                 connection offline.",
                                ),
                                (
                                    "Keepalive interval",
                                    &mut keepalive,
                                    fm_engine::REMOTE_KEEPALIVE_RANGE,
                                    "Idle links are probed this often; three misses drop the link. \
                                 Applies to new connections.",
                                ),
                            ] {
                                let caption = ui.label(label);
                                changed |= ui
                                    .add_sized(
                                        [96.0, ui.spacing().interact_size.y],
                                        egui::DragValue::new(value)
                                            .range(range)
                                            .speed(0.2)
                                            .suffix(" s"),
                                    )
                                    .on_hover_text(tip)
                                    .labelled_by(caption.id)
                                    .changed();
                                ui.end_row();
                            }
                        });
            });
            if changed {
                s.dispatch_or_notice(
                    "prefs.remote.timing",
                    &json!({"poll_secs": poll, "timeout_secs": timeout, "keepalive_secs": keepalive}),
                );
            }
            let hosts: Vec<String> = s
                .remote
                .known_hosts()
                .entries()
                .iter()
                .map(|h| h.host.clone())
                .collect();
            section(
                ui,
                tokens,
                "Trusted hosts",
                Some("You will be asked to verify a host again after forgetting it."),
                |ui| {
                    if hosts.is_empty() {
                        hint(ui, tokens, "No trusted hosts.");
                    } else {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::Grid::new("fm-prefs-known-hosts")
                                .num_columns(2)
                                .max_col_width((ui.available_width() - 100.0).max(60.0))
                                .spacing([24.0, 8.0])
                                .show(ui, |ui| {
                                    for host in hosts {
                                        ui.add(egui::Label::new(&host).truncate())
                                            .on_hover_text(&host);
                                        if ui.button("Forget").clicked() {
                                            s.dispatch_or_notice(
                                                "remote.hostkey.forget",
                                                &json!({"host": host}),
                                            );
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    }
                },
            );
        }
        Topic::Workspaces => {
            section_heading(ui, tokens, "Switching");
            tab_order_row(ui, s, tokens);
            section_heading(ui, tokens, "Unsaved changes");
            unsaved_row(ui, s, tokens);
            section_heading(ui, tokens, "Saved workspaces");
            crate::workspaces::prefs_section(ui, s, tokens);
        }
    }
}

/// Where the thumbnail cache lives: shown, editable, and clearable.
fn cache_location(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    let shown = s
        .prefs
        .thumb_cache_path()
        .map_or_else(|| "(unavailable)".to_string(), |p| p.display().to_string());
    ui.horizontal(|ui| {
        ui.label("Stored in");
        ui.add(egui::Label::new(&shown).truncate())
            .on_hover_text(&shown);
    });
    let id = Id::NULL.with("fm-thumb-dir");
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| s.prefs.thumb_cache_dir.clone().unwrap_or_default());
    ui.horizontal(|ui| {
        let caption = ui.label("Parent folder");
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("Default location")
                    .desired_width(300.0),
            )
            .labelled_by(caption.id);
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if ui.button("Apply").clicked() || enter {
            set(s, "thumb_cache_dir", json!(text.trim()));
        }
        if ui
            .add_enabled(
                s.prefs.thumb_cache_dir.is_some(),
                egui::Button::new("Use default"),
            )
            .clicked()
        {
            set(s, "thumb_cache_dir", json!("default"));
            text.clear();
        }
    });
    hint(
        ui,
        tokens,
        "Thumbnails go in a \"thumbs\" folder inside it. Existing thumbnails stay in the old place.",
    );
    ui.add_space(4.0);
    if ui
        .button("Clear thumbnail cache")
        .on_hover_text("Delete every stored thumbnail, in memory and on disk.")
        .clicked()
    {
        s.dispatch_or_notice("preview.thumbs.clear_cache", &Params::Null);
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// The order Ctrl+Tab walks workspace tabs in (change `workspace-keys`).
fn tab_order_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    use fm_engine::prefs::WorkspaceTabOrder;
    if let Some(value) = choose(
        ui,
        tokens,
        "workspace_tab_order",
        "Ctrl+Tab order",
        &[
            ("Most recently used", WorkspaceTabOrder::Recent),
            ("Tab order", WorkspaceTabOrder::Cycle),
        ],
        s.prefs.workspace_tab_order,
    ) {
        let value = match value {
            WorkspaceTabOrder::Recent => "recent",
            WorkspaceTabOrder::Cycle => "cycle",
        };
        set(s, "workspace_tab_order", json!(value));
    }
    hint(
        ui,
        tokens,
        match s.prefs.workspace_tab_order {
            WorkspaceTabOrder::Recent => {
                "Ctrl+Tab returns to the tab used before; keep Ctrl held and press Tab again to go further back."
            }
            WorkspaceTabOrder::Cycle => {
                "Ctrl+Tab goes to the tab on the right, Ctrl+Shift+Tab to the left, wrapping around."
            }
        },
    );
}

/// What quitting or closing a tab does with unsaved workspaces ("Don't ask again" sets it).
fn unsaved_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    use fm_engine::prefs::UnsavedWorkspaces;
    if let Some(value) = choose(
        ui,
        tokens,
        "unsaved_workspaces",
        "When quitting or closing a tab",
        &[
            ("Ask", UnsavedWorkspaces::Ask),
            ("Save", UnsavedWorkspaces::Save),
            ("Don't save", UnsavedWorkspaces::Discard),
        ],
        s.prefs.unsaved_workspaces,
    ) {
        let value = match value {
            UnsavedWorkspaces::Ask => "ask",
            UnsavedWorkspaces::Save => "save",
            UnsavedWorkspaces::Discard => "discard",
        };
        set(s, "unsaved_workspaces", json!(value));
    }
    hint(
        ui,
        tokens,
        match s.prefs.unsaved_workspaces {
            UnsavedWorkspaces::Ask => {
                "Workspaces with unsaved changes ask whether to save them. Auto-save workspaces never ask."
            }
            UnsavedWorkspaces::Save => {
                "Changed workspaces are saved without asking. If saving fails, you are asked."
            }
            UnsavedWorkspaces::Discard => {
                "Changes to workspaces are left unsaved without asking. Auto-save workspaces still save."
            }
        },
    );
}

/// Default app or Media pane, separately for audio and video.
/// Windows: what draws the window (change `windows-wgpu`); applies at the next start.
fn renderer_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    use fm_engine::prefs::WindowsRenderer;
    ui.add_space(8.0);
    if let Some(value) = choose(
        ui,
        tokens,
        "windows_renderer",
        "Renderer",
        &[
            ("Direct3D", WindowsRenderer::Direct3d),
            ("OpenGL", WindowsRenderer::Opengl),
        ],
        s.prefs.windows_renderer,
    ) {
        let value = match value {
            WindowsRenderer::Direct3d => "direct3d",
            WindowsRenderer::Opengl => "opengl",
        };
        set(s, "windows_renderer", json!(value));
    }
    let in_use = fm_engine::diag::renderer().unwrap_or_else(|| "unknown".into());
    hint(
        ui,
        tokens,
        format!("In use: {in_use}. A change applies the next time the app starts."),
    );
}

/// Preferences → Editor (capabilities `text-editor`, `editor-open-preference`).
/// A clickable sample of a code colour scheme, in the app theme's lightness.
fn scheme_card(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    scheme: fm_engine::editor_prefs::EditorScheme,
    selected: bool,
) -> egui::Response {
    use egui::text::{LayoutJob, TextFormat};
    let code = crate::theme::CodePalette::of(scheme, tokens.dark);
    let font = egui::FontId::monospace((tokens.font_size - 1.0).max(9.0));
    let row = (font.size * 1.45).round();
    let lines: [&[(&str, egui::Color32, bool)]; 4] = [
        &[
            ("fn ", code.keyword, false),
            ("total", code.function, false),
            ("(xs: &[", code.punctuation, false),
            ("u32", code.type_name, false),
            ("]) {", code.punctuation, false),
        ],
        &[("    // add them up", code.comment, true)],
        &[
            ("    let ", code.keyword, false),
            ("n", code.text, false),
            (" = ", code.operator, false),
            ("42", code.number, false),
            (";", code.punctuation, false),
        ],
        &[
            ("    log(", code.text, false),
            ("\"done\"", code.string, false),
            (")", code.text, false),
        ],
    ];
    let size = egui::vec2(230.0, row * 4.0 + 46.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, scheme.label())
    });
    let painter = ui.painter();
    let radius = tokens.corner_radius.max(6.0);
    painter.rect_filled(rect, radius, code.bg);
    let stroke = if selected {
        egui::Stroke::new(2.5, tokens.accent)
    } else if response.hovered() {
        egui::Stroke::new(1.5, tokens.accent.gamma_multiply(0.6))
    } else {
        egui::Stroke::new(1.0, tokens.border)
    };
    painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    for (i, parts) in lines.iter().enumerate() {
        let mut job = LayoutJob::default();
        for (text, color, italics) in parts.iter() {
            job.append(
                text,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: *color,
                    italics: *italics,
                    ..Default::default()
                },
            );
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let y = rect.top() + 12.0 + row * i as f32;
        if i == 2 {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 2.0, y - 2.0),
                    egui::vec2(rect.width() - 4.0, row),
                ),
                0.0,
                code.line_bg,
            );
        }
        painter.galley(egui::pos2(rect.left() + 12.0, y), galley, code.text);
    }
    let name = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            scheme.label().to_string(),
            egui::FontId::proportional(tokens.font_size),
            code.text,
        )
    });
    painter.galley(
        egui::pos2(rect.left() + 12.0, rect.bottom() - name.size().y - 10.0),
        name,
        code.text,
    );
    if selected {
        let c = egui::pos2(rect.right() - 18.0, rect.bottom() - 18.0);
        painter.circle_filled(c, 8.0, tokens.accent);
        let st = egui::Stroke::new(2.0, tokens.bg);
        painter.line_segment([c + egui::vec2(-4.0, 0.0), c + egui::vec2(-1.0, 3.0)], st);
        painter.line_segment([c + egui::vec2(-1.0, 3.0), c + egui::vec2(4.0, -3.0)], st);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn editor_topic(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    use fm_engine::editor_prefs::{EditorKind, EditorTarget};
    section(
        ui,
        tokens,
        "Colours and spacing",
        Some(
            "Used by the editor and by code in the Preview pane. Each scheme has a dark and a light version that follow the app's theme.",
        ),
        |ui| {
            use fm_engine::editor_prefs::EditorScheme;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for scheme in EditorScheme::ALL {
                    let selected = s.prefs.editor.scheme == scheme;
                    if scheme_card(ui, tokens, scheme, selected).clicked() && !selected {
                        s.dispatch_or_notice(
                            "prefs.editor_scheme",
                            &json!({"scheme": scheme.id()}),
                        );
                    }
                }
            });
            let mut pct = s.prefs.editor.line_spacing_pct.clamp(
                fm_engine::editor_prefs::MIN_LINE_SPACING,
                fm_engine::editor_prefs::MAX_LINE_SPACING,
            );
            ui.horizontal(|ui| {
                let caption = ui.label("Line spacing");
                let response = ui
                    .add(
                        egui::Slider::new(
                            &mut pct,
                            fm_engine::editor_prefs::MIN_LINE_SPACING
                                ..=fm_engine::editor_prefs::MAX_LINE_SPACING,
                        )
                        .step_by(5.0)
                        .suffix(" %"),
                    )
                    .labelled_by(caption.id)
                    .on_hover_text("Height of each editor line, as a percentage of the text size");
                if response.changed() {
                    s.dispatch_or_notice("prefs.editor_line_spacing", &json!({"percent": pct}));
                }
            });
        },
    );
    section(
        ui,
        tokens,
        "Opening files",
        Some(
            "What Enter, a double-click and F4 do with each kind of text file. Open in External Editor is always in the right-click menu.",
        ),
        |ui| {
            let rows: Vec<(&str, &str, EditorTarget)> = EditorKind::ALL
                .iter()
                .map(|kind| {
                    let (title, detail) = match kind {
                        EditorKind::Text => ("Text", ".txt, .csv and any other text"),
                        EditorKind::Code => ("Code", ".rs, .py, .js, .sh and more"),
                        EditorKind::Markup => ("Markup", ".md, .html, .xml, .css"),
                        EditorKind::Config => ("Settings", ".toml, .json, .yaml, .ini"),
                        EditorKind::Log => ("Logs", ".log"),
                    };
                    (title, detail, s.prefs.editor.open.get(*kind))
                })
                .collect();
            let options = [
                ("Built-in editor", EditorTarget::Internal),
                ("External editor", EditorTarget::External),
            ];
            if let Some((row, target)) = open_table(ui, tokens, "editor", &rows, &options)
                && let Some(kind) = EditorKind::ALL.get(row)
            {
                let key = match target {
                    EditorTarget::Internal => "internal",
                    EditorTarget::External => "external",
                };
                s.dispatch_or_notice(
                    "prefs.editor_open",
                    &json!({"kind": kind.key(), "target": key}),
                );
            }
        },
    );
    section(
        ui,
        tokens,
        "Saving",
        Some(
            "Files are written through a temporary copy, and never over a file that changed on disk without asking.",
        ),
        |ui| {
            let mut autosave = s.prefs.editor.autosave;
            if ui
                .checkbox(&mut autosave, "Autosave on focus loss")
                .on_hover_text(
                    "Save when you click another pane, switch application, open another file or close the pane. Off: you are asked instead",
                )
                .changed()
            {
                s.dispatch_or_notice("prefs.editor_autosave", &json!({"enabled": autosave}));
            }
        },
    );
    section(ui, tokens, "Editing", None, |ui| {
        let mut steps = s.prefs.editor.undo_steps();
        ui.horizontal(|ui| {
            let caption = ui.label("Undo steps to remember");
            let response = ui
                .add(
                    egui::DragValue::new(&mut steps)
                        .range(fm_editor::MIN_STEPS..=fm_editor::MAX_STEPS)
                        .speed(10.0),
                )
                .labelled_by(caption.id)
                .on_hover_text("Per open file; older steps are dropped. History is also capped at 64 MiB of text");
            if response.changed() {
                s.dispatch_or_notice("prefs.editor_undo_steps", &json!({"steps": steps}));
            }
        });
        let mut wrap = s.prefs.editor.wrap;
        if ui.checkbox(&mut wrap, "Wrap long lines").changed() {
            s.dispatch_or_notice("prefs.editor_view", &json!({"wrap": wrap}));
        }
        let mut numbers = s.prefs.editor.line_numbers;
        if ui.checkbox(&mut numbers, "Show line numbers").changed() {
            s.dispatch_or_notice("prefs.editor_view", &json!({"line_numbers": numbers}));
        }
    });
    section(
        ui,
        tokens,
        "Syntax colouring",
        Some("Larger files are shown as plain text so typing stays smooth."),
        |ui| {
            let mut on = s.prefs.editor.syntax;
            if ui.checkbox(&mut on, "Colour source code").changed() {
                s.dispatch_or_notice("prefs.editor_syntax", &json!({"enabled": on}));
            }
            let mut rainbow = s.prefs.editor.rainbow;
            if ui
                .add_enabled(on, egui::Checkbox::new(&mut rainbow, "Rainbow brackets"))
                .on_hover_text(
                    "Colour brackets by how deeply they are nested; unmatched brackets show in red",
                )
                .changed()
            {
                s.dispatch_or_notice("prefs.editor_rainbow", &json!({"enabled": rainbow}));
            }
            let mut mib = s.prefs.editor.syntax_max_mib;
            ui.horizontal(|ui| {
                let caption = ui.label("Up to (MiB)");
                let response = ui
                    .add_enabled(
                        on,
                        egui::DragValue::new(&mut mib)
                            .range(1..=fm_engine::editor_prefs::MAX_SYNTAX_MIB),
                    )
                    .labelled_by(caption.id);
                if response.changed() {
                    s.dispatch_or_notice("prefs.editor_syntax", &json!({"max_mib": mib}));
                }
            });
        },
    );
}

fn media_open_choices(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    use fm_engine::prefs::MediaOpenTarget;
    let rows = [
        (
            "Audio",
            ".mp3, .flac, .ogg, .m4a, .wav…",
            s.prefs.media_open_audio,
        ),
        (
            "Video",
            ".mp4, .mkv, .webm, .mov, .avi…",
            s.prefs.media_open_video,
        ),
    ];
    let options = [
        ("Media pane", MediaOpenTarget::MediaPane),
        ("Default app", MediaOpenTarget::DefaultApp),
    ];
    if let Some((row, target)) = open_table(ui, tokens, "media", &rows, &options) {
        let kind = if row == 0 { "audio" } else { "video" };
        let key = match target {
            MediaOpenTarget::DefaultApp => "default",
            MediaOpenTarget::MediaPane => "media",
        };
        s.dispatch_or_notice("prefs.media_open", &json!({"kind": kind, "target": key}));
    }
}

/// The program that reads RAR archives: empty searches for 7-Zip or unrar.
fn rar_tool_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    let id = Id::NULL.with("fm-rar-tool");
    let found_id = Id::NULL.with("fm-rar-tool-found");
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| s.prefs.archives_rar_tool.clone());
    ui.horizontal(|ui| {
        let caption = ui.label("RAR program");
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("Find 7-Zip or unrar")
                    .desired_width(260.0),
            )
            .labelled_by(caption.id);
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if ui.button("Apply").clicked() || enter {
            set(s, "archives_rar_tool", json!(text.trim()));
            ui.data_mut(|d| d.remove::<Option<String>>(found_id));
        }
    });
    ui.data_mut(|d| d.insert_temp(id, text));
    // Searching PATH every frame is wasteful: remember the answer until Apply.
    let found = match ui.data(|d| d.get_temp::<Option<String>>(found_id)) {
        Some(found) => found,
        None => {
            let found = fm_engine::display_prefs_cmds::rar_tool_found();
            ui.data_mut(|d| d.insert_temp(found_id, found.clone()));
            found
        }
    };
    hint(
        ui,
        tokens,
        &match found {
            Some(tool) => format!("RAR archives open through {tool}."),
            None => "No 7-Zip or unrar found: RAR archives cannot be opened until one is installed or chosen here.".to_string(),
        },
    );
}

/// Name patterns added to the built-in archive names (`cbz=zip, epub=zip`).
fn patterns_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    let id = Id::NULL.with("fm-archive-patterns");
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| s.prefs.archives_patterns.clone());
    ui.horizontal(|ui| {
        let caption = ui.label("Also open as archives");
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("cbz=zip, epub=zip")
                    .desired_width(260.0),
            )
            .labelled_by(caption.id)
            .on_hover_text(
                "File name endings to open like the built-in archive names, each with the \
                 format to read it as: zip, tar, tar.gz, tar.bz2, tar.xz, tar.zst, 7z, iso, \
                 rar, gz, bz2, xz or zst. Separate them with commas.",
            );
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        if ui.button("Apply").clicked() || enter {
            set(s, "archives_patterns", json!(text.trim()));
            // Show the stored, tidied form; refused text stays for correcting.
            if fm_engine::archive_cmds::check_patterns(&text).is_ok() {
                text = s.prefs.archives_patterns.clone();
            }
        }
    });
    let check = fm_engine::archive_cmds::check_patterns(&text);
    ui.data_mut(|d| d.insert_temp(id, text));
    match check {
        Err(why) => {
            ui.colored_label(tokens.error, why);
        }
        Ok(()) => hint(
            ui,
            tokens,
            "Built in: zip, jar, tar, tar.gz, tgz, tar.bz2, tbz2, tar.xz, txz, tar.zst, tzst, 7z, \
             iso, rar, gz, bz2, xz, zst and numbered parts (.001, .z01, .r00).",
        ),
    }
}

/// The cap on temporary copies of archives (remote RAR sets).
fn spool_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    let mut gib = s.prefs.archives_spool_gib;
    ui.horizontal(|ui| {
        let caption = ui.label("Temporary copies, all archives together, up to");
        let response = ui
            .add_sized(
                [110.0, ui.spacing().interact_size.y],
                egui::DragValue::new(&mut gib)
                    .range(fm_engine::archive_cmds::SPOOL_CAP_GIB)
                    .suffix(" GiB"),
            )
            .labelled_by(caption.id);
        if response.changed() {
            set(s, "archives_spool_gib", json!(gib));
        }
    });
    let in_use = fm_engine::archive_cmds::temporary_copies_bytes() as f64 / f64::from(1u32 << 30);
    hint(
        ui,
        tokens,
        format!(
            "Open archives may keep a private temporary copy: a RAR on a server before 7-Zip \
             or unrar reads it, and a large .tar.gz, .tar.xz, .tar.bz2 or .tar.zst once \
             several files are read from it, so later reads are fast. A copy is skipped (and \
             reading stays slower) when it would pass this limit or leave less than a fifth \
             of the free disk space. Copies are deleted when their archive closes. In use \
             now: {in_use:.1} GiB."
        ),
    );
}

fn set(s: &mut Session, key: &str, value: serde_json::Value) {
    s.dispatch_or_notice("prefs.set", &json!({"key": key, "value": value}));
}

/// A labelled point-size field for the font setting `key`.
fn size_row(ui: &mut egui::Ui, s: &mut Session, label: &str, key: &str, current: f32) {
    let mut size = current;
    ui.horizontal(|ui| {
        let caption = ui.label(label);
        let response = ui
            .add_sized(
                [96.0, ui.spacing().interact_size.y],
                egui::DragValue::new(&mut size)
                    .range(fm_engine::prefs::FONT_SIZE_RANGE)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(" pt"),
            )
            .labelled_by(caption.id);
        if response.changed() {
            set(s, key, json!(size));
        }
    });
}

/// Accent presets, a custom `#rrggbb` field and a way back to the theme's own colour.
fn accent_row(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    let current = s.prefs.accent.clone();
    ui.horizontal(|ui| {
        let presets: Vec<(String, egui::Color32, &str)> = crate::theme::ACCENT_PRESETS
            .iter()
            .filter_map(|(name, hex)| {
                crate::theme::swatch(hex).map(|color| (format!("Accent {name}"), color, *hex))
            })
            .collect();
        let faces: Vec<Face<'_>> = presets
            .iter()
            .map(|(name, color, _)| Face::Swatch(*color, name))
            .collect();
        let selected = presets
            .iter()
            .position(|(_, _, hex)| current.as_deref() == Some(*hex));
        if let Some((_, _, hex)) = focus_nav::choice_row(
            ui,
            tokens,
            Id::NULL.with("fm-preferences-accent"),
            Some("Accent"),
            &faces,
            selected,
        )
        .and_then(|index| presets.get(index))
        {
            set(s, "accent", json!(hex));
        }
        if ui
            .add_enabled(current.is_some(), egui::Button::new("Default"))
            .clicked()
        {
            set(s, "accent", json!("default"));
        }
    });
    ui.horizontal(|ui| {
        let caption = ui.label("Custom colour");
        let id = Id::NULL.with("fm-accent-hex");
        let mut text = ui
            .data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| current.clone().unwrap_or_default());
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("#rrggbb")
                    .char_limit(7)
                    .desired_width(90.0),
            )
            .labelled_by(caption.id);
        if response.changed() && fm_engine::prefs::parse_hex_color(&text).is_some() {
            set(s, "accent", json!(text));
        }
        if !response.has_focus() {
            text = s.prefs.accent.clone().unwrap_or_default();
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    });
}

fn section(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    description: Option<&str>,
    contents: impl FnOnce(&mut egui::Ui),
) {
    section_heading(ui, tokens, title);
    if let Some(description) = description {
        hint(ui, tokens, description);
    }
    contents(ui);
    ui.add_space(12.0);
}

/// Where ffmpeg was found and, where a verified build is pinned, how to download it.
fn ffmpeg_section(ui: &mut egui::Ui, s: &mut Session, tokens: &Tokens) {
    let found = fm_preview::video::find_tool("ffmpeg").zip(fm_preview::video::find_tool("ffprobe"));
    let managed = found
        .as_ref()
        .zip(s.ffmpeg.dir())
        .is_some_and(|((tool, _), dir)| tool.starts_with(dir));
    match &found {
        Some((tool, _)) if managed => {
            let version = s.ffmpeg.installed_version().unwrap_or("?");
            ui.label(format!("Downloaded by the app (version {version})"));
            hint(ui, tokens, tool.display().to_string());
        }
        Some((tool, _)) => {
            ui.label("Found on this computer");
            hint(ui, tokens, tool.display().to_string());
        }
        None => {
            ui.label(RichText::new("Not found").color(tokens.warning));
        }
    }
    if s.ffmpeg.installing() {
        let (done, total) = s.ffmpeg.progress();
        let fraction = if total == 0 {
            0.0
        } else {
            (done as f64 / total as f64) as f32
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_width(260.0)
                .text(format!(
                    "{:.0} of {:.0} MB",
                    done as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                )),
        );
    } else if s.ffmpeg.downloadable() {
        let label = if s.ffmpeg.update_available() {
            format!(
                "Update to {}",
                s.ffmpeg.pinned_version().unwrap_or_default()
            )
        } else if managed {
            "Reinstall".to_string()
        } else if found.is_none() {
            "Download ffmpeg".to_string()
        } else {
            "Download a separate copy".to_string()
        };
        let enabled = s.command_enabled("tools.ffmpeg.install");
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text(
                "Downloads a pinned build over https and verifies its checksum before installing",
            )
            .clicked()
        {
            s.dispatch_or_notice("tools.ffmpeg.install", &serde_json::Value::Null);
        }
    } else if found.is_none() {
        hint(
            ui,
            tokens,
            "Install ffmpeg with your package manager (for example `brew install ffmpeg` or `apt install ffmpeg`).",
        );
    }
    if let Some(error) = s.ffmpeg.error.clone() {
        ui.label(RichText::new(error).color(tokens.error));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    /// Preferences over a pane: the "Behind pane" button is drawn first, like the real panes.
    fn walk_harness(size: egui::Vec2) -> Harness<'static, Session> {
        let mut session = Session::new().expect("session");
        session.preferences_open = true;
        Harness::builder().with_size(size).build_ui_state(
            move |ui, session| {
                let tokens = Tokens::dark();
                crate::theme::apply_to(ui.ctx(), &tokens);
                let _ = ui.button("Behind pane");
                show(ui.ctx(), session, &tokens);
            },
            session,
        )
    }

    fn focused(h: &Harness<'_, Session>) -> String {
        use egui_kittest::kittest::NodeT;
        h.query_by(|n| n.is_focused())
            .and_then(|n| n.accesskit_node().label())
            .unwrap_or_default()
    }

    fn press(h: &mut Harness<'_, Session>, key: Key, modifiers: egui::Modifiers) -> String {
        h.key_press_modifiers(modifiers, key);
        h.run();
        focused(h)
    }

    fn topic_of(h: &Harness<'_, Session>) -> &'static str {
        h.ctx
            .data_mut(|d| d.get_temp::<Topic>(Id::NULL.with("fm-preferences-topic")))
            .unwrap_or_default()
            .label()
    }

    #[test]
    fn tab_cycles_inside_the_dialog_and_never_reaches_the_panes() {
        use egui::Modifiers;
        let mut h = walk_harness(egui::vec2(1000.0, 700.0));
        h.run();
        assert_eq!(
            focused(&h),
            "General",
            "opening puts the keyboard on the topics"
        );
        let forward: Vec<String> = (0..3)
            .map(|_| press(&mut h, Key::Tab, Modifiers::NONE))
            .collect();
        assert_eq!(
            forward,
            ["Check for updates at startup", "Close", "General"]
        );
        let back: Vec<String> = (0..3)
            .map(|_| press(&mut h, Key::Tab, Modifiers::SHIFT))
            .collect();
        assert_eq!(back, ["Close", "Check for updates at startup", "General"]);

        // Topics are one stop: arrows select, and a full Tab round visits each control once.
        assert_eq!(press(&mut h, Key::ArrowDown, Modifiers::NONE), "Appearance");
        assert_eq!(topic_of(&h), "Appearance");
        let mut stops = Vec::new();
        for _ in 0..20 {
            let at = press(&mut h, Key::Tab, Modifiers::NONE);
            if at == "Appearance" {
                break;
            }
            stops.push(at);
        }
        assert_eq!(
            stops.first().map(String::as_str),
            Some("System"),
            "{stops:?}"
        );
        assert_eq!(stops.last().map(String::as_str), Some("Close"), "{stops:?}");
        for skipped in ["Dark", "Light", "Compact", "Comfortable", "Behind pane"] {
            assert!(
                !stops.iter().any(|s| s == skipped),
                "{skipped} in {stops:?}"
            );
        }
        assert!(
            stops.iter().any(|s| s == "Normal"),
            "the selected density is the stop"
        );

        // A choice row: the arrows choose and apply, and the focus follows.
        assert_eq!(press(&mut h, Key::Tab, Modifiers::NONE), "System");
        assert_eq!(press(&mut h, Key::ArrowRight, Modifiers::NONE), "Dark");
        assert_eq!(h.state().prefs.theme, ThemePreference::Dark);
        assert_eq!(press(&mut h, Key::End, Modifiers::NONE), "Light");
        assert_eq!(h.state().prefs.theme, ThemePreference::Light);

        // Topic switching from anywhere in the dialog.
        assert_eq!(press(&mut h, Key::Tab, Modifiers::CTRL), "Files");
        assert_eq!(topic_of(&h), "Files");
        assert_eq!(
            press(&mut h, Key::Tab, Modifiers::CTRL | Modifiers::SHIFT),
            "Appearance"
        );
        assert_eq!(press(&mut h, Key::PageDown, Modifiers::CTRL), "Files");
        assert_eq!(press(&mut h, Key::End, Modifiers::NONE), "Advanced");
        assert_eq!(press(&mut h, Key::Home, Modifiers::NONE), "General");
        assert_eq!(
            press(&mut h, Key::PageUp, Modifiers::CTRL),
            "Advanced",
            "wraps"
        );
        assert!(h.state().preferences_open);
    }

    #[test]
    fn tabbing_scrolls_the_focused_control_into_view() {
        let mut h = walk_harness(egui::vec2(900.0, 650.0));
        h.run();
        h.get_by_label("Media").click();
        h.run();
        let close = h.get_by_label("Close").rect();
        let mut reached = false;
        for _ in 0..60 {
            let at = press(&mut h, Key::Tab, egui::Modifiers::NONE);
            if at == "Distance from bottom" {
                reached = true;
                break;
            }
        }
        assert!(reached, "Tab reaches the end of a long topic");
        let shown = h
            .get_all_by_label("Distance from bottom")
            .map(|n| n.rect())
            .find(|r| r.width() > 0.0)
            .expect("slider");
        assert!(
            shown.bottom() <= close.top() && shown.top() > 100.0,
            "{shown:?} is scrolled into view above the footer ({close:?})"
        );
    }

    fn harness(tokens: Tokens, size: egui::Vec2) -> Harness<'static, Session> {
        let mut session = Session::new().expect("session");
        session.preferences_open = true;
        Harness::builder().with_size(size).build_ui_state(
            move |ui, session| {
                crate::theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                show(ui.ctx(), session, &tokens);
            },
            session,
        )
    }

    #[test]
    fn new_settings_dispatch_through_the_command() {
        let mut h = harness(Tokens::dark(), egui::vec2(1000.0, 700.0));
        h.get_by_label("Files").click();
        h.run();
        for label in [
            "Open with a single click",
            "Ask before deleting permanently",
        ] {
            h.get_by_label(label).scroll_to_me();
            h.run();
            h.get_by_label(label).click();
            h.run();
        }
        h.get_by_label("Bytes").scroll_to_me();
        h.run();
        h.get_by_label("Bytes").click();
        h.run();
        assert!(h.state().prefs.single_click_open);
        assert!(!h.state().prefs.confirm_delete);
        assert_eq!(h.state().prefs.size_units, SizeUnits::Bytes);
        h.get_by_label("Appearance").click();
        h.run();
        h.get_by_label("Compact").click();
        h.get_by_label("Accent Orange").click();
        h.run();
        assert_eq!(h.state().prefs.density, Density::Compact);
        assert_eq!(h.state().prefs.accent.as_deref(), Some("#f0913a"));
        h.get_by_label("Default").click();
        h.run();
        assert!(h.state().prefs.accent.is_none());
        crate::listing::set_display(SizeUnits::Binary, DateFormat::Iso);
    }

    #[test]
    fn the_windows_renderer_row_switches_the_preference() {
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(700.0, 200.0))
            .build_ui_state(
                move |ui, s: &mut Session| renderer_row(ui, s, &tokens),
                Session::new().expect("session"),
            );
        h.run();
        h.get_by_label("Direct3D");
        h.get_by_label("OpenGL").click();
        h.run();
        assert_eq!(
            h.state().prefs.windows_renderer,
            fm_engine::prefs::WindowsRenderer::Opengl
        );
    }

    #[test]
    fn the_unsaved_changes_row_switches_the_preference() {
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(700.0, 200.0))
            .build_ui_state(
                move |ui, s: &mut Session| unsaved_row(ui, s, &tokens),
                Session::new().expect("session"),
            );
        h.run();
        h.get_by_label("Don't save").click();
        h.run();
        assert_eq!(
            h.state().prefs.unsaved_workspaces,
            fm_engine::prefs::UnsavedWorkspaces::Discard
        );
        h.get_by_label("Ask").click();
        h.run();
        assert_eq!(
            h.state().prefs.unsaved_workspaces,
            fm_engine::prefs::UnsavedWorkspaces::Ask
        );
    }

    #[test]
    fn the_tab_order_row_switches_the_preference() {
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(700.0, 200.0))
            .build_ui_state(
                move |ui, s: &mut Session| tab_order_row(ui, s, &tokens),
                Session::new().expect("session"),
            );
        h.run();
        h.get_by_label("Most recently used");
        h.get_by_label("Tab order").click();
        h.run();
        assert_eq!(
            h.state().prefs.workspace_tab_order,
            fm_engine::prefs::WorkspaceTabOrder::Cycle
        );
    }

    #[test]
    fn troubleshooting_offers_the_diagnostic_log() {
        let mut h = harness(Tokens::dark(), egui::vec2(1000.0, 700.0));
        h.get_by_label("Advanced").click();
        h.run();
        // Not clicked: turning the log on would write into this machine's data folder.
        h.get_by_label("Write diagnostic log").scroll_to_me();
        h.run();
        h.get_by_label("Write diagnostic log");
        h.get_by_label("Open log folder");
        assert!(!h.state().prefs.diagnostics_log);
    }

    #[test]
    fn topics_show_only_their_settings_and_preserve_edits_in_both_themes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut h = harness(tokens, egui::vec2(1000.0, 700.0));
            let updates = h.state().prefs.check_updates;
            h.get_by_label("Check for updates at startup").click();
            h.run();
            assert_eq!(h.state().prefs.check_updates, !updates);
            let rect = h
                .ctx
                .memory(|m| m.area_rect(Id::NULL.with("fm-preferences")))
                .expect("window");
            for (topic, setting) in [
                ("Appearance", "Theme"),
                ("Files", "Show hidden files at startup"),
                ("Copy and move", "Re-target links when copying"),
                ("Preview", "Recursive folder preview"),
                ("Terminal", "Shell"),
                ("Connections", "Require encrypted connections"),
                ("Workspaces", "No saved workspaces (Workspace > Save)."),
                ("Keyboard shortcuts", "Search shortcuts"),
                ("Advanced", "Memory cache"),
            ] {
                h.get_by_label(topic).click();
                h.run();
                h.get_by_label(setting);
                assert!(h.query_by_label("Check for updates at startup").is_none());
                let current = h
                    .ctx
                    .memory(|m| m.area_rect(Id::NULL.with("fm-preferences")))
                    .expect("window");
                assert!((current.width() - rect.width()).abs() < 1.0, "{topic}");
                assert!((current.height() - rect.height()).abs() < 1.0, "{topic}");
            }
            h.get_by_label("Appearance").click();
            h.run();
            h.get_by_label("Light").click();
            h.run();
            assert_eq!(h.state().prefs.theme, ThemePreference::Light);
            h.get_by_label("Copy and move").click();
            h.run();
            let retarget = h.state().prefs.retarget_links;
            h.get_by_label("Re-target links when copying").click();
            h.run();
            assert_eq!(h.state().prefs.retarget_links, !retarget);
            h.get_by_label("Preview").click();
            h.run();
            let recursive = h.state().prefs.recursive_preview;
            h.get_by_label("Recursive folder preview").click();
            h.run();
            assert_eq!(h.state().prefs.recursive_preview, !recursive);
            h.get_by_label("Connections").click();
            h.run();
            let encryption = h.state().prefs.smb_require_encryption;
            h.get_by_label("Require encrypted connections").click();
            h.run();
            assert_eq!(h.state().prefs.smb_require_encryption, !encryption);
            h.get_by_label("Refresh interval");
            h.get_by_label("Stalled call timeout");
            h.get_by_label("Keepalive interval");
            h.get_by_label("General").click();
            h.run();
            assert_eq!(h.state().prefs.check_updates, !updates);
        }
    }

    #[test]
    fn closing_and_reopening_remembers_topic_without_reverting_settings() {
        let mut h = harness(Tokens::dark(), egui::vec2(900.0, 650.0));
        h.get_by_label("Preview").click();
        h.run();
        h.get_by_label("Close").click();
        h.run();
        assert!(!h.state().preferences_open);
        h.state_mut().preferences_open = true;
        h.run();
        h.get_by_label("Recursive folder preview");
        h.key_press(Key::Escape);
        h.run();
        assert!(!h.state().preferences_open);
    }

    #[test]
    fn escape_in_a_text_field_leaves_it_before_closing() {
        let mut h = harness(Tokens::dark(), egui::vec2(900.0, 650.0));
        h.get_by_label("Preview").click();
        h.run();
        let fields = h
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .count();
        h.get_by_label("Filename patterns for Logs").scroll_to_me();
        h.run();
        h.get_by_label("PDF").click();
        h.run();
        h.get_by_label("*.pdf").click();
        h.run();
        // A second click on the selected pattern edits it in place.
        h.get_by_label("*.pdf").click();
        h.run();
        assert_eq!(
            h.query_all_by_role(egui::accesskit::Role::TextInput)
                .count(),
            fields + 1
        );
        h.key_press(Key::Escape);
        h.run();
        assert!(h.state().preferences_open, "Esc cancelled the edit only");
        assert_eq!(
            h.query_all_by_role(egui::accesskit::Role::TextInput)
                .count(),
            fields
        );
        h.key_press(Key::Escape);
        h.run();
        assert!(!h.state().preferences_open);
    }

    #[test]
    fn small_viewport_keeps_close_and_settings_inside_window() {
        let mut h = harness(Tokens::light(), egui::vec2(900.0, 650.0));
        for i in 0..10 {
            h.state_mut()
                .dispatch("workspace.new", &json!({"name": format!("Saved {i}")}))
                .expect("new");
            h.state_mut()
                .dispatch("workspace.save", &json!({}))
                .expect("save");
        }
        h.set_size(egui::vec2(560.0, 320.0));
        h.run();
        for topic in [
            "Connections",
            "Workspaces",
            "General",
            "Appearance",
            "Copy and move",
            "Preview",
            "Terminal",
            "Keyboard shortcuts",
        ] {
            h.get_all_by_label(topic)
                .next()
                .expect("topic")
                .scroll_to_me();
            h.run();
            h.get_all_by_label(topic).next().expect("topic").click();
            h.run();
            assert_eq!(
                h.ctx
                    .data_mut(|d| d.get_temp::<Topic>(Id::NULL.with("fm-preferences-topic")))
                    .expect("topic")
                    .label(),
                topic
            );
            let rect = h
                .ctx
                .memory(|m| m.area_rect(Id::NULL.with("fm-preferences")))
                .expect("window");
            assert!(rect.min.x >= 0.0 && rect.max.x <= 560.0, "{rect:?}");
            assert!(rect.min.y >= 0.0 && rect.max.y <= 320.0, "{rect:?}");
            let close = h.get_by_label("Close").rect();
            assert!(rect.contains_rect(close), "Close is clipped: {close:?}");
        }
    }

    #[test]
    fn shortcuts_are_searchable_read_only_and_remembered_in_both_themes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut h = harness(tokens, egui::vec2(1000.0, 700.0));
            let before = serde_json::to_value(&h.state().prefs).expect("prefs");
            h.get_by_label("Keyboard shortcuts").click();
            h.run();
            h.get_by_label("Application and appearance");
            h.get_by_label("Search shortcuts").click();
            h.get_by_label("Search shortcuts").type_text("scrollback");
            h.run();
            h.get_by_label("Scroll terminal history");
            h.get_by_label("Shift+PageUp / Shift+PageDown");
            assert!(h.query_by_label("Application and appearance").is_none());
            h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
            h.get_by_label("Search shortcuts")
                .type_text("no-such-shortcut");
            h.run();
            h.get_by_label("No shortcuts match your search.");
            h.get_by_label("Close").click();
            h.run();
            h.state_mut().preferences_open = true;
            h.run();
            h.get_by_label("Search shortcuts");
            assert_eq!(
                serde_json::to_value(&h.state().prefs).expect("prefs"),
                before
            );
        }
    }

    #[test]
    fn settings_font_is_scoped_and_remote_controls_still_dispatch() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut h = harness(tokens, egui::vec2(900.0, 650.0));
            h.run();
            let window = h
                .ctx
                .memory(|m| m.area_rect(Id::NULL.with("fm-preferences")))
                .expect("window");
            assert!(h.ctx.content_rect().contains_rect(window));
            assert!(window.contains_rect(h.get_by_label("Close").rect()));
            assert_eq!(
                h.ctx
                    .style_of(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    })
                    .text_styles[&egui::TextStyle::Body]
                    .family,
                egui::FontFamily::Monospace,
                "Preferences must not change the rest of the application's font"
            );
            h.get_by_label("Connections").click();
            h.run();
            h.get_by_label("Refresh interval").click();
            h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
            h.run();
            h.get_by_label("Refresh interval").type_text("20");
            h.key_press(Key::Enter);
            h.run();
            assert_eq!(h.state().prefs.remote_poll_secs, 20);
            h.get_by_label("Terminal").click();
            h.run();
            let (label, shell) = if cfg!(windows) {
                ("cmd", TerminalShell::Cmd)
            } else {
                ("PowerShell", TerminalShell::PowerShell)
            };
            h.get_by_label(label).click();
            h.run();
            assert_eq!(h.state().prefs.terminal_shell, shell);
        }
    }

    #[test]
    fn trusted_host_rows_keep_forget_actions_aligned_and_remove_only_the_target() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let dir = tempfile::tempdir().expect("config directory");
            std::fs::write(
                dir.path().join(fm_engine::remote::KNOWN_HOSTS_FILE),
                json!({"hosts": [
                    {"host": "short.test:22", "algorithm": "ssh-ed25519",
                     "fingerprint": "fixture", "openssh": "fixture"},
                    {"host": "longer-host.example.test:22", "algorithm": "ssh-ed25519",
                     "fingerprint": "fixture", "openssh": "fixture"}
                ]})
                .to_string(),
            )
            .expect("known hosts");
            let mut h = harness(tokens, egui::vec2(900.0, 650.0));
            h.state_mut().remote.set_config_dir(dir.path().into());
            h.get_by_label("Connections").click();
            h.run();
            h.run_steps(3);
            let buttons: Vec<_> = h.get_all_by_label("Forget").collect();
            assert_eq!(buttons.len(), 2);
            assert!((buttons[0].rect().left() - buttons[1].rect().left()).abs() < 1.0);
            buttons[0].click();
            h.run();
            let hosts = h.state().remote.known_hosts();
            assert_eq!(hosts.len(), 1);
            assert_eq!(hosts.entries()[0].host, "longer-host.example.test:22");
            assert_eq!(fm_engine::remote::KnownHosts::load(dir.path()), hosts);
        }
    }

    #[test]
    fn workspace_controls_keep_dispatching_from_the_selected_topic() {
        let mut h = harness(Tokens::dark(), egui::vec2(900.0, 650.0));
        h.state_mut()
            .dispatch("workspace.new", &json!({"name": "Work"}))
            .expect("new");
        h.state_mut()
            .dispatch("workspace.save", &json!({}))
            .expect("save");
        h.get_by_label("Workspaces").click();
        h.run();
        h.get_by_label("Work");
        h.get_by_label("Work Auto-save").click();
        h.run();
        assert!(
            h.state()
                .workspaces
                .saved
                .first()
                .expect("workspace")
                .auto_save
        );
        h.get_by_label("Delete").click();
        h.run();
        assert!(h.state().workspaces.saved.is_empty());
        h.get_by_label("No saved workspaces (Workspace > Save).");
    }

    #[test]
    fn preview_audio_autoplay_is_opt_in() {
        let mut h = harness(Tokens::dark(), egui::vec2(900.0, 650.0));
        h.get_by_label("Preview").click();
        h.run();
        assert!(!h.state().prefs.preview_audio_autoplay);
        assert!(h.state().prefs.preview_video_autoplay);
        h.get_by_label("Autoplay video previews").click();
        h.run();
        assert!(!h.state().prefs.preview_video_autoplay);
        h.get_by_label("Autoplay audio previews").click();
        h.run();
        assert!(h.state().prefs.preview_audio_autoplay);
        h.get_by_label("Autoplay audio previews").click();
        h.run();
        assert!(!h.state().prefs.preview_audio_autoplay);
    }

    #[test]
    fn log_settings_dispatch_and_unlimited_keeps_safety_caps() {
        let mut h = harness(Tokens::dark(), egui::vec2(900.0, 650.0));
        h.get_by_label("Preview").click();
        h.run();
        h.get_by_label("Live tail interval (ms)").scroll_to_me();
        h.run();
        h.get_by_label("Live tail interval (ms)").click();
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        h.run();
        h.get_by_label("Live tail interval (ms)").type_text("2000");
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(h.state().prefs.log_poll_ms, 2000);
        h.get_by_label("Unlimited initial tail").scroll_to_me();
        h.run();
        h.get_by_label("Unlimited initial tail").click();
        h.run();
        assert_eq!(h.state().prefs.log_tail_mib, 0);
        assert_eq!(h.state().prefs.log_limits().initial_tail, 64 << 20);
        h.get_by_label("Unlimited initial tail").click();
        h.run();
        assert_eq!(h.state().prefs.log_tail_mib, 1);
        h.get_by_label("Initial tail size (MiB)");
    }
}
