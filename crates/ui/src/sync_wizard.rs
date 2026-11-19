//! The guided sync workflow: one question per screen, then a review that compares and changes
//! nothing until the user presses the start button. Every answer is a `sync.profile.set` patch and
//! every move a `sync.wizard.go`; the window state lives in the engine.

use egui::RichText;
use fm_engine::sync_hub::{Arming, Phase, SyncState, WIZARD_REVIEW};
use fm_sync::{CompareMethod, DeletePolicy, Side, SyncMode};
use serde_json::{Value, json};

use crate::sync::{
    Command, Draft, caption, check_button, folder_pair, grid_reserving, set_profile, status_bar,
};
use crate::sync_icons::{self, Icon};
use crate::theme::Tokens;

const TITLES: [&str; 7] = [
    "Folders",
    "Goal",
    "Deleting",
    "Compare by",
    "Leave out",
    "Keep in sync",
    "Review",
];

fn step_icon(step: usize, state: &SyncState) -> Icon {
    match step {
        0 => Icon::Folder,
        1 => Icon::mode(state.profile.mode),
        2 => Icon::Trash,
        3 => Icon::compare(state.profile.compare),
        4 => Icon::Funnel,
        5 => Icon::Loop,
        _ => Icon::List,
    }
}

/// A choice shown as a card: a title, a sentence on what it means, and whether it is selected.
fn option(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    icon: Icon,
    selected: bool,
    title: &str,
    text: &str,
) -> bool {
    let stroke = if selected {
        egui::Stroke::new(2.0, tokens.accent)
    } else {
        egui::Stroke::new(1.0, tokens.text_muted.gamma_multiply(0.5))
    };
    let fill = if selected { tokens.hover_bg } else { tokens.bg };
    let inner = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(tokens.corner_radius)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                sync_icons::icon(ui, tokens, icon, 24.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong().color(tokens.text));
                    ui.label(RichText::new(text).color(tokens.text_muted));
                });
            });
        });
    let r = ui.interact(
        inner.response.rect,
        inner.response.id.with("pick"),
        egui::Sense::click(),
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    let step = state.step.min(WIZARD_REVIEW);
    steps_bar(ui, tokens, state, step);
    ui.add_space(4.0);
    let title = TITLES.get(usize::from(step)).copied().unwrap_or("");
    header_band(
        ui,
        tokens,
        step_icon(usize::from(step), state),
        &format!("Step {} of {}: {title}", step + 1, TITLES.len()),
        intro(state, step),
    );
    if step == WIZARD_REVIEW {
        review(ui, tokens, state, draft, commands);
        return;
    }
    // A taller window gives every step the same extra room, so Back/Next stay put. Never
    // less than designed: the review page's taller tail is only known once it was shown, and
    // shrinking by it later would move the buttons.
    let body = crate::sync::part_height(ui, "wizard", "step", BODY_HEIGHT, BODY_HEIGHT);
    let body_end = fixed_body(ui, body, |ui| {
        ui.add_enabled_ui(!state.busy(), |ui| match step {
            0 => folders(ui, tokens, state, draft, commands),
            1 => goal(ui, tokens, state, commands),
            2 => deleting(ui, tokens, state, commands),
            3 => compare_by(ui, tokens, state, commands),
            4 => leave_out(ui, tokens, state, commands),
            _ => keep_in_sync(ui, tokens, state, commands),
        });
    });
    let next = TITLES
        .get(usize::from(step) + 1)
        .copied()
        .unwrap_or("Review");
    let anchor = footer(ui, tokens, step, state, next, commands);
    crate::sync::measure_below(ui, "wizard", "step", body_end, anchor);
}

/// Every step has the same body height, so the Back/Next buttons never move.
const BODY_HEIGHT: f32 = 400.0;
/// Review shows the save card below the body, so its body is that much shorter.
const SAVE_CARD_HEIGHT: f32 = 121.0;
/// The comparison result below the list in the review step.
pub(crate) const FOOTER_HEIGHT: f32 = 60.0;

/// Returns where the body ends: its own edge, so the spacing after it counts as page height
/// for [`crate::sync::measure_below`].
fn fixed_body(ui: &mut egui::Ui, height: f32, contents: impl FnOnce(&mut egui::Ui)) -> f32 {
    ui.scope(|ui| {
        egui::ScrollArea::vertical()
            .id_salt("fm-sync-wizard-body")
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .show(ui, contents);
    })
    .response
    .rect
    .bottom()
}

