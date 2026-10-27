//! The update CTA inside the Jobs panel; never steals keyboard focus.

use fm_engine::{Params, Session, updates::UpdateStatus};

use crate::theme::Tokens;

pub fn available(session: &Session) -> bool {
    session.updates.visible && matches!(session.updates.status, UpdateStatus::Available { .. })
}

pub fn show(ui: &mut egui::Ui, session: &mut Session, tokens: &Tokens) {
    if !available(session) {
        return;
    }
    let UpdateStatus::Available { version, .. } = &session.updates.status else {
        return;
    };
    ui.label(format!("Version {version} is available."));
    ui.label(tokens.muted("Download the installer from GitHub."));
    ui.label(tokens.muted("Installation is manual; the app will not restart."));
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                session.command_enabled("app.updates.download"),
                egui::Button::new("Download update…"),
            )
            .clicked()
        {
            session.dispatch_or_notice("app.updates.download", &Params::Null);
        }
        if ui.button("Dismiss update").clicked() {
            session.dispatch_or_notice("app.updates.dismiss", &Params::Null);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_does_not_take_focus_or_disable_the_application() {
        let ctx = egui::Context::default();
        let focus = egui::Id::NULL.with("test-file-manager-input");
        let mut session = Session::default();
        let mut value = String::new();
        for frame in 0..3 {
            if frame == 1 {
                session.updates.visible = true;
                session.updates.status = UpdateStatus::Available {
                    version: "0.2.0".into(),
                    url: String::new(),
                };
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.add(egui::TextEdit::singleline(&mut value).id(focus));
                        assert!(response.enabled());
                        if frame == 0 {
                            response.request_focus();
                        }
                    });
                    crate::jobs::show(ctx, &mut session, &Tokens::dark());
                },
            );
            output.textures_delta.clear();
            assert_eq!(ctx.memory(|m| m.focused()), Some(focus));
        }
    }
}
