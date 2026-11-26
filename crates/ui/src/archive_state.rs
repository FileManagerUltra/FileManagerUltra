//! What a Files pane shows in place of a listing when an archive cannot be opened for a
//! reason the user can act on (`archive-browsing`): no 7-Zip or unrar for a RAR, an
//! archive nested deeper than the limit, or a file whose contents do not match its name.
//! Each state says what happened and offers the way out as buttons.

use egui::{RichText, Ui};
use fm_engine::Session;
use fm_engine::archive_cmds::ArchiveProblem;
use fm_layout::PaneId;
use fm_vfs::VfsUri;
use serde_json::json;

use crate::theme::Tokens;

enum Act {
    Retry,
    Up,
    OpenWithApp(VfsUri),
    ChooseProgram,
}

/// Draw the state for `problem` at `location` (an archive location whose listing failed).
pub fn show(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    location: &VfsUri,
    problem: ArchiveProblem,
    message: &str,
    tokens: &Tokens,
) {
    let container = fm_engine::archive_cmds::container_of(location).map(|r| r.container);
    let name = container
        .as_ref()
        .and_then(|c| c.file_name().map(str::to_string))
        .unwrap_or_else(|| "This archive".to_string());
    let mut act = None;
    ui.add_space(8.0);
    match problem {
        ArchiveProblem::NeedsTool => {
            ui.label(RichText::new("RAR archives need 7-Zip or unrar").strong());
            ui.label(tokens.muted(format!(
                "To open {name}, {}. Then choose Try again, or pick the program in \
                 Preferences › Files if it is not found by itself.",
                fm_engine::archive_cmds::rar_install_hint()
            )));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Try again").clicked() {
                    act = Some(Act::Retry);
                }
                if ui.button("Choose program…").clicked() {
                    act = Some(Act::ChooseProgram);
                }
                if let Some(c) = &container
                    && ui
                        .button("Open with Default App")
                        .on_hover_text("Open the archive with the program the system uses for it")
                        .clicked()
                {
                    act = Some(Act::OpenWithApp(c.clone()));
                }
            });
        }
        ArchiveProblem::TooDeep => {
            ui.label(RichText::new(format!("{name} is nested too deep")).strong());
            ui.label(tokens.muted(format!(
                "Archives inside archives open up to {} levels deep. Copy {name} out to a \
                 folder and open it there.",
                fm_engine::archive_cmds::MAX_NESTING
            )));
            ui.add_space(4.0);
            if ui
                .button("Show the archive file")
                .on_hover_text("Go to the folder holding it, to copy it out")
                .clicked()
            {
                act = Some(Act::Up);
            }
        }
        ArchiveProblem::NotArchive => {
            ui.colored_label(tokens.error, plain(message));
            ui.label(tokens.muted(
                "Its contents do not match its name. It may be damaged, or another kind of file \
                 with an archive's name.",
            ));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Back").clicked() {
                    act = Some(Act::Up);
                }
                if let Some(c) = &container
                    && ui.button("Open with Default App").clicked()
                {
                    act = Some(Act::OpenWithApp(c.clone()));
                }
            });
        }
    }
    if act.is_some() {
        let _ = s.layout.focus_pane(pane);
    }
    match act {
        Some(Act::Retry) => {
            s.dispatch_or_notice("nav.refresh", &json!({"uri": location.to_string()}));
        }
        Some(Act::Up) => {
            s.dispatch_or_notice("nav.up", &serde_json::Value::Null);
        }
        Some(Act::OpenWithApp(c)) => {
            s.dispatch_or_notice("archive.open_app", &json!({"uri": c.to_string()}));
        }
        Some(Act::ChooseProgram) => crate::preferences::open_files(ui.ctx(), s),
        None => {}
    }
}

/// The message without the "backend archive: " prefix errors carry.
fn plain(message: &str) -> &str {
    message
        .split_once("archive: ")
        .map_or(message, |(_, rest)| rest)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_state_renders_in_both_palettes() {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        let location = VfsUri::archive_root("0123456789abcdef").expect("root");
        for tokens in [Tokens::dark(), Tokens::light()] {
            for problem in [
                ArchiveProblem::NeedsTool,
                ArchiveProblem::TooDeep,
                ArchiveProblem::NotArchive,
            ] {
                let ctx = egui::Context::default();
                crate::theme::apply_to(&ctx, &tokens);
                let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show(
                            ui,
                            &mut s,
                            pane,
                            &location,
                            problem,
                            "backend archive: x.zip is not a zip archive",
                            &tokens,
                        );
                    });
                });
                out.textures_delta.clear();
            }
        }
        assert!(s.notice.is_none(), "drawing alone dispatches nothing");
    }

    #[test]
    fn the_error_prefix_is_dropped() {
        assert_eq!(
            plain("backend archive: notes.zip is not a zip archive"),
            "notes.zip is not a zip archive"
        );
        assert_eq!(plain("plain text"), "plain text");
    }
}
