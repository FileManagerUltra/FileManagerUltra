//! Remote-connection widgets: the Connect to Server dialog, the host-key confirmation,
//! the changed-key card, the pane badge and stale banner, and the free-space label.
//! Pure views over `Session::remote`; every action is a command (`remote.*`).

use egui::{Context, Id, Key, RichText, TextEdit, Ui};
use serde_json::{Map, Value, json};

use fm_engine::Session;
use fm_engine::remote::ConnState;
use fm_layout::PaneId;
use fm_vfs::{Scheme, VfsUri};

use crate::dialogs::frame;
use crate::theme::Tokens;

#[derive(Clone)]
struct Form {
    profile: String,
    host: String,
    port: String,
    user: String,
    use_key: bool,
    password: String,
    key_path: String,
    passphrase: String,
    start_dir: String,
    save_as: String,
    /// Location the form was last prefilled from; a new address reseeds it.
    seeded_for: String,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            profile: String::new(),
            host: String::new(),
            port: "22".to_string(),
            user: String::new(),
            use_key: false,
            password: String::new(),
            key_path: String::new(),
            passphrase: String::new(),
            start_dir: String::new(),
            save_as: String::new(),
            seeded_for: String::new(),
        }
    }
}

impl Form {
    fn load_profile(&mut self, p: &fm_engine::prefs::Profile) {
        self.profile = p.name.clone();
        self.host = p.host.clone();
        self.port = p.port.to_string();
        self.user = p.user.clone();
        self.use_key = p.key_path.is_some();
        self.key_path = p.key_path.clone().unwrap_or_default();
        self.start_dir = p.start_dir.clone().unwrap_or_default();
        self.save_as = p.name.clone();
        self.password.clear();
        self.passphrase.clear();
    }

    fn connect_params(&self) -> Result<Value, String> {
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
        }
        if !self.start_dir.trim().is_empty() {
            map.insert("start_dir".into(), json!(self.start_dir.trim()));
        }
        Ok(Value::Object(map))
    }

    /// Secrets never outlive the dialog.
    fn wipe(&mut self) {
        self.password.clear();
        self.passphrase.clear();
    }
}

/// A text field; `true` when Enter was pressed while it had the keyboard.
fn field(ui: &mut Ui, label: &str, text: &mut String, password: bool, width: f32) -> bool {
    let caption = ui.label(label);
    let response = ui
        .add(
            TextEdit::singleline(text)
                .password(password)
                .desired_width(width),
        )
        .labelled_by(caption.id);
    ui.end_row();
    response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
}

fn form_id(pane: PaneId) -> Id {
    Id::NULL.with(("fm-connect-form", pane))
}

/// Drop a pane's connect form (and any typed secret) once it is no longer an SFTP/SMB pane.
pub fn forget_form(ctx: &Context, pane: PaneId) {
    ctx.data_mut(|d| d.remove::<Form>(form_id(pane)));
}

/// Whether an SFTP/SMB pane still has to connect (so it shows the form, not a listing).
pub fn needs_connect(s: &Session, pane: PaneId) -> bool {
    s.layout.state(pane).is_some_and(|st| {
        let scheme = match st.kind {
            fm_layout::PaneKind::Sftp => Scheme::Sftp,
            _ => return false,
        };
        st.location.scheme() != scheme || s.backends.require(&st.location).is_err()
    })
}

