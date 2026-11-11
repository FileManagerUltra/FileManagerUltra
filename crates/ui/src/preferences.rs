//! The Preferences dialog. Pure view over `Session::prefs`: every change is a command
//! (`view.theme.*`, `prefs.links.retarget`), so the UI never edits settings itself.

use egui::{Context, Id, Key, RichText};
use serde_json::json;

use fm_engine::{DateFormat, Density, Params, Session, SizeUnits, TerminalShell, ThemePreference};

use crate::dialog_style::{hint, section_heading};
use crate::theme::Tokens;

#[path = "preferences_shortcuts.rs"]
mod shortcuts;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Topic {
    #[default]
    General,
    Appearance,
    Files,
    FileOperations,
    Preview,
    Media,
    Terminal,
    Connections,
    Workspaces,
    Shortcuts,
    Advanced,
}

impl Topic {
    const ALL: [Self; 11] = [
        Self::General,
        Self::Appearance,
        Self::Files,
        Self::FileOperations,
        Self::Preview,
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
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    if !s.preferences_open {
        return;
    }
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let topic_id = Id::NULL.with("fm-preferences-topic");
    let mut topic = ctx.data_mut(|data| data.get_temp::<Topic>(topic_id).unwrap_or_default());
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
                            for item in Topic::ALL {
                                let response = ui.add_sized(
                                    [160.0, 32.0],
                                    egui::Button::selectable(topic == item, "")
                                        .left_text(item.label())
                                        .corner_radius(2.0),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::Role::Button,
                                        response.enabled(),
                                        item.label(),
                                    )
                                });
                                if response.clicked() {
                                    topic = item;
                                }
                            }
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
                        .show(ui, |ui| settings(ui, s, tokens, topic));
                });
            });
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(width, 32.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized([88.0, 30.0], egui::Button::new("Close"))
                            .clicked()
                        {
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
        });
    ctx.data_mut(|data| data.insert_temp(topic_id, topic));
    if close || !open {
        s.preferences_open = false;
    }
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
            ui.horizontal(|ui| {
                ui.label("Theme");
                for (label, id, value) in [
                    ("System", "view.theme.system", ThemePreference::System),
                    ("Dark", "view.theme.dark", ThemePreference::Dark),
                    ("Light", "view.theme.light", ThemePreference::Light),
                ] {
                    if ui.radio(s.prefs.theme == value, label).clicked() {
                        s.dispatch_or_notice(id, &Params::Null);
                    }
                }
            });
            ui.add_space(8.0);
            accent_row(ui, s);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Density");
                for (label, value, current) in [
                    ("Compact", "compact", Density::Compact),
                    ("Normal", "normal", Density::Normal),
                    ("Comfortable", "comfortable", Density::Comfortable),
                ] {
                    if ui.radio(s.prefs.density == current, label).clicked() {
                        set(s, "density", json!(value));
                    }
                }
            });
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
            section(ui, tokens, "Display", None, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Sizes");
                    for (label, value, current) in [
                        ("KiB (1024)", "binary", SizeUnits::Binary),
                        ("kB (1000)", "decimal", SizeUnits::Decimal),
                        ("Bytes", "bytes", SizeUnits::Bytes),
                    ] {
                        if ui.radio(s.prefs.size_units == current, label).clicked() {
                            set(s, "size_units", json!(value));
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Dates").on_hover_text(
                        "File times use your operating system's timezone, detected at startup, including daylight saving. Log time filters remain UTC.",
                    );
                    for (label, value, current) in [
                        ("2026-10-28 13:40", "iso", DateFormat::Iso),
                        ("2026-10-28 13:40:05", "isoseconds", DateFormat::IsoSeconds),
                        ("28/10/2026", "eu", DateFormat::Eu),
                        ("10/28/2026", "us", DateFormat::Us),
                    ] {
                        if ui.radio(s.prefs.date_format == current, label).clicked() {
                            set(s, "date_format", json!(value));
                        }
                    }
                });
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
        }
        Topic::Media => {
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
            ui.horizontal(|ui| {
                ui.label("Shell");
                let choices: &[(&str, &str, TerminalShell)] = if cfg!(windows) {
                    &[
                        ("PowerShell", "default", TerminalShell::Default),
                        ("cmd", "cmd", TerminalShell::Cmd),
                    ]
                } else {
                    &[
                        ("Login shell", "default", TerminalShell::Default),
                        ("PowerShell", "powershell", TerminalShell::PowerShell),
                    ]
                };
                for (label, value, shell) in choices {
                    let current = s.prefs.terminal_shell == *shell
                        || (cfg!(windows)
                            && *shell == TerminalShell::Default
                            && s.prefs.terminal_shell == TerminalShell::PowerShell);
                    if ui
                        .radio(current, *label)
                        .on_hover_text("Used by new terminals; restart a running one to switch.")
                        .clicked()
                    {
                        s.dispatch_or_notice("prefs.terminal.shell", &json!({"value": value}));
                    }
                }
            });
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
fn accent_row(ui: &mut egui::Ui, s: &mut Session) {
    let current = s.prefs.accent.clone();
    ui.horizontal(|ui| {
        ui.label("Accent");
        for (name, hex) in crate::theme::ACCENT_PRESETS {
            let Some(color) = crate::theme::swatch(hex) else {
                continue;
            };
            let chosen = current.as_deref() == Some(hex);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Accent {name}"))
            });
            ui.painter().rect_filled(rect.shrink(2.0), 4.0, color);
            if chosen || response.hovered() {
                ui.painter().rect_stroke(
                    rect,
                    5.0,
                    egui::Stroke::new(2.0, ui.visuals().text_color()),
                    egui::StrokeKind::Inside,
                );
            }
            if response.on_hover_text(name).clicked() {
                set(s, "accent", json!(hex));
            }
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
        h.get_by_label("Open with a single click").click();
        h.get_by_label("Ask before deleting permanently").click();
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
