//! The advanced sync view: the guided setup's questions (same order, names and icons) as one
//! page of settings, followed by the comparison table. Both views edit the same profile, so a
//! change made in one shows in the other.

use egui::RichText;
use fm_engine::sync_hub::{Arming, SyncState};
use fm_sync::{CompareMethod, DeletePolicy, Side, SyncMode};
use serde_json::{Value, json};

use crate::sync::{
    self, Command, Draft, RowFilterBar, caption, folder_pair, grid_reserving, set_profile,
    split_list, status_bar,
};
use crate::sync_icons::{self, Icon};
use crate::sync_wizard::FOOTER_HEIGHT;
use crate::theme::Tokens;

/// Height of everything below the dialog's top bar, equal to the guided setup's, so switching
/// views never resizes the window.
pub(crate) const PAGE_HEIGHT: f32 = 708.0;
const LABEL_WIDTH: f32 = 150.0;
/// Room for the save row and the Synchronize button under the table.
const BOTTOM_HEIGHT: f32 = 124.0;

pub(crate) fn show(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    let top = ui.cursor().top();
    folder_pair(
        ui,
        tokens,
        state,
        draft,
        ("First folder (left)", "Second folder (right)"),
        commands,
    );
    if state.plan.is_some() && !settings_open(ui, state) {
        summary_card(ui, tokens, state);
    } else {
        settings(ui, tokens, state, draft, commands);
    }
    actions(ui, tokens, state, commands);
    RowFilterBar::show(ui, state, commands, false, tokens);
    let used = ui.cursor().top() - top;
    let designed = (PAGE_HEIGHT - used - FOOTER_HEIGHT - BOTTOM_HEIGHT).max(60.0);
    // The list takes what the window leaves: more in a taller window, less on a small screen.
    let table = sync::part_height(ui, "advanced", "advanced", designed, 60.0);
    let width = ui.available_width();
    let table_end = ui
        .allocate_ui(egui::vec2(width, table), |ui| {
            grid_reserving(ui, tokens, state, commands, 0.0, false);
        })
        .response
        .rect
        .bottom();
    // From the list's own edge, so the spacing after it counts as page height too (measured
    // from the cursor the page ended 6 points below the window, which then grew).
    let below = ui.cursor().top();
    // The same footer as the guided review: the result of the comparison is always in view.
    egui::Frame::new()
        .fill(tokens.hover_bg)
        .corner_radius(tokens.corner_radius)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(FOOTER_HEIGHT - 16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                sync::expand_button(ui, tokens);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    status_bar(ui, tokens, state, commands);
                });
            });
        });
    ui.separator();
    name_row(ui, tokens, state, draft, commands);
    synchronize_row(ui, tokens, state, commands);
    sync::measure_below(ui, "advanced", "advanced", table_end, below);
}

fn name_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        let r = ui
            .add(
                sync::input(&mut draft.name)
                    .hint_text("Name of this sync")
                    .desired_width(260.0),
            )
            .on_hover_text("Profile name");
        if r.lost_focus() && draft.name != state.profile.name {
            commands.push(("sync.profile.rename", json!({ "name": draft.name })));
        }
        ui.add_enabled_ui(!state.busy(), |ui| {
            if sync_icons::button(
                ui,
                tokens,
                Icon::Save,
                if state.saved { "Saved" } else { "Save profile" },
                !state.saved,
            )
            .on_hover_text("Keep this sync in the list of saved syncs")
            .clicked()
            {
                commands.push(("sync.profile.save", Value::Null));
            }
            if ui
                .button("Duplicate")
                .on_hover_text("Make a copy of this sync to change")
                .clicked()
            {
                commands.push(("sync.profile.duplicate", Value::Null));
            }
            if state.saved
                && ui
                    .button("Delete")
                    .on_hover_text("Delete this saved sync and its history")
                    .clicked()
            {
                commands.push(("sync.profile.delete", Value::Null));
            }
        });
    });
}