/// The inline connect form of an SFTP/SMB pane that has no server yet.
pub fn connect_form(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let ctx = ui.ctx().clone();
    let form_id = form_id(pane);
    let mut form: Form = ctx
        .data(|d| d.get_temp::<Form>(form_id))
        .unwrap_or_default();
    // A restored workspace leaves the pane on its old server: prefill the form from it.
    let here = s
        .layout
        .state(pane)
        .map(|st| st.location.as_str())
        .unwrap_or_default();
    if form.seeded_for != here {
        form.seeded_for = here;
        if let Some(st) = s.layout.state(pane)
            && st.location.scheme() == Scheme::Sftp
        {
            let a = st.location.authority();
            form.host = a.host.clone();
            form.user = a.user.clone().unwrap_or_default();
            if let Some(port) = a.port {
                form.port = port.to_string();
            }
            form.start_dir = st.location.path_str().to_string();
        }
    }
    let mut close = false;
    let mut submit = false;
    let mut enter = false;
    let mut delete_profile = false;
    let mut error: Option<String> = None;
    let profiles = s.prefs.profiles.clone();
    let busy = s.remote.is_busy();
    egui::ScrollArea::vertical()
        .id_salt(("fm-connect-scroll", pane))
        .show(ui, |ui| {
            ui.heading("Connect to Server");
            ui.add_space(tokens.padding);
            if !profiles.is_empty() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_label("Profile")
                        .selected_text(if form.profile.is_empty() {
                            "(none)"
                        } else {
                            form.profile.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for p in &profiles {
                                if ui
                                    .selectable_label(form.profile == p.name, &p.name)
                                    .clicked()
                                {
                                    form.load_profile(p);
                                }
                            }
                        });
                    if !form.profile.is_empty() && ui.small_button("Delete").clicked() {
                        delete_profile = true;
                    }
                });
                ui.add_space(tokens.padding);
            }
            egui::Grid::new("fm-connect-grid")
                .num_columns(2)
                .spacing([tokens.padding, tokens.padding / 2.0])
                .show(ui, |ui| {
                    enter |= field(ui, "Host", &mut form.host, false, 240.0);
                    enter |= field(ui, "Port", &mut form.port, false, 60.0);
                    enter |= field(ui, "User", &mut form.user, false, 240.0);
                    ui.label("Sign in with");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut form.use_key, false, "Password");
                        ui.selectable_value(&mut form.use_key, true, "Key file");
                    });
                    ui.end_row();
                    if form.use_key {
                        enter |= field(ui, "Key file", &mut form.key_path, false, 240.0);
                        enter |= field(ui, "Passphrase", &mut form.passphrase, true, 240.0);
                    } else {
                        enter |= field(ui, "Password", &mut form.password, true, 240.0);
                    }
                    enter |= field(ui, "Start folder", &mut form.start_dir, false, 240.0);
                    enter |= field(ui, "Save as", &mut form.save_as, false, 240.0);
                });
            ui.add_space(tokens.padding);
            ui.label(
                tokens
                    .muted("Passwords and passphrases are kept for this session only, in memory."),
            );
            ui.add_space(tokens.padding);
            if busy {
                ui.label(tokens.muted("Connecting…"));
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Connect"))
                    .clicked()
                    || (enter && !busy)
                {
                    submit = true;
                }
                if ui.button("Back to Files").clicked() {
                    close = true;
                }
            });
        });
    if delete_profile {
        s.dispatch_or_notice("remote.profile.delete", &json!({"name": form.profile}));
        form.profile.clear();
    }
    if submit {
        match form.connect_params() {
            Err(message) => error = Some(message),
            Ok(params) => {
                if !form.save_as.trim().is_empty() {
                    let mut save = params.clone();
                    if let Some(map) = save.as_object_mut() {
                        map.remove("password");
                        map.remove("passphrase");
                        map.insert("name".into(), json!(form.save_as.trim()));
                    }
                    s.dispatch_or_notice("remote.profile.save", &save);
                }
                let _ = s.layout.focus_pane(pane);
                if s.dispatch_or_notice("remote.connect", &params) {
                    form.wipe();
                }
            }
        }
    }
    if let Some(message) = error {
        s.notice = Some(message);
    }
    if close {
        forget_form(&ctx, pane);
        s.dispatch_or_notice("pane.kind.set", &json!({"kind": "files", "pane": pane}));
    } else {
        ctx.data_mut(|d| d.insert_temp(form_id, form));
    }
}