fn steps_bar(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, step: u8) {
    ui.horizontal(|ui| {
        for (i, t) in TITLES.iter().enumerate() {
            let here = usize::from(step) == i;
            let color = if here {
                tokens.accent
            } else if usize::from(step) > i {
                tokens.text
            } else {
                tokens.text_muted
            };
            sync_icons::icon(ui, tokens, step_icon(i, state), 16.0);
            let mut text = RichText::new(format!("{}  {t}", i + 1)).color(color);
            if here {
                text = text.strong();
            }
            ui.label(text);
            if i + 1 < TITLES.len() {
                ui.label(RichText::new("›").color(tokens.text_muted));
            }
        }
    });
}

/// The question each step asks, shown in the header band.
fn intro(state: &SyncState, step: u8) -> &'static str {
    match step {
        0 => {
            "Which two folders do you want to bring together? A folder that does not exist yet \
             is created when you start."
        }
        1 => "What should the sync do?",
        2 if state.profile.mode == SyncMode::Update => {
            "Copying new and changed files never deletes anything, so there is nothing to \
             decide here. Press Next."
        }
        2 => "How should files that have to be removed be deleted?",
        3 => "How should files be judged to be different?",
        4 => {
            "Is there anything the sync should ignore? List names or patterns separated by \
             commas, for example: *.tmp, .git, node_modules. Leave empty to include everything."
        }
        5 => "Should the folders stay in sync after the first run?",
        _ => "Last step: check the folders and the list of changes below, then press Start.",
    }
}

/// A clearly separated band with the step title and its question.
fn header_band(ui: &mut egui::Ui, tokens: &Tokens, icon: Icon, title: &str, text: &str) {
    let margin = 14.0;
    let width = (ui.available_width() - 2.0 * margin - 4.0).max(100.0);
    egui::Frame::new()
        .fill(tokens.panel.lerp_to_gamma(tokens.text, 0.12))
        .stroke(egui::Stroke::new(
            1.0,
            tokens.panel.lerp_to_gamma(tokens.text, 0.3),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(width);
            // Room for a three-line question on every step, so the body never moves.
            let line =
                |size: f32| ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(size)));
            ui.set_min_height(
                line(tokens.font_size + 4.0).max(24.0)
                    + 4.0
                    + 3.0 * line(tokens.font_size + 2.0)
                    + 2.0 * ui.spacing().item_spacing.y,
            );
            ui.horizontal(|ui| {
                sync_icons::icon(ui, tokens, icon, 24.0);
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(tokens.font_size + 4.0)
                        .color(tokens.text),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(text)
                    .size(tokens.font_size + 2.0)
                    .color(tokens.text),
            );
        });
    ui.add_space(20.0);
}

fn folders(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    folder_pair(
        ui,
        tokens,
        state,
        draft,
        ("First folder (left)", "Second folder (right)"),
        commands,
    );
    if let Err(e) = state.profile.validate()
        && !state.profile.left.is_empty()
        && !state.profile.right.is_empty()
    {
        ui.label(RichText::new(e.to_string()).color(tokens.text_muted));
    }
}

fn goal(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let p = &state.profile;
    if option(
        ui,
        tokens,
        Icon::Update,
        p.mode == SyncMode::Update,
        "Copy new and changed files",
        "Files that are new or newer in the source are copied to the other folder. \
         Nothing is ever deleted.",
    ) {
        set_profile(commands, json!({ "mode": "Update" }));
    }
    if option(
        ui,
        tokens,
        Icon::Mirror,
        p.mode == SyncMode::Mirror,
        "Make the target an exact copy",
        "The target ends up identical to the source. Files that exist only in the target \
         are deleted (the next question decides how).",
    ) {
        set_profile(commands, json!({ "mode": "Mirror" }));
    }
    if option(
        ui,
        tokens,
        Icon::TwoWay,
        p.mode == SyncMode::TwoWay,
        "Merge both ways",
        "Changes made in either folder are carried over to the other one. If a file changed \
         on both sides it is listed as a conflict and left alone.",
    ) {
        set_profile(commands, json!({ "mode": "TwoWay" }));
    }
    if p.mode != SyncMode::TwoWay {
        ui.add_space(4.0);
        ui.label(RichText::new("Copy from:").color(tokens.text));
        ui.horizontal(|ui| {
            for (side, label, key) in [
                (Side::Left, "Left folder to right folder", "Left"),
                (Side::Right, "Right folder to left folder", "Right"),
            ] {
                if ui.selectable_label(p.source == side, label).clicked() {
                    set_profile(commands, json!({ "source": key }));
                }
            }
        });
    }
}