fn settings_key(state: &SyncState) -> egui::Id {
    egui::Id::NULL.with(("fm-sync-settings-open", &state.profile.id))
}

/// Once there is a list of changes the settings fold into one line, so the list gets the room.
fn settings_open(ui: &egui::Ui, state: &SyncState) -> bool {
    ui.data(|d| d.get_temp::<bool>(settings_key(state)))
        .unwrap_or(false)
}

fn summary_card(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState) {
    let p = &state.profile;
    let goal = match p.mode {
        SyncMode::Update => "Copy new and changed files",
        SyncMode::Mirror => "Make the target an exact copy",
        SyncMode::TwoWay => "Merge both ways",
    };
    let direction = match (p.mode, p.source) {
        (SyncMode::TwoWay, _) => "",
        (_, Side::Left) => " (left to right)",
        (_, Side::Right) => " (right to left)",
    };
    let compare = match p.compare {
        CompareMethod::SizeMtime => "size and date",
        CompareMethod::Checksum => "content when unsure",
        CompareMethod::ChecksumAlways => "always content",
    };
    let deleting = if p.mode == SyncMode::Update {
        String::new()
    } else if p.delete == Some(DeletePolicy::Permanent) {
        " · deletes permanently".to_string()
    } else {
        " · deletes to trash".to_string()
    };
    let left_out = p.filters.exclude.len();
    let text = format!(
        "{goal}{direction}{deleting} · compares {compare} · {left_out} left out · keep in sync {}",
        if state.watching() { "on" } else { "off" }
    );
    egui::Frame::new()
        .stroke(egui::Stroke::new(
            1.5,
            tokens.panel.lerp_to_gamma(tokens.text, 0.3),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width((ui.max_rect().width() - 24.0).max(200.0));
            ui.horizontal(|ui| {
                sync_icons::icon(ui, tokens, Icon::mode(p.mode), 20.0);
                ui.add(egui::Label::new(RichText::new(text).color(tokens.text)).truncate());
            });
        });
}

/// One labelled setting: icon and name on the left, the control and a short hint on the right.
fn setting(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    icon: Icon,
    title: &str,
    hint: &str,
    control: impl FnOnce(&mut egui::Ui, egui::Id),
) {
    ui.horizontal(|ui| {
        let label = ui
            .allocate_ui_with_layout(
                egui::vec2(LABEL_WIDTH, 30.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(LABEL_WIDTH);
                    sync_icons::icon(ui, tokens, icon, 20.0);
                    ui.label(RichText::new(title).strong().color(tokens.text))
                },
            )
            .inner;
        control(ui, label.id);
        ui.add(egui::Label::new(RichText::new(hint).color(tokens.text_muted)).truncate());
    });
}

fn settings(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    let p = &state.profile;
    let margin = 12.0;
    egui::Frame::new()
        .stroke(egui::Stroke::new(
            1.5,
            tokens.panel.lerp_to_gamma(tokens.text, 0.3),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width((ui.max_rect().width() - 2.0 * margin - 4.0).max(200.0));
            ui.add_enabled_ui(!state.busy(), |ui| {
                goal(ui, tokens, state, commands);
                setting(
                    ui,
                    tokens,
                    if p.delete == Some(DeletePolicy::Permanent) {
                        Icon::Permanent
                    } else {
                        Icon::Trash
                    },
                    "Deleting",
                    if p.mode == SyncMode::Update {
                        "Not used: this goal never deletes."
                    } else {
                        "What happens to files that have to be removed."
                    },
                    |ui, label| {
                        ui.add_enabled_ui(p.mode != SyncMode::Update, |ui| {
                            deleting(ui, state, label, commands);
                        });
                    },
                );
                setting(
                    ui,
                    tokens,
                    Icon::compare(p.compare),
                    "Compare by",
                    "How files are judged to be different.",
                    |ui, label| compare_by(ui, state, label, commands),
                );
                setting(
                    ui,
                    tokens,
                    Icon::Funnel,
                    "Leave out",
                    "Names or patterns, separated by commas.",
                    |ui, label| leave_out(ui, state, draft, label, commands),
                );
                keep_in_sync(ui, tokens, state, commands);
            });
        });
}