/// Ask whether to trust an unknown host key (trust on first use).
pub fn host_key_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(info) = s.remote.pending_host_key() else {
        return;
    };
    let mut answer = None;
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        answer = Some(false);
    }
    egui::Window::new("Unknown host key")
        .id(Id::NULL.with("fm-hostkey-dialog"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            ui.label(format!(
                "The authenticity of {}:{} can't be established.",
                info.host, info.port
            ));
            ui.add_space(tokens.padding);
            ui.label(tokens.muted(format!("Key type: {}", info.algorithm)));
            ui.label(RichText::new(&info.fingerprint).monospace().strong());
            ui.add_space(tokens.padding);
            ui.label("Compare the fingerprint with one you know to be correct. Trusting it saves the key, and a later change will be refused.");
            ui.add_space(tokens.padding);
            ui.horizontal(|ui| {
                if ui.button("Trust and continue").clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
    if let Some(accept) = answer {
        s.dispatch_or_notice("remote.hostkey.answer", &json!({"accept": accept}));
    }
}

/// Input of the Authentication Required popup, reseeded whenever the prompt changes.
#[derive(Clone, Default)]
struct LoginForm {
    seed: Option<fm_engine::network::LoginPrompt>,
    user: String,
    domain: String,
    password: String,
    remember: bool,
    focus_pending: bool,
    host: String,
    port: String,
    share: String,
    require_encryption: bool,
}

/// The "Authentication Required" popup for a Network-folder host; answers with
/// `network.login` / `network.login.cancel`.
pub fn login_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(prompt) = s.remote.login_prompt.clone() else {
        return;
    };
    let id = Id::NULL.with("fm-login-dialog");
    let mut form: LoginForm = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    if form.seed.as_ref() != Some(&prompt) {
        form = LoginForm {
            seed: Some(prompt.clone()),
            user: prompt.user.clone(),
            domain: prompt.domain.clone(),
            password: String::new(),
            remember: false,
            focus_pending: true,
            ..LoginForm::default()
        };
    }
    let mut connect = false;
    let mut cancel = ctx.input(|i| i.key_pressed(Key::Escape));
    let title = if prompt.manual {
        "Connect to SMB Server"
    } else {
        "Authentication Required"
    };
    egui::Window::new(title)
        .id(id.with("window"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            if !prompt.manual {
                ui.label(format!("Enter your credentials for {}.", prompt.host));
            }
            if let Some(error) = &prompt.error {
                ui.label(RichText::new(error).color(tokens.error));
            }
            ui.add_space(tokens.padding);
            egui::Grid::new(id.with("grid"))
                .num_columns(2)
                .spacing([tokens.padding, tokens.padding / 2.0])
                .show(ui, |ui| {
                    if prompt.manual {
                        let label = ui.label("Host");
                        let host = ui
                            .add(TextEdit::singleline(&mut form.host).desired_width(220.0))
                            .labelled_by(label.id);
                        if form.focus_pending {
                            host.request_focus();
                            form.focus_pending = false;
                        }
                        ui.end_row();
                        let label = ui.label("Port");
                        ui.add(
                            TextEdit::singleline(&mut form.port)
                                .hint_text("445")
                                .desired_width(220.0),
                        )
                        .labelled_by(label.id);
                        ui.end_row();
                        let label = ui.label("Share");
                        ui.add(
                            TextEdit::singleline(&mut form.share)
                                .hint_text("optional")
                                .desired_width(220.0),
                        )
                        .labelled_by(label.id);
                        ui.end_row();
                    }
                    let label = ui.label("Username");
                    let user = ui
                        .add(TextEdit::singleline(&mut form.user).desired_width(220.0))
                        .labelled_by(label.id);
                    if form.focus_pending {
                        user.request_focus();
                        form.focus_pending = false;
                    }
                    ui.end_row();
                    let label = ui.label("Domain");
                    ui.add(TextEdit::singleline(&mut form.domain).desired_width(220.0))
                        .labelled_by(label.id);
                    ui.end_row();
                    let label = ui.label("Password");
                    let pw = ui
                        .add(
                            TextEdit::singleline(&mut form.password)
                                .password(true)
                                .desired_width(220.0),
                        )
                        .labelled_by(label.id);
                    if pw.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        connect = true;
                    }
                    ui.end_row();
                });
            ui.add_space(tokens.padding);
            ui.radio_value(&mut form.remember, false, "Forget password immediately");
            ui.radio_value(&mut form.remember, true, "Remember password until you quit");
            if prompt.manual {
                ui.checkbox(&mut form.require_encryption, "Require encryption");
            }
            ui.add_space(tokens.padding);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                let ready = !form.user.is_empty() && (!prompt.manual || !form.host.is_empty());
                if ui
                    .add_enabled(ready, egui::Button::new("Connect"))
                    .clicked()
                {
                    connect = true;
                }
            });
        });
    if cancel {
        ctx.data_mut(|d| d.remove::<LoginForm>(id));
        s.dispatch_or_notice("network.login.cancel", &Value::Null);
    } else if connect && !form.user.is_empty() && (!prompt.manual || !form.host.is_empty()) {
        let mut params = json!({
            "user": form.user,
            "password": form.password,
            "domain": form.domain,
            "remember": if form.remember { "session" } else { "forget" },
        });
        if prompt.manual
            && let Some(map) = params.as_object_mut()
        {
            map.insert("host".into(), json!(form.host.trim()));
            map.insert("share".into(), json!(form.share.trim()));
            map.insert("require_encryption".into(), json!(form.require_encryption));
            if !form.port.trim().is_empty() {
                // A non-number is passed through as a string so the command rejects it.
                let port = form
                    .port
                    .trim()
                    .parse::<u64>()
                    .map_or(json!(form.port), |p| json!(p));
                map.insert("port".into(), port);
            }
        }
        ctx.data_mut(|d| d.remove::<LoginForm>(id));
        s.dispatch_or_notice("network.login", &params);
    } else {
        ctx.data_mut(|d| d.insert_temp(id, form));
    }
}

