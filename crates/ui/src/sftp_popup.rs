//! The SFTP connect popup: Connect…, a saved server, or a typed `sftp://` address.
//! Answers with `remote.connect` / `remote.connect.cancel`.

use egui::{Context, Id, Key, TextEdit, Ui};
use serde_json::{Map, Value, json};

use fm_engine::Session;
use fm_engine::remote::SftpPrompt;
use fm_vfs::VfsUri;

use crate::dialogs::frame;
use crate::theme::Tokens;

/// Input of the popup, reseeded whenever the prompt changes.
#[derive(Clone, Default)]
struct SftpForm {
    seed: Option<SftpPrompt>,
    host: String,
    port: String,
    user: String,
    use_key: bool,
    password: String,
    key_path: String,
    passphrase: String,
    start_dir: String,
    save_as: String,
    remember: bool,
    focus_pending: bool,
    /// Folder shown by the key-file picker, while it is open.
    picker: Option<VfsUri>,
}

impl SftpForm {
    fn seeded(prompt: &SftpPrompt) -> Self {
        let user = if prompt.user.is_empty() && !prompt.editing {
            std::env::var("USER")
                .ok()
                .or_else(|| std::env::var("USERNAME").ok())
                .unwrap_or_default()
        } else {
            prompt.user.clone()
        };
        Self {
            seed: Some(prompt.clone()),
            host: prompt.host.clone(),
            port: prompt
                .port
                .map_or_else(|| "22".to_string(), |p| p.to_string()),
            user,
            use_key: !prompt.key_path.is_empty(),
            key_path: prompt.key_path.clone(),
            start_dir: prompt.start_dir.clone(),
            save_as: prompt.save_as.clone(),
            focus_pending: true,
            ..Self::default()
        }
    }

    fn params(&self) -> Result<Value, String> {
        let mut map = Map::new();
        map.insert("host".into(), json!(self.host.trim()));
        map.insert("user".into(), json!(self.user.trim()));
        if !self.port.trim().is_empty() {
            let port: u16 = self
                .port
                .trim()
                .parse()
                .map_err(|_| "Port must be a number from 1 to 65535".to_string())?;
            map.insert("port".into(), json!(port));
        }
        if self.use_key {
            map.insert("key_path".into(), json!(self.key_path.trim()));
            if !self.passphrase.is_empty() {
                map.insert("passphrase".into(), json!(self.passphrase));
            }
        } else {
            map.insert("password".into(), json!(self.password));
            map.insert("remember".into(), json!(self.remember));
        }
        if !self.start_dir.trim().is_empty() {
            map.insert("start_dir".into(), json!(self.start_dir.trim()));
        }
        if !self.save_as.trim().is_empty() {
            map.insert("save_as".into(), json!(self.save_as.trim()));
        }
        Ok(Value::Object(map))
    }
}

fn text(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    password: bool,
    width: f32,
) -> egui::Response {
    let caption = ui.label(label);
    let response = ui
        .add(
            TextEdit::singleline(value)
                .password(password)
                .desired_width(width),
        )
        .labelled_by(caption.id);
    ui.end_row();
    response
}

