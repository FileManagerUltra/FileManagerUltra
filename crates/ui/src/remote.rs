//! Remote-connection widgets: the Connect to Server dialog, the host-key confirmation,
//! the changed-key card, the pane badge and stale banner, and the free-space label.
//! Pure views over `Session::remote`; every action is a command (`remote.*`).

use egui::{Context, Id, Key, RichText, TextEdit, Ui};
use serde_json::{Value, json};

use fm_engine::Session;
use fm_engine::remote::ConnState;
use fm_layout::PaneId;
use fm_vfs::VfsUri;

use crate::theme::Tokens;

/// Ask whether to trust an unknown host key (trust on first use).
pub fn host_key_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(info) = s.remote.pending_host_key() else {
        return;
    };
    let mut answer = None;
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        answer = Some(false);
    }
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Unknown host key", &mut open)
        .id(Id::NULL.with("fm-hostkey-dialog"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.label(format!(
                "The authenticity of {}:{} can't be established.",
                info.host, info.port
            ));
            ui.add_space(tokens.padding);
            crate::dialog_style::hint(ui, tokens, format!("Key type: {}", info.algorithm));
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
    if !open {
        answer = Some(false);
    }
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
    profile: String,
    save_as: String,
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
            require_encryption: s.prefs.smb_require_encryption,
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
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, title, &mut open)
        .id(id.with("window"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            crate::dialog_style::body(ui, |ui| {
                if !prompt.manual {
                    ui.label(format!("Enter your credentials for {}.", prompt.host));
                }
                if let Some(error) = &prompt.error {
                    ui.label(RichText::new(error).color(tokens.error));
                }
                ui.add_space(tokens.padding);
                let smb_profiles: Vec<_> = s
                    .prefs
                    .profiles
                    .iter()
                    .filter(|p| p.protocol == fm_engine::prefs::Protocol::Smb)
                    .collect();
                if prompt.manual && !smb_profiles.is_empty() {
                    egui::ComboBox::from_label("Profile")
                        .selected_text(if form.profile.is_empty() {
                            "(none)"
                        } else {
                            form.profile.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for p in smb_profiles {
                                if ui
                                    .selectable_label(form.profile == p.name, &p.name)
                                    .clicked()
                                {
                                    form.profile = p.name.clone();
                                    form.save_as = p.name.clone();
                                    form.host = p.host.clone();
                                    form.port = p.port.to_string();
                                    form.share = p.share.clone().unwrap_or_default();
                                    form.user = p.user.clone();
                                    form.domain = p.domain.clone().unwrap_or_default();
                                    form.password.clear();
                                }
                            }
                        });
                    ui.add_space(tokens.padding);
                }
                egui::Grid::new(id.with("grid"))
                    .num_columns(2)
                    .spacing([tokens.padding, tokens.padding / 2.0])
                    .show(ui, |ui| {
                        if prompt.manual {
                            let label = ui.label("Host");
                            let host = ui
                                .add(
                                    TextEdit::singleline(&mut form.host)
                                        .hint_text(r"server or \\server")
                                        .desired_width(220.0),
                                )
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
                    ui.horizontal(|ui| {
                        let label = ui.label("Save as profile");
                        ui.add(
                            TextEdit::singleline(&mut form.save_as)
                                .hint_text("optional")
                                .desired_width(160.0),
                        )
                        .labelled_by(label.id);
                    });
                }
            });
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
    if cancel || !open {
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
            if !form.save_as.trim().is_empty() {
                let mut save = json!({
                    "protocol": "smb",
                    "name": form.save_as.trim(),
                    "host": form.host.trim(),
                    "user": form.user,
                    "domain": form.domain,
                    "share": form.share.trim(),
                });
                if let (Some(m), Ok(p)) = (save.as_object_mut(), form.port.trim().parse::<u64>()) {
                    m.insert("port".into(), json!(p));
                }
                s.dispatch_or_notice("remote.profile.save", &save);
            }
        }
        ctx.data_mut(|d| d.remove::<LoginForm>(id));
        s.dispatch_or_notice("network.login", &params);
    } else {
        ctx.data_mut(|d| d.insert_temp(id, form));
    }
}

/// A card for a refused, changed host key. Stays until dismissed.
pub fn host_key_alert(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(message) = s.remote.host_key_alert.clone() else {
        return;
    };
    let mut dismiss = ctx.input(|i| i.key_pressed(Key::Escape));
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Host key changed", &mut open)
        .id(Id::NULL.with("fm-hostkey-alert"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.label(
                RichText::new("The server's identity changed")
                    .color(tokens.error)
                    .strong(),
            );
            ui.add_space(tokens.padding);
            ui.label(message);
            ui.add_space(tokens.padding);
            crate::dialog_style::hint(
                ui,
                tokens,
                "Nothing was sent to the server. If you expected this change, forget the \
                 host in Preferences (Trusted hosts) and connect again.",
            );
            ui.add_space(tokens.padding);
            if ui.button("Close").clicked() {
                dismiss = true;
            }
        });
    if dismiss || !open {
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
    let rich = RichText::new("Disconnect")
        .font(tokens.font())
        .color(colour);
    let rich = if focused { rich.strong() } else { rich };
    let mut disconnect = false;
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.weak_bg_fill = Default::default();
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, colour);
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
    fn a_restored_sftp_pane_without_a_connection_shows_saved_servers() {
        let mut s = Session::default();
        let pane = s.layout.focused();
        let uri = VfsUri::parse("sftp://alice@nas.local/home/alice").expect("uri");
        if let Some(st) = s.layout.state_mut(pane) {
            st.kind = PaneKind::Sftp;
            st.location = uri;
        }
        s.poll();
        let st = s.layout.state(pane).expect("pane");
        assert_eq!(st.location, VfsUri::servers());
        assert_eq!(st.tree.root, VfsUri::servers());
    }
}