/// Short popup for an SFTP host opened from the Network folder.
pub fn sftp_prompt_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(uri) = s.remote.sftp_prompt.clone() else {
        return;
    };
    let mut open = ctx.input(|i| i.key_pressed(Key::Enter));
    let mut cancel = ctx.input(|i| i.key_pressed(Key::Escape));
    egui::Window::new("SFTP Server")
        .id(Id::NULL.with("fm-sftp-prompt"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            ui.label(format!("{} is an SSH (SFTP) server.", uri.authority().host));
            ui.label("Connect to it from an SFTP pane.");
            ui.add_space(tokens.padding);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui.button("Open SFTP Pane").clicked() {
                    open = true;
                }
            });
        });
    if cancel {
        s.dispatch_or_notice("network.sftp.cancel", &Value::Null);
    } else if open {
        s.dispatch_or_notice("network.sftp.open", &Value::Null);
    }
}

/// A card for a refused, changed host key. Stays until dismissed.
pub fn host_key_alert(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(message) = s.remote.host_key_alert.clone() else {
        return;
    };
    let mut dismiss = ctx.input(|i| i.key_pressed(Key::Escape));
    egui::Window::new("Host key changed")
        .id(Id::NULL.with("fm-hostkey-alert"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            ui.label(
                RichText::new("The server's identity changed")
                    .color(tokens.error)
                    .strong(),
            );
            ui.add_space(tokens.padding);
            ui.label(message);
            ui.add_space(tokens.padding);
            ui.label(tokens.muted(
                "Nothing was sent to the server. If you expected this change, forget the \
                 host in Preferences (Trusted hosts) and connect again.",
            ));
            ui.add_space(tokens.padding);
            if ui.button("Close").clicked() {
                dismiss = true;
            }
        });
    if dismiss {
        s.remote.host_key_alert = None;
    }
}

