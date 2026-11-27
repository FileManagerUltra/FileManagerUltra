//! The password field shown in place of a listing when an encrypted archive needs one
//! (`archive-browsing` "Encrypted archives open with a password"). The typed text lives
//! only in this frame's temporary memory and is cleared as soon as it is submitted.

use egui::Ui;
use fm_engine::Session;
use fm_vfs::VfsUri;

use crate::theme::Tokens;

pub fn prompt(ui: &mut Ui, s: &mut Session, location: &VfsUri, message: &str, tokens: &Tokens) {
    let id = ui.make_persistent_id(("fm-archive-password", location.to_string()));
    let wrong = message.starts_with("wrong password for ");
    let name = fm_engine::archive_cmds::container_of(location)
        .and_then(|r| r.container.file_name().map(str::to_string))
        .unwrap_or_else(|| "This archive".to_string());
    ui.add_space(8.0);
    ui.label(egui::RichText::new(format!("{name} is encrypted")).strong());
    if wrong {
        ui.colored_label(tokens.error, "Wrong password");
    }
    ui.label(
        tokens.muted("Enter its password to open it. It is kept only while the archive is open."),
    );
    if fm_engine::archive_cmds::is_rar_location(location) {
        ui.label(tokens.muted(
            "RAR archives are read by 7-Zip or unrar: the password is handed to that program \
             while it reads, where other programs running as you could see it.",
        ));
    }
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_default();
    let mut submit = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(&mut text)
                .password(true)
                .hint_text("Password")
                .desired_width(220.0),
        );
        field
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::PasswordInput, true, "Password"));
        // Take the keyboard once: when the prompt appears and after a wrong password, and
        // only if no other text field has it. Taking it every frame made two prompts for the same
        // archive (two panes, or a Files and a Preview pane) steal it from each other, so
        // a click or a paste never reached the field the user chose.
        let frame = ui.ctx().cumulative_frame_nr();
        let seen_id = id.with("seen");
        let shown_last_frame = ui
            .data(|d| d.get_temp::<u64>(seen_id))
            .is_some_and(|last| last.saturating_add(1) >= frame);
        ui.data_mut(|d| d.insert_temp(seen_id, frame));
        let again_id = id.with("refocus");
        let again = ui
            .data_mut(|d| d.remove_temp::<bool>(again_id))
            .unwrap_or(false);
        if (!shown_last_frame || again) && (field.has_focus() || !ui.ctx().text_edit_focused()) {
            field.request_focus();
        }
        submit |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        submit |= ui
            .add_enabled(!text.is_empty(), egui::Button::new("Open"))
            .clicked();
    });
    if submit && !text.is_empty() {
        s.dispatch_or_notice(
            "archive.password",
            &serde_json::json!({ "uri": location.to_string(), "password": text }),
        );
        ui.data_mut(|d| {
            d.remove::<String>(id);
            // Back in the field if the password turns out wrong.
            d.insert_temp(id.with("refocus"), true);
        });
        zeroize_string(&mut text);
    } else {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
}

/// Overwrite a string's bytes before it is dropped.
fn zeroize_string(text: &mut String) {
    let len = text.len();
    text.clear();
    text.extend(std::iter::repeat_n('\0', len));
    text.clear();
}
