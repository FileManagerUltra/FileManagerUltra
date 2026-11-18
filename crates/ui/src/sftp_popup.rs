//! The SFTP connect popup: Connect…, a saved server, or a typed `sftp://` address.
//! Answers with `remote.connect` / `remote.connect.cancel`.

use egui::{Context, Id, Key, TextEdit, Ui};
use serde_json::{Map, Value, json};

use fm_engine::Session;
use fm_engine::remote::SftpPrompt;

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
    key_chooser_open: bool,
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
        crate::dialog_style::raise_on_open(ctx, id.with("window"), false);
        return;
    };
    crate::dialog_style::raise_on_open(ctx, id.with("window"), true);
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
    let mut browse: Option<String> = None;
    // The shared chooser answers through `Session::picked`.
    if s.picked.as_ref().is_some_and(|(p, _)| p == KEY_PURPOSE)
        && let Some((_, path)) = s.picked.take()
    {
        form.key_path = path;
    }
    let mut cancel = ctx.input(|i| i.key_pressed(Key::Escape));
    let title = if prompt.editing {
        "Edit Saved Server"
    } else {
        "Connect to Server"
    };
    let mut open = true;
    crate::dialog_style::sign_in_window(ctx, tokens, title, &mut open)
        .id(id.with("window"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            crate::dialog_style::body(ui, |ui| {
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
                            ui.radio_value(&mut form.use_key, false, "Password");
                            ui.radio_value(&mut form.use_key, true, "Key file");
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
                                    browse = Some(key_start(&form.key_path));
                                }
                            });
                            ui.end_row();
                            enter |= text(ui, "Passphrase", &mut form.passphrase, true, 240.0)
                                .lost_focus();
                        } else {
                            enter |=
                                text(ui, "Password", &mut form.password, true, 240.0).lost_focus();
                        }
                        enter |= text(ui, "Start folder", &mut form.start_dir, false, 240.0)
                            .lost_focus();
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
                crate::dialog_style::hint(ui, tokens, "Passwords are never written to disk.");
                ui.add_space(tokens.padding);
                if busy {
                    crate::dialog_style::hint(ui, tokens, "Connecting…");
                } else if let Some(error) = &prompt.error {
                    ui.add(egui::Label::new(egui::RichText::new(error).color(tokens.error)).wrap());
                }
            });
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
    if let Some(start) = browse {
        s.dispatch_or_notice(
            "picker.open",
            &json!({"purpose": KEY_PURPOSE, "kind": "file", "title": "Choose Key File", "start": start}),
        );
    }
    // Escape closes the chooser first, not the whole popup.
    let key_chooser_open = s.picker.as_ref().is_some_and(|p| p.purpose == KEY_PURPOSE);
    if key_chooser_open {
        cancel = false;
    }
    // When the key chooser closes, an earlier chooser reappears; this form goes back in front.
    if form.key_chooser_open && !key_chooser_open {
        ctx.move_to_top(egui::LayerId::new(
            egui::Order::Foreground,
            id.with("window"),
        ));
    }
    form.key_chooser_open = key_chooser_open;
    if cancel || !open {
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

const KEY_PURPOSE: &str = "sftp.key";

/// Where the chooser opens: the typed key file (`~` expanded) or `~/.ssh`.
fn key_start(typed: &str) -> String {
    let home = fm_engine::workspace::home_location();
    let typed = typed.trim();
    let ssh = || {
        home.join(".ssh")
            .map(|u| fm_engine::item_cmds::display_path(&u))
            .unwrap_or_default()
    };
    match typed.strip_prefix("~/") {
        Some(rest) if !rest.is_empty() => {
            let base = fm_engine::item_cmds::display_path(&home);
            format!("{}/{rest}", base.trim_end_matches('/'))
        }
        _ if typed.is_empty() || typed.starts_with('~') => ssh(),
        _ => typed.to_string(),
    }
}