/// The pane's remote authority and its state, if the pane shows a remote location.
fn pane_state(s: &Session, pane: PaneId) -> Option<(String, ConnState)> {
    let loc = &s.layout.state(pane)?.location;
    let state = s.remote.state(loc.scheme(), loc.authority())?.clone();
    Some((loc.authority().to_string(), state))
}

/// `" (encrypted)"` / `" (signed)"` for a ready SMB pane, otherwise empty.
fn smb_security(s: &Session, pane: PaneId) -> &'static str {
    let Some(loc) = s.layout.state(pane).map(|p| &p.location) else {
        return "";
    };
    match s
        .remote
        .connection(loc.scheme(), loc.authority())
        .and_then(|c| c.smb_encrypted())
    {
        Some(true) => " (encrypted)",
        Some(false) => " (signed)",
        None => "",
    }
}

/// Connection badge for a pane header. `focused` headers sit on the accent colour.
pub fn badge(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens, focused: bool) {
    let Some((who, state)) = pane_state(s, pane) else {
        return;
    };
    let security = smb_security(s, pane);
    let (tip, colour) = match &state {
        ConnState::Ready => (
            format!("Connected to {who}{security}. Click to disconnect"),
            tokens.success,
        ),
        ConnState::Connecting => (
            format!("Connecting to {who}. Click to cancel"),
            tokens.warning,
        ),
        ConnState::Failed(why) => (format!("{why}. Click to disconnect"), tokens.error),
    };
    let rich = RichText::new("Disconnect").font(tokens.font());
    let rich = if focused {
        rich.color(tokens.bg).strong()
    } else {
        rich.color(colour)
    };
    let mut disconnect = false;
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.weak_bg_fill = Default::default();
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
        disconnect = ui.button(rich).on_hover_text(tip).clicked();
    });
    if disconnect {
        let _ = s.layout.focus_pane(pane);
        s.dispatch_or_notice("remote.disconnect", &Value::Null);
    }
}

/// Banner under a stale pane's header with a Reconnect action.
pub fn stale_banner(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let Some((who, state)) = pane_state(s, pane) else {
        return;
    };
    let ConnState::Failed(why) = state else {
        return;
    };
    let can_reconnect = s
        .layout
        .state(pane)
        .and_then(|p| {
            s.remote
                .connection(p.location.scheme(), p.location.authority())
        })
        .is_some_and(|c| c.can_reconnect());
    egui::Frame::NONE
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(tokens.border_width, tokens.error))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin::symmetric(tokens.padding as i8, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{who} is offline — showing the last listing"))
                        .color(tokens.error),
                )
                .on_hover_text(why);
                if can_reconnect && ui.button("Reconnect").clicked() {
                    let _ = s.layout.focus_pane(pane);
                    s.dispatch_or_notice("remote.reconnect", &Value::Null);
                }
                if ui.button("Disconnect").clicked() {
                    let _ = s.layout.focus_pane(pane);
                    s.dispatch_or_notice("remote.disconnect", &Value::Null);
                }
            });
        });
}

/// `"12.3 GB free of 500 GB"` for the volume at `uri`, once known. Never blocks.
pub fn free_space_label(s: &mut Session, uri: &VfsUri) -> Option<String> {
    let info = s.free_space(uri)?;
    Some(format!(
        "{} free of {}",
        crate::listing::human_size(info.available),
        crate::listing::human_size(info.total)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_layout::PaneKind;

    #[test]
    fn a_restored_sftp_pane_without_a_connection_shows_the_form() {
        let mut s = Session::default();
        let pane = s.layout.focused();
        let uri = VfsUri::parse("sftp://alice@nas.local/home/alice").expect("uri");
        if let Some(st) = s.layout.state_mut(pane) {
            st.kind = PaneKind::Sftp;
            st.location = uri;
        }
        assert!(needs_connect(&s, pane), "no backend registered yet");
        if let Some(st) = s.layout.state_mut(pane) {
            st.kind = PaneKind::Files;
        }
        assert!(!needs_connect(&s, pane), "only SFTP panes show the form");
    }
}