fn goal(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let p = &state.profile;
    let name = |m: SyncMode| match m {
        SyncMode::Update => "Copy new and changed files",
        SyncMode::Mirror => "Make the target an exact copy",
        SyncMode::TwoWay => "Merge both ways",
    };
    setting(ui, tokens, Icon::mode(p.mode), "Goal", "", |ui, label| {
        egui::ComboBox::from_id_salt("sync-mode")
            .selected_text(name(p.mode))
            .width(270.0)
            .show_ui(ui, |ui| {
                for (m, key) in [
                    (SyncMode::Update, "Update"),
                    (SyncMode::Mirror, "Mirror"),
                    (SyncMode::TwoWay, "TwoWay"),
                ] {
                    if ui.selectable_label(p.mode == m, name(m)).clicked() {
                        set_profile(commands, json!({ "mode": key }));
                    }
                }
            })
            .response
            .labelled_by(label)
            .on_hover_text("How the two folders are brought together");
        if p.mode != SyncMode::TwoWay {
            let (text, next) = match p.source {
                Side::Left => ("Left folder to right folder", "Right"),
                Side::Right => ("Right folder to left folder", "Left"),
            };
            if ui
                .button(text)
                .on_hover_text("Direction: click to copy the other way round")
                .clicked()
            {
                set_profile(commands, json!({ "source": next }));
            }
        }
    });
}

fn deleting(ui: &mut egui::Ui, state: &SyncState, label: egui::Id, commands: &mut Vec<Command>) {
    let permanent = state.profile.delete == Some(DeletePolicy::Permanent);
    egui::ComboBox::from_id_salt("sync-delete")
        .selected_text(if permanent {
            "Delete permanently"
        } else {
            "Move to trash"
        })
        .width(270.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(!permanent, "Move to trash").clicked() {
                set_profile(commands, json!({ "delete": Value::Null }));
            }
            if ui
                .selectable_label(permanent, "Delete permanently")
                .clicked()
            {
                set_profile(commands, json!({ "delete": "Permanent" }));
            }
        })
        .response
        .labelled_by(label);
}

fn compare_by(ui: &mut egui::Ui, state: &SyncState, label: egui::Id, commands: &mut Vec<Command>) {
    let p = &state.profile;
    let name = |m: CompareMethod| match m {
        CompareMethod::SizeMtime => "Size and date",
        CompareMethod::Checksum => "Content when unsure",
        CompareMethod::ChecksumAlways => "Always content",
    };
    egui::ComboBox::from_id_salt("sync-compare")
        .selected_text(name(p.compare))
        .width(270.0)
        .show_ui(ui, |ui| {
            for (m, key) in [
                (CompareMethod::SizeMtime, "SizeMtime"),
                (CompareMethod::Checksum, "Checksum"),
                (CompareMethod::ChecksumAlways, "ChecksumAlways"),
            ] {
                if ui.selectable_label(p.compare == m, name(m)).clicked() {
                    set_profile(commands, json!({ "compare": key }));
                }
            }
        })
        .response
        .labelled_by(label);
}

fn leave_out(
    ui: &mut egui::Ui,
    state: &SyncState,
    draft: &mut Draft,
    label: egui::Id,
    commands: &mut Vec<Command>,
) {
    let focus = egui::Id::NULL.with("fm-sync-exclude");
    if !ui.memory(|m| m.has_focus(focus)) {
        draft.exclude = state.profile.filters.exclude.join(", ");
    }
    let r = ui
        .add(
            sync::input(&mut draft.exclude)
                .id(focus)
                .hint_text("Ignore: *.tmp, .git")
                .desired_width(270.0),
        )
        .labelled_by(label);
    if r.lost_focus() {
        set_profile(
            commands,
            json!({ "filters": { "exclude": split_list(&draft.exclude) } }),
        );
    }
}

