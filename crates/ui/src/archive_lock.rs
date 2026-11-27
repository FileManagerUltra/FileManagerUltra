//! "Set password…" / "Change password…" for a zip or 7z (add-archive-vfs 5.6): the new
//! password typed twice and, for 7z, whether the file names are hidden too. The change is
//! staged; Save applies it.

use egui::{Context, Id, Key, RichText};
use fm_engine::Session;
use serde_json::json;

use crate::compress_dialog::Hidden;
use crate::theme::Tokens;

#[derive(Clone, Debug, Default)]
struct Form {
    root: String,
    name: String,
    seven: bool,
    change: bool,
    password: Hidden,
    confirm: Hidden,
    hide_names: bool,
    error: Option<String>,
}

fn form_id() -> Id {
    Id::NULL.with("fm-archive-lock")
}

/// Open the dialog for the archive the focused pane shows (File › Archive › Set
/// Password…).
pub fn ask(ctx: &Context, s: &Session) {
    if let Some(mount) = s
        .layout
        .state(s.layout.focused())
        .and_then(|p| s.archives.for_uri(&p.location))
    {
        ask_for(ctx, mount);
    }
}

/// Ask for a new password for the opened archive `mount` (the archive file's context menu).
pub fn ask_for(ctx: &Context, mount: &fm_engine::archive_cmds::LazyArchive) {
    let Some(fs) = mount.opened() else {
        return;
    };
    let info = fs.info();
    let form = Form {
        root: mount.root().to_string(),
        name: mount
            .container()
            .file_name()
            .unwrap_or("the archive")
            .to_string(),
        seven: fs.format().id() == "7z",
        change: info.encrypted || info.names_encrypted,
        hide_names: true,
        ..Form::default()
    };
    ctx.data_mut(|d| d.insert_temp(form_id(), Some(form)));
}

/// Whether the dialog is open (keyboard shortcuts stay with it).
pub fn is_open(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<Option<Form>>(form_id()).flatten().is_some())
}

pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(mut form) = ctx
        .data(|d| d.get_temp::<Option<Form>>(form_id()))
        .flatten()
    else {
        return;
    };
    let mut open = true;
    let mut done = false;
    let title = if form.change {
        "Change password"
    } else {
        "Set password"
    };
    crate::dialog_style::window(ctx, tokens, title, &mut open)
        .id(Id::NULL.with("fm-archive-lock-window"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.set_max_width(420.0);
            ui.label(RichText::new(format!("New password for {}", form.name)).color(tokens.text));
            let first = ui.add(
                egui::TextEdit::singleline(&mut form.password.0)
                    .password(true)
                    .hint_text("Password")
                    .desired_width(240.0),
            );
            first.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::PasswordInput, true, "Password")
            });
            let second = ui.add(
                egui::TextEdit::singleline(&mut form.confirm.0)
                    .password(true)
                    .hint_text("Type it again")
                    .desired_width(240.0),
            );
            second.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::PasswordInput, true, "Type it again")
            });
            if form.seven {
                ui.checkbox(&mut form.hide_names, "Also hide the file names");
            } else {
                crate::dialog_style::hint(
                    ui,
                    tokens,
                    "Every file is encrypted with AES-256; names stay visible.",
                );
            }
            crate::dialog_style::hint(
                ui,
                tokens,
                "The archive is encrypted when you save it; until then Discard undoes this.",
            );
            if let Some(err) = &form.error {
                ui.label(RichText::new(err).color(tokens.error));
            }
            ui.horizontal(|ui| {
                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                let button = if form.change { "Change" } else { "Set" };
                if ui.button(button).clicked() || enter {
                    if form.password.0.is_empty() {
                        form.error = Some("Type a password".into());
                    } else if form.password.0 != form.confirm.0 {
                        form.error = Some("The passwords do not match".into());
                    } else {
                        let r = s.dispatch(
                            "archive.password.set",
                            &json!({
                                "uri": form.root,
                                "password": form.password.0,
                                "encrypt_names": form.hide_names,
                            }),
                        );
                        match r {
                            Ok(()) => done = true,
                            Err(e) => form.error = Some(e.to_string()),
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if !open || done || ctx.input(|i| i.key_pressed(Key::Escape)) {
        ctx.data_mut(|d| d.remove_temp::<Option<Form>>(form_id()));
    } else {
        ctx.data_mut(|d| d.insert_temp(form_id(), Some(form)));
    }
}
