//! Audiobook controls of the Media pane: skip, chapters, bookmarks and book marks.

use egui::Ui;
use fm_engine::Session;
use serde_json::{Value, json};

use crate::media::clock;

type Actions = Vec<(&'static str, Value)>;

/// Skip, chapter and bookmark controls; shown only while the current item is a book.
pub(crate) fn controls(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    let Some(view) = s.media.book_view() else {
        return;
    };
    let back = s.prefs.media_skip_back_secs;
    let forward = s.prefs.media_skip_forward_secs;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(format!("Back {back:.0}s"))
            .on_hover_text("Skip back")
            .clicked()
        {
            actions.push(("media.skip", json!({"direction": "back"})));
        }
        if ui
            .button(format!("Forward {forward:.0}s"))
            .on_hover_text("Skip forward")
            .clicked()
        {
            actions.push(("media.skip", json!({"direction": "forward"})));
        }
        let chapters = view.chapters.len();
        ui.add_enabled_ui(chapters > 1, |ui| {
            if ui
                .button("Prev chapter")
                .on_hover_text("Previous chapter")
                .clicked()
            {
                actions.push(("media.chapter", json!({"delta": -1})));
            }
            if ui
                .button("Next chapter")
                .on_hover_text("Next chapter")
                .clicked()
            {
                actions.push(("media.chapter", json!({"delta": 1})));
            }
            let title = view
                .chapter
                .and_then(|at| view.chapters.get(at))
                .map_or_else(|| "Chapters".to_string(), |c| c.title.clone());
            ui.menu_button(format!("{title}"), |ui| {
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
            });
        });
        ui.menu_button(format!("Bookmarks ({})", view.bookmarks.len()), |ui| {
            if ui.button("Add bookmark here").clicked() {
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
                    if ui.button(label).clicked() {
                        actions.push(("media.bookmark", json!({"action": "jump", "index": at})));
                        ui.close();
                    }
                    if ui
                        .small_button("x")
                        .on_hover_text("Remove bookmark")
                        .clicked()
                    {
                        actions.push(("media.bookmark", json!({"action": "remove", "index": at})));
                        ui.close();
                    }
                });
            }
        });
    });
}

/// Context-menu entries that mark a row's book finished or as an audiobook.
pub(crate) fn menu_items(ui: &mut Ui, index: usize, actions: &mut Actions) {
    ui.separator();
    for (label, command, on) in [
        ("Mark as finished", "media.finished", true),
        ("Mark as unfinished", "media.finished", false),
        ("Treat as audiobook", "media.book", true),
        ("Treat as music", "media.book", false),
    ] {
        if ui.button(label).clicked() {
            actions.push((command, json!({"index": index, "on": on})));
            ui.close();
        }
    }
    ui.separator();
}