pub fn sftp_prompt_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let id = Id::NULL.with("fm-sftp-prompt");
    let Some(prompt) = s.remote.sftp_prompt.clone() else {
        ctx.data_mut(|d| d.remove::<SftpForm>(id));
        return;
    };
    let mut form: SftpForm = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    // A new error must not wipe what the user typed.
    let seed = SftpPrompt {
        error: None,
        ..prompt.clone()
    };
    if form.seed.as_ref() != Some(&seed) {
        form = SftpForm::seeded(&seed);
    }
    let busy = s.remote.is_connecting();
    let mut submit = false;
    let mut cancel = ctx.input(|i| i.key_pressed(Key::Escape));
    let title = if prompt.editing {
        "Edit Saved Server"
    } else {
        "Connect to Server"
    };
    egui::Window::new(title)
        .id(id.with("window"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            egui::Grid::new(id.with("grid"))
                .num_columns(2)
                .spacing([tokens.padding, tokens.padding / 2.0])
                .show(ui, |ui| {
                    let mut enter = false;
                    let host = text(ui, "Host", &mut form.host, false, 240.0);
                    if form.focus_pending {
                        host.request_focus();
                        form.focus_pending = false;
                    }
                    enter |= host.lost_focus();
                    enter |= text(ui, "Port", &mut form.port, false, 60.0).lost_focus();
                    enter |= text(ui, "User", &mut form.user, false, 240.0).lost_focus();
                    ui.label("Sign in with");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut form.use_key, false, "Password");
                        ui.selectable_value(&mut form.use_key, true, "Key file");
                    });
                    ui.end_row();
                    if form.use_key {
                        let caption = ui.label("Key file");
                        ui.horizontal(|ui| {
                            let r = ui
                                .add(
                                    TextEdit::singleline(&mut form.key_path)
                                        .hint_text("~/.ssh/id_ed25519")
                                        .desired_width(160.0),
                                )
                                .labelled_by(caption.id);
                            enter |= r.lost_focus();
                            if ui.button("Browse…").clicked() {
                                let home = fm_engine::workspace::home_location();
                                form.picker = Some(home.join(".ssh").unwrap_or(home));
                            }
                        });
                        ui.end_row();
                        enter |=
                            text(ui, "Passphrase", &mut form.passphrase, true, 240.0).lost_focus();
                    } else {
                        enter |= text(ui, "Password", &mut form.password, true, 240.0).lost_focus();
                    }
                    enter |=
                        text(ui, "Start folder", &mut form.start_dir, false, 240.0).lost_focus();
                    let caption = ui.label("Save as");
                    ui.add_enabled(
                        !prompt.editing,
                        TextEdit::singleline(&mut form.save_as)
                            .hint_text("optional")
                            .desired_width(240.0),
                    )
                    .labelled_by(caption.id);
                    ui.end_row();
                    submit |= enter && ui.input(|i| i.key_pressed(Key::Enter));
                });
            ui.add_space(tokens.padding);
            if !form.use_key {
                ui.radio_value(&mut form.remember, false, "Forget password immediately");
                ui.radio_value(&mut form.remember, true, "Remember password until you quit");
            }
            ui.label(tokens.muted("Passwords are never written to disk."));
            ui.add_space(tokens.padding);
            if busy {
                ui.label(tokens.muted("Connecting…"));
            } else if let Some(error) = &prompt.error {
                ui.add(egui::Label::new(egui::RichText::new(error).color(tokens.error)).wrap());
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Connect"))
                    .clicked()
                {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if let Some(dir) = form.picker.clone() {
        match key_picker(ctx, s, tokens, &dir) {
            Picked::Stay => {}
            Picked::Go(next) => form.picker = Some(next),
            Picked::File(path) => {
                form.key_path = path;
                form.picker = None;
            }
            Picked::Close => form.picker = None,
        }
        // Escape closes the picker first, not the whole popup.
        cancel = false;
    }
    if cancel {
        ctx.data_mut(|d| d.remove::<SftpForm>(id));
        s.dispatch_or_notice("remote.connect.cancel", &Value::Null);
        return;
    }
    if submit && !busy {
        match form.params() {
            Err(message) => {
                if let Some(prompt) = s.remote.sftp_prompt.as_mut() {
                    prompt.error = Some(message);
                }
            }
            Ok(params) => {
                if let Err(err) = s.dispatch("remote.connect", &params)
                    && let Some(prompt) = s.remote.sftp_prompt.as_mut()
                {
                    prompt.error = Some(err.to_string());
                }
            }
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, form));
}

enum Picked {
    Stay,
    Go(VfsUri),
    File(String),
    Close,
}

/// Local path of a `file://` uri, in the form a user would type.
fn display_path(uri: &VfsUri) -> String {
    let p = uri.path_str();
    let b = p.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[2] == b':' {
        p.get(1..).unwrap_or(p).to_string()
    } else {
        p.to_string()
    }
}

/// A small file chooser over the local folder `dir`, read through the engine's directory cache.
fn key_picker(ctx: &Context, s: &mut Session, tokens: &Tokens, dir: &VfsUri) -> Picked {
    use fm_engine::DirState;
    let mut out = Picked::Stay;
    if let Err(err) = s.ensure_read(dir.clone()) {
        s.notice = Some(err.to_string());
    }
    let state = s.dirs.state(dir);
    // No ~/.ssh: fall back to the home folder.
    if matches!(state, DirState::Failed { .. }) {
        let home = fm_engine::workspace::home_location();
        return if *dir == home {
            Picked::Close
        } else {
            Picked::Go(home)
        };
    }
    let entries = s.dirs.entries(dir);
    egui::Window::new("Choose Key File")
        .id(Id::NULL.with("fm-key-picker"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            ui.label(tokens.muted(display_path(dir)));
            ui.add_space(tokens.padding);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .min_scrolled_width(320.0)
                .show(ui, |ui| {
                    if let Some(parent) = dir.parent()
                        && ui.selectable_label(false, "📁 ..").clicked()
                    {
                        out = Picked::Go(parent);
                    }
                    let mut rows: Vec<_> = entries
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .filter(|e| !e.name.ends_with(".pub"))
                        .collect();
                    rows.sort_by_cached_key(|e| (!e.is_dir_like(), e.name.to_lowercase()));
                    for e in rows {
                        let dir_like = e.is_dir_like();
                        let label = if dir_like {
                            format!("📁 {}", e.name)
                        } else {
                            format!("    {}", e.name)
                        };
                        if ui.selectable_label(false, label).clicked() {
                            out = if dir_like {
                                Picked::Go(e.uri.clone())
                            } else {
                                Picked::File(display_path(&e.uri))
                            };
                        }
                    }
                    if matches!(state, DirState::Loading { .. }) {
                        ui.label(tokens.muted("Loading…"));
                    }
                });
            ui.add_space(tokens.padding);
            if ui.button("Cancel").clicked() {
                out = Picked::Close;
            }
        });
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        out = Picked::Close;
    }
    out
}
