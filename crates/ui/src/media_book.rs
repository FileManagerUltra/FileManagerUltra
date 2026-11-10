//! Audiobook controls of the Media pane: skip, chapters, bookmarks and book marks.

use egui::Ui;
use fm_engine::Session;
use serde_json::{Value, json};

use crate::media::clock;
use crate::media_icons::{Icon, button};
use crate::theme::Tokens;

type Actions = Vec<(&'static str, Value)>;

/// Skip, chapter and bookmark controls; shown only while the current item is a book.
pub(crate) fn controls(ui: &mut Ui, s: &Session, tokens: &Tokens, actions: &mut Actions) {
    let Some(view) = s.media.book_view() else {
        return;
    };
    let back = s.prefs.media_skip_back_secs;
    let forward = s.prefs.media_skip_forward_secs;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if button(
            ui,
            tokens,
            Icon::SkipBack,
            &format!("Back {back:.0} s"),
            true,
            false,
        )
        .clicked()
        {
            actions.push(("media.skip", json!({"direction": "back"})));
        }
        if button(
            ui,
            tokens,
            Icon::SkipForward,
            &format!("Forward {forward:.0} s"),
            true,
            false,
        )
        .clicked()
        {
            actions.push(("media.skip", json!({"direction": "forward"})));
        }
        let chapters = view.chapters.len() > 1;
        if button(
            ui,
            tokens,
            Icon::ChapterPrev,
            "Previous chapter",
            chapters,
            false,
        )
        .clicked()
        {
            actions.push(("media.chapter", json!({"delta": -1})));
        }
        if button(
            ui,
            tokens,
            Icon::ChapterNext,
            "Next chapter",
            chapters,
            false,
        )
        .clicked()
        {
            actions.push(("media.chapter", json!({"delta": 1})));
        }
        ui.add_enabled_ui(chapters, |ui| {
            let title = view
                .chapter
                .and_then(|at| view.chapters.get(at))
                .map_or_else(|| "Chapters".to_string(), |c| c.title.clone());
            egui::ComboBox::from_id_salt("media-chapters")
                .selected_text(title)
                .width(150.0)
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (at, chapter) in view.chapters.iter().enumerate() {
                                let label = format!("{}  {}", clock(chapter.start), chapter.title);
                                if ui
                                    .add(egui::Button::selectable(view.chapter == Some(at), label))
                                    .clicked()
                                {
                                    actions.push(("media.chapter", json!({"index": at})));
                                    ui.close();
                                }
                            }
                        });
                })
                .response
                .on_hover_text("Jump to a chapter")
                .on_disabled_hover_text("This book has no chapters");
        });
        egui::ComboBox::from_id_salt("media-bookmarks")
            .selected_text(format!("Bookmarks ({})", view.bookmarks.len()))
            .width(150.0)
            .show_ui(ui, |ui| {
                if ui
                    .button("Add bookmark here")
                    .on_hover_text("Save the current position")
                    .clicked()
                {
                    actions.push(("media.bookmark", json!({"action": "add"})));
                    ui.close();
                }
                if !view.bookmarks.is_empty() {
                    ui.separator();
                }
                for (at, mark) in view.bookmarks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let label = if mark.note.is_empty() {
                            clock(mark.position)
                        } else {
                            format!("{}  {}", clock(mark.position), mark.note)
                        };
                        if ui
                            .button(label)
                            .on_hover_text("Jump to this bookmark")
                            .clicked()
                        {
                            actions
                                .push(("media.bookmark", json!({"action": "jump", "index": at})));
                            ui.close();
                        }
                        if ui
                            .small_button("x")
                            .on_hover_text("Remove bookmark")
                            .clicked()
                        {
                            actions
                                .push(("media.bookmark", json!({"action": "remove", "index": at})));
                            ui.close();
                        }
                    });
                }
            })
            .response
            .on_hover_text("Saved positions in this book");
    });
}

/// Context-menu entries that mark a row's book finished or as an audiobook.
pub(crate) fn menu_items(ui: &mut Ui, index: usize, actions: &mut Actions) {
    ui.separator();
    for (label, tip, command, on) in [
        (
            "Mark as finished",
            "Next play starts from the top",
            "media.finished",
            true,
        ),
        (
            "Mark as unfinished",
            "Keep the saved position",
            "media.finished",
            false,
        ),
        (
            "Treat as audiobook",
            "Adds chapters, skip, bookmarks and resume",
            "media.book",
            true,
        ),
        (
            "Treat as music",
            "Plays without remembering a position",
            "media.book",
            false,
        ),
    ] {
        if ui.button(label).on_hover_text(tip).clicked() {
            actions.push((command, json!({"index": index, "on": on})));
            ui.close();
        }
    }
    ui.separator();
}