fn deleting(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let p = &state.profile;
    if p.mode == SyncMode::Update {
        return;
    }
    if option(
        ui,
        tokens,
        Icon::Trash,
        p.delete.is_none() || p.delete == Some(DeletePolicy::Trash),
        "Move them to the trash (recommended)",
        "You can restore them from the trash. Folders on another computer have no trash \
         and delete permanently.",
    ) {
        set_profile(commands, json!({ "delete": Value::Null }));
    }
    if option(
        ui,
        tokens,
        Icon::Permanent,
        p.delete == Some(DeletePolicy::Permanent),
        "Delete them permanently",
        "They cannot be restored. You are asked to confirm before this happens.",
    ) {
        set_profile(commands, json!({ "delete": "Permanent" }));
    }
}

fn compare_by(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let p = &state.profile;
    for (m, key, title, text) in [
        (
            CompareMethod::SizeMtime,
            "SizeMtime",
            "Size and date (fast, recommended)",
            "A file differs when its size or modification time differs. No file content is read.",
        ),
        (
            CompareMethod::Checksum,
            "Checksum",
            "Also check the content when unsure",
            "When sizes match but dates differ, the content is compared. Slower on large files.",
        ),
        (
            CompareMethod::ChecksumAlways,
            "ChecksumAlways",
            "Always check the content (slow, exact)",
            "Every file of the same size is read and compared. Use when dates cannot be trusted.",
        ),
    ] {
        if option(ui, tokens, Icon::compare(m), p.compare == m, title, text) {
            set_profile(commands, json!({ "compare": key }));
        }
    }
}

fn leave_out(ui: &mut egui::Ui, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    if let Some(list) = crate::preview_filters::pattern_list(
        ui,
        tokens,
        "sync-exclude",
        "Ignore these files and folders",
        &state.profile.filters.exclude,
        5.0,
    ) {
        set_profile(commands, json!({ "filters": { "exclude": list } }));
    }
    crate::dialog_style::hint(
        ui,
        tokens,
        "Patterns match names: * is any text, ? one character. For example *.tmp, .git or node_modules.",
    );
}

fn keep_in_sync(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
) {
    let p = &state.profile;
    if option(
        ui,
        tokens,
        Icon::Check,
        !(p.watch.enabled || state.watching()),
        "No, synchronize once",
        "The changes are applied one time when you press Start. Run it again whenever you like.",
    ) {
        set_profile(
            commands,
            json!({ "watch": { "enabled": false, "poll_secs": p.watch.poll_secs } }),
        );
    }
    if option(
        ui,
        tokens,
        Icon::Loop,
        p.watch.enabled || state.watching(),
        "Yes, keep in sync while the app is open",
        "After the first run, later changes are applied automatically. Conflicts and large \
         deletes still wait for you. You can stop it from the Activity panel.",
    ) {
        set_profile(
            commands,
            json!({ "watch": { "enabled": true, "poll_secs": p.watch.poll_secs } }),
        );
    }
}

/// Back and Next, each with one line saying what it does.
fn footer(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    step: u8,
    state: &SyncState,
    next: &str,
    commands: &mut Vec<Command>,
) -> f32 {
    ui.separator();
    // Top of the Back/Next row: the anchor that stays in place on every step.
    let anchor = ui.cursor().top();
    ui.horizontal(|ui| {
        ui.add_enabled_ui(step > 0 && !state.busy(), |ui| {
            if sync_icons::button(ui, tokens, Icon::Back, "Back", false).clicked() {
                commands.push(("sync.wizard.go", json!({ "step": step.saturating_sub(1) })));
            }
        });
        let ready = state.profile.validate().is_ok();
        let label = if step + 1 == WIZARD_REVIEW {
            "Next: compare and review"
        } else {
            "Next"
        };
        ui.add_enabled_ui(ready && !state.busy(), |ui| {
            let icon = if step + 1 == WIZARD_REVIEW {
                Icon::Compare
            } else {
                Icon::Next
            };
            if sync_icons::button(ui, tokens, icon, label, true).clicked() {
                commands.push(("sync.wizard.go", json!({ "step": step + 1 })));
            }
        });
    });
    let back = if step > 0 {
        "Back: return to the previous question. Your answers are kept."
    } else {
        "Back: this is the first question, so there is nothing to go back to."
    };
    caption(ui, tokens, back);
    let what = if step + 1 == WIZARD_REVIEW {
        "scans both folders and shows what would change. Nothing is changed yet.".to_string()
    } else {
        format!("saves this answer and goes to “{next}”. Nothing is changed yet.")
    };
    caption(ui, tokens, &format!("Next: {what}"));
    anchor
}