fn keep_in_sync(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
) {
    let p = &state.profile;
    setting(
        ui,
        tokens,
        Icon::Loop,
        "Keep in sync",
        "Compare after changes and apply them while the app is open.",
        |ui, _| {
            let mut watching = state.watching() || state.arming != Arming::Off;
            if ui
                .checkbox(&mut watching, "Keep in sync")
                .on_hover_text("Conflicts, permanent deletes and mass deletes wait for your review")
                .changed()
            {
                set_profile(
                    commands,
                    json!({ "watch": {
                        "enabled": watching, "poll_secs": p.watch.poll_secs } }),
                );
                commands.push((
                    if watching {
                        "sync.watch.start"
                    } else {
                        "sync.watch.stop"
                    },
                    Value::Null,
                ));
            }
        },
    );
}

/// Compare on the left; the button that changes files alone in the far corner.
fn actions(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    ui.horizontal(|ui| {
        if state.busy() {
            if ui.button("Cancel").clicked() {
                commands.push(("sync.cancel", Value::Null));
            }
            return;
        }
        if sync_icons::button(ui, tokens, Icon::Compare, "Compare", false)
            .on_hover_text("Scan both folders and list the differences; changes nothing")
            .clicked()
        {
            ui.data_mut(|d| d.insert_temp(settings_key(state), false));
            commands.push(("sync.compare", Value::Null));
        }
        if state.plan.is_some() {
            let open = settings_open(ui, state);
            if sync_icons::button(
                ui,
                tokens,
                Icon::Sliders,
                if open {
                    "Hide settings"
                } else {
                    "Change settings"
                },
                false,
            )
            .clicked()
            {
                ui.data_mut(|d| d.insert_temp(settings_key(state), !open));
            }
        }
    });
    caption(
        ui,
        tokens,
        "Compare: list the differences, changes nothing.",
    );
}

/// The step that changes files comes last, alone in the far corner.
fn synchronize_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
) {
    let keep = state.watching() || state.arming != Arming::Off;
    // Exactly one button tall: a bare `with_layout` takes the rest of the window, centres the
    // button in it and counts as page height, so the list shrank a little every frame.
    let size = egui::vec2(
        ui.available_width(),
        sync_icons::button_height(ui, tokens, true),
    );
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let has_work = state.plan.as_ref().is_some_and(|pl| !pl.is_empty_work());
            if state.watching() {
                if sync::check_button(ui, tokens, "Stop syncing").clicked() {
                    commands.push(("sync.watch.stop", Value::Null));
                }
            } else if keep {
                let ready = state.arming == Arming::Confirm;
                ui.add_enabled_ui(ready, |ui| {
                    if sync::check_button(ui, tokens, "Start syncing")
                        .on_hover_text(
                            "Apply the listed changes now, then apply later ones automatically",
                        )
                        .clicked()
                    {
                        commands.push(("sync.watch.confirm", Value::Null));
                    }
                });
            } else {
                ui.add_enabled_ui(!state.busy() && has_work, |ui| {
                    if sync::check_button(ui, tokens, "Synchronize now")
                        .on_hover_text("Carry out the listed actions")
                        .clicked()
                    {
                        commands.push(("sync.run", Value::Null));
                    }
                });
            }
            if keep {
                if ui
                    .button("Minimize to Activity panel")
                    .on_hover_text(
                        "Keep syncing in the background; progress shows in the Activity panel",
                    )
                    .clicked()
                {
                    commands.push(("sync.minimize", Value::Null));
                }
                if state.arming == Arming::Confirm && ui.button("Cancel").clicked() {
                    commands.push(("sync.watch.cancel", Value::Null));
                }
            }
            // The explanation shares the row, so the page height never changes.
            let text = if keep {
                if state.arming == Arming::Confirm {
                    "Applies the listed changes now, then later ones automatically."
                } else {
                    ""
                }
            } else {
                "Synchronize: carry out the listed actions."
            };
            if !text.is_empty() {
                ui.add(egui::Label::new(RichText::new(text).color(tokens.text_muted)).truncate());
            }
        },
    );
}