/// Name and save the profile: prominent, on the last step.
fn save_card(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    egui::Frame::new()
        .fill(tokens.hover_bg)
        .stroke(egui::Stroke::new(1.0, tokens.accent))
        .corner_radius(tokens.corner_radius)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                sync_icons::save_illustration(ui, tokens, 88.0, state.saved);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Name and save this sync")
                            .strong()
                            .color(tokens.text),
                    );
                    ui.label(
                        RichText::new(
                            "Saved syncs appear under Profiles so you can run them again. \
                             Saving changes nothing in your folders.",
                        )
                        .color(tokens.text_muted),
                    );
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            crate::sync::input(&mut draft.name)
                                .hint_text("Name")
                                .desired_width(280.0),
                        );
                        if r.lost_focus() && draft.name != state.profile.name {
                            commands.push(("sync.profile.rename", json!({ "name": draft.name })));
                        }
                        let label = if state.saved { "Saved" } else { "Save profile" };
                        ui.add_enabled_ui(!state.busy(), |ui| {
                            if sync_icons::button(ui, tokens, Icon::Save, label, !state.saved)
                                .clicked()
                            {
                                if draft.name != state.profile.name {
                                    commands.push((
                                        "sync.profile.rename",
                                        json!({ "name": draft.name }),
                                    ));
                                }
                                commands.push(("sync.profile.save", Value::Null));
                            }
                        });
                    });
                });
            });
        });
}

fn review(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    commands: &mut Vec<Command>,
) {
    let keep = state.profile.watch.enabled;
    let designed = BODY_HEIGHT - SAVE_CARD_HEIGHT - FOOTER_HEIGHT - 6.0;
    // The list takes whatever a taller window adds.
    let body = crate::sync::part_height(ui, "wizard", "review", designed, designed);
    let body_end = fixed_body(ui, body, |ui| {
        grid_reserving(ui, tokens, state, commands, 0.0, true);
    });
    // Outside the scrolling body so the result of the comparison is always in view.
    egui::Frame::new()
        .fill(tokens.hover_bg)
        .corner_radius(tokens.corner_radius)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(FOOTER_HEIGHT - 16.0);
            ui.horizontal_top(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    crate::sync::expand_button(ui, tokens);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        status_bar(ui, tokens, state, commands);
                    });
                });
            });
        });
    ui.separator();
    ui.add_space(8.0);
    save_card(ui, tokens, state, draft, commands);
    ui.add_space(4.0);
    ui.separator();
    let working = state.busy();
    let has_work = state.plan.as_ref().is_some_and(|p| !p.is_empty_work());
    let comparing =
        matches!(state.phase, Phase::Comparing { .. }) || state.arming == Arming::Comparing;
    let anchor = ui.cursor().top();
    ui.horizontal(|ui| {
        if working {
            if ui.button("Cancel").clicked() {
                commands.push(("sync.cancel", Value::Null));
            }
        } else {
            if sync_icons::button(ui, tokens, Icon::Back, "Back", false).clicked() {
                commands.push(("sync.wizard.go", json!({ "step": WIZARD_REVIEW - 1 })));
            }
            if sync_icons::button(ui, tokens, Icon::Compare, "Compare again", false).clicked() {
                commands.push(("sync.wizard.go", json!({ "step": WIZARD_REVIEW })));
            }
            // The action that changes files sits alone in the far corner, away from Next/Back.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.watching() {
                    if check_button(ui, tokens, "Stop syncing").clicked() {
                        commands.push(("sync.watch.stop", Value::Null));
                    }
                } else if keep && !comparing && state.plan.is_some() {
                    // Also after Stop syncing or a switch of view, which end the wait for
                    // Start but keep the list: Start picks it up again.
                    if check_button(ui, tokens, "Start and keep in sync").clicked() {
                        commands.push(("sync.watch.confirm", Value::Null));
                    }
                } else if !keep && has_work && !comparing {
                    if check_button(ui, tokens, "Start: synchronize now").clicked() {
                        commands.push(("sync.run", Value::Null));
                    }
                } else if !comparing && state.plan.is_some() {
                    ui.label(RichText::new("The folders already match.").color(tokens.text_muted));
                }
            });
        }
    });
    if !working {
        let start = if keep {
            "Start and keep in sync: applies the listed changes now, then applies later ones automatically."
        } else {
            "Start: applies the listed changes once."
        };
        for text in [
            start,
            "Back: change an answer. Nothing is changed.",
            "Compare again: scan the folders once more. Nothing is changed.",
        ] {
            caption(ui, tokens, text);
        }
    }
    crate::sync::measure_below(ui, "wizard", "review", body_end, anchor);
}
