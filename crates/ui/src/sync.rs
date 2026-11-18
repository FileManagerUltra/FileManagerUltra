//! The Synchronize Folders window: folders and mode on top, a virtualized comparison grid with a
//! per-row action, and a summary bar. Everything is a command (`sync.*`); this module only
//! reads the engine's plan and never touches a backend.

use egui::{Color32, Context, Id, RichText};
use fm_engine::Session;
use fm_engine::item_cmds::display_path;
use fm_engine::sync_hub::{Arming, Phase, RowFilter, RunStats, SyncState, View};
use fm_sync::{Action, Item, NodeKind, Side, Status, SyncMode};
use serde_json::{Value, json};

use crate::listing::{format_mtime, human_size};
use crate::sync_icons::{self, Icon};
use crate::sync_table::table;
use crate::theme::Tokens;

/// Text being edited; sent to the engine only when the field is committed.
#[derive(Clone, Default)]
pub(crate) struct Draft {
    pub profile: String,
    pub name: String,
    pub left: String,
    pub right: String,
    pub exclude: String,
    /// The profile's folders when the inputs were last synced, to spot changes made elsewhere.
    seen: (String, String),
}

fn draft_id() -> Id {
    Id::NULL.with("fm-sync-draft")
}

pub(crate) type Command = (&'static str, Value);

/// A single-line input with the same height and padding everywhere in the sync window, its text
/// centered vertically.
pub(crate) fn input(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .margin(egui::vec2(10.0, 6.0))
        .min_size(egui::vec2(0.0, 32.0))
        .align(egui::Align2::LEFT_CENTER)
}

/// Change the profile (`sync.profile.set`); the engine resets the plan when it matters.
pub(crate) fn set_profile(commands: &mut Vec<Command>, patch: Value) {
    commands.push(("sync.profile.set", json!({ "patch": patch })));
}

/// What an action does, in words (menu entries and tooltips).
fn action_label(action: Action) -> &'static str {
    match action {
        Action::CopyToRight => "Copy to the right",
        Action::CopyToLeft => "Copy to the left",
        Action::DeleteLeft => "Delete on the left",
        Action::DeleteRight => "Delete on the right",
        Action::Skip => "Skip this item",
        Action::None => "Do nothing",
    }
}

/// How the marker column shows an item: its icon, what it means and its colour.
pub(crate) fn look(item: &Item, tokens: &Tokens) -> (Icon, &'static str, Color32) {
    match item.action {
        Action::CopyToRight => (Icon::CopyRight, action_label(item.action), tokens.git_added),
        Action::CopyToLeft => (Icon::CopyLeft, action_label(item.action), tokens.git_added),
        Action::DeleteLeft => (
            Icon::DeleteLeft,
            action_label(item.action),
            tokens.git_removed,
        ),
        Action::DeleteRight => (
            Icon::DeleteRight,
            action_label(item.action),
            tokens.git_removed,
        ),
        Action::Skip => (Icon::Skip, "Skipped", tokens.text_muted),
        Action::None => match item.status {
            Status::Equal => (Icon::Equal, "Identical", tokens.text_muted),
            Status::Conflict => (
                Icon::Conflict,
                "Conflict: choose an action",
                tokens.git_conflict,
            ),
            Status::Unknown => (Icon::Unknown, "Could not be read", tokens.warning),
            _ => (Icon::NoAction, "Nothing to do", tokens.text_muted),
        },
    }
}

/// What each choice in the action menu means, for its tooltip.
pub(crate) fn action_hint(name: &str) -> &'static str {
    match name {
        "copy_to_right" => "Copies this item to the right-hand folder when you start.",
        "copy_to_left" => "Copies this item to the left-hand folder when you start.",
        "delete_left" => "Deletes this item from the left-hand folder when you start.",
        "delete_right" => "Deletes this item from the right-hand folder when you start.",
        "skip" => {
            "Leave this item out of this run. It is still different, so it is counted as \
             skipped and shows up again at the next compare."
        }
        _ => {
            "Nothing needs to happen to this item. Use it for differences you accept as they \
             are; nothing is counted or reported."
        }
    }
}

/// The actions the plan accepts for this item (same rules as `Plan::set_action`), as
/// `(command name, label)`.
pub(crate) fn choices(
    item: &Item,
    mode: SyncMode,
    source: Side,
) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    let mixed = matches!((&item.left, &item.right), (Some(l), Some(r)) if l.kind != r.kind);
    if item.status != Status::Unknown && !mixed {
        let two_way = mode == SyncMode::TwoWay;
        let may_touch = |side: Side| two_way || side != source;
        let may_delete = mode != SyncMode::Update;
        if item.left.is_some() && may_touch(Side::Right) {
            out.push(("copy_to_right", action_label(Action::CopyToRight)));
        }
        if item.right.is_some() && may_touch(Side::Left) {
            out.push(("copy_to_left", action_label(Action::CopyToLeft)));
        }
        if may_delete && item.left.is_some() && may_touch(Side::Left) {
            out.push(("delete_left", action_label(Action::DeleteLeft)));
        }
        if may_delete && item.right.is_some() && may_touch(Side::Right) {
            out.push(("delete_right", action_label(Action::DeleteRight)));
        }
    }
    out.push(("skip", action_label(Action::Skip)));
    out.push(("none", action_label(Action::None)));
    out
}

pub(crate) fn node_text(item: &Item, side: Side) -> (String, String) {
    match item.node(side) {
        None => (String::new(), String::new()),
        Some(n) => {
            let size = match n.kind {
                NodeKind::File => human_size(n.size),
                NodeKind::Dir => "folder".to_string(),
                NodeKind::Symlink => "link".to_string(),
            };
            let when = n.mtime.map(format_mtime).unwrap_or_default();
            (size, when)
        }
    }
}

fn summary_line(state: &SyncState) -> String {
    let Some(plan) = &state.plan else {
        return String::new();
    };
    let s = &plan.summary;
    let mut parts = vec![
        format!("{} to the right", s.copy_right),
        format!("{} to the left", s.copy_left),
        format!("{} deleted on left", s.delete_left),
        format!("{} deleted on right", s.delete_right),
    ];
    if s.conflicts > 0 {
        parts.push(format!("{} conflicts", s.conflicts));
    }
    if s.skipped > 0 {
        parts.push(format!("{} skipped", s.skipped));
    }
    parts.push(format!("{} equal", s.equal));
    parts.push(human_size(s.bytes));
    parts.join("  ·  ")
}

/// "1.2 GiB of 4 GiB · 38 MiB/s · 1m 5s left · 40 s elapsed" for a running sync.
fn run_detail(p: &fm_sync::SyncProgress, stats: Option<RunStats>) -> String {
    let mut parts = Vec::new();
    if p.bytes_total > 0 {
        parts.push(format!(
            "{} of {}",
            human_size(p.bytes_done),
            human_size(p.bytes_total)
        ));
    }
    if let Some(st) = stats {
        let rate = crate::jobs::rate_text(st.bytes_per_sec, st.eta_secs);
        if !rate.is_empty() {
            parts.push(rate);
        }
        parts.push(format!("{} elapsed", duration_text(st.elapsed.as_secs())));
    }
    parts.join("  ·  ")
}

fn duration_text(secs: u64) -> String {
    match secs {
        s if s >= 3600 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
        s if s >= 60 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{s} s"),
    }
}

/// A sync that is working, watching or minimised, as the Activity panel and status bar show it.
pub struct Line {
    pub id: String,
    pub name: String,
    pub text: String,
    /// Speed, elapsed and remaining time of a running sync.
    pub detail: String,
    pub running: bool,
    pub watching: bool,
    /// Compared for "Keep in sync" and waiting for the user's go-ahead.
    pub confirming: bool,
    pub attention: bool,
    pub fraction: Option<f32>,
}

fn pending_changes(state: &SyncState) -> usize {
    state.plan.as_ref().map_or(0, |p| {
        let s = &p.summary;
        s.copy_left + s.copy_right + s.delete_left + s.delete_right
    })
}

fn line_for(id: &str, state: &SyncState) -> Line {
    let text: String;
    let mut detail = String::new();
    let (mut running, mut attention, mut fraction) = (false, false, None);
    match &state.phase {
        Phase::Running(p) => {
            running = true;
            text = format!("Copying {} of {} items", p.items_done, p.items_total);
            detail = run_detail(p, state.run_stats());
            fraction = Some(if p.bytes_total > 0 {
                p.bytes_done as f32 / p.bytes_total as f32
            } else if p.items_total > 0 {
                p.items_done as f32 / p.items_total as f32
            } else {
                0.0
            });
        }
        Phase::Comparing { left, right } => {
            running = true;
            text = format!("Comparing… {left} / {right} items");
        }
        Phase::Failed(e) => {
            attention = true;
            text = e.clone();
        }
        Phase::Idle | Phase::Ready => {
            let conflicts = state.plan.as_ref().map_or(0, |p| p.summary.conflicts);
            let pending = pending_changes(state);
            let failed = state.report.as_ref().map_or(0, |r| r.failed.len());
            if state.arming == Arming::Confirm {
                attention = true;
                text = format!("Confirm to start syncing ({pending} change(s))");
            } else if conflicts > 0 {
                attention = true;
                text = format!("{conflicts} conflict(s) need a decision");
            } else if failed > 0 {
                attention = true;
                text = format!("Last run: {failed} failed");
            } else if state.plan.is_none() {
                text = "Not compared yet".to_string();
            } else if pending > 0 {
                text = format!("{pending} change(s) waiting");
            } else {
                text = "Up to date".to_string();
            }
        }
    }
    Line {
        id: id.to_string(),
        name: state.profile.name.clone(),
        text,
        detail,
        running,
        watching: state.watching(),
        confirming: state.arming == Arming::Confirm,
        attention,
        fraction: fraction.map(|f| f.clamp(0.0, 1.0)),
    }
}

/// Syncs worth showing outside their window: working, keeping in sync, or minimised.
pub fn activity(s: &Session) -> Vec<Line> {
    s.sync
        .states
        .iter()
        .filter(|(id, st)| {
            st.busy()
                || st.watching()
                || st.arming == Arming::Confirm
                || (s.sync.minimized && s.sync.current.as_deref() == Some(id.as_str()))
        })
        .map(|(id, st)| line_for(id, st))
        .collect()
}

/// Render the window while a sync is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    // Decided before the main window draws: a table clicked open mid-frame must not render twice.
    let was_expanded = is_expanded(ctx);
    let Some(state) = s.sync.current_state() else {
        ctx.data_mut(|d| d.remove_temp::<Draft>(draft_id()));
        return;
    };
    // Minimised: the Activity panel and status bar show it; the draft stays as it was.
    if s.sync.minimized {
        return;
    }
    let mut draft = ctx
        .data_mut(|d| d.get_temp::<Draft>(draft_id()))
        .unwrap_or_default();
    if draft.profile != state.profile.id {
        draft = Draft {
            profile: state.profile.id.clone(),
            name: state.profile.name.clone(),
            left: shown(&state.profile.left),
            right: shown(&state.profile.right),
            exclude: state.profile.filters.exclude.join(", "),
            seen: (shown(&state.profile.left), shown(&state.profile.right)),
        };
    }
    // The Browse picker (or a swap) changed the profile: show it in the inputs.
    let (left, right) = (shown(&state.profile.left), shown(&state.profile.right));
    if draft.seen.0 != left {
        draft.left = left.clone();
    }
    if draft.seen.1 != right {
        draft.right = right.clone();
    }
    draft.seen = (left, right);
    if state.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let profiles = s.sync.profiles.clone();
    let mut commands: Vec<Command> = Vec::new();
    let mut open = true;
    let mut close = false;
    let watching = state.watching();
    // Read before drawing: a menu inside the window closes itself on Escape while it is
    // drawn, and would otherwise let the same key close the whole window too.
    let popup_was_open = egui::Popup::is_any_open(ctx);
    crate::dialog_style::window(ctx, tokens, "Synchronize Folders", &mut open)
        .id(Id::NULL.with("fm-sync"))
        .resizable(true)
        .title_bar(false)
        .default_size(egui::vec2(940.0, 560.0))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.spacing_mut().item_spacing.y = 6.0;
            match title_bar(ui, tokens, watching) {
                Some(TitleAction::Minimize) => commands.push(("sync.minimize", Value::Null)),
                Some(TitleAction::Close) => close = true,
                None => {}
            }
            ui.separator();
            workflow_switch(ui, tokens, state, &profiles, &mut commands);
            match state.view {
                View::Wizard => {
                    crate::sync_wizard::show(ui, tokens, state, &mut draft, &mut commands);
                }
                View::Advanced => {
                    crate::sync_advanced::show(ui, tokens, state, &mut draft, &mut commands);
                }
            }
        });
    ctx.data_mut(|d| d.insert_temp(draft_id(), draft));
    // Escape closes only the topmost layer: an open picker, menu or the large table goes first.
    let mut esc = ctx.input(|i| i.key_pressed(egui::Key::Escape))
        && s.picker.is_none()
        && !popup_was_open
        && !egui::Popup::is_any_open(ctx);
    if state.confirm {
        confirm_dialog(ctx, tokens, state, &mut commands);
        if esc {
            commands.push(("sync.confirm.dismiss", Value::Null));
            esc = false;
        }
    }
    if s.sync.close_prompt {
        close_dialog(ctx, tokens, state, &mut commands);
        if esc {
            commands.push(("sync.close.cancel", Value::Null));
            esc = false;
        }
    }
    if is_expanded(ctx) {
        if esc {
            set_expanded(ctx, false);
            esc = false;
        } else if !was_expanded {
            ctx.request_repaint();
        } else if let Some(state) = s.sync.current_state() {
            expanded_window(ctx, tokens, state, &mut commands);
        }
    }
    if !open || close || esc {
        // While keeping in sync, closing only hides the window so the sync keeps running
        // visibly in the Activity panel.
        commands.push((
            if watching {
                "sync.minimize"
            } else {
                "sync.close"
            },
            Value::Null,
        ));
    }
    for (id, params) in commands {
        if let Err(e) = s.dispatch(id, &params) {
            s.notice = Some(e.to_string());
        }
    }
}

/// One saved sync as a card: name, what it does, and which folder goes where.
fn saved_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    p: &fm_sync::Profile,
    here: bool,
) -> egui::Response {
    let height = 96.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    let name = if p.name.trim().is_empty() {
        "Unnamed sync"
    } else {
        p.name.as_str()
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let (fill, stroke) = if response.hovered() {
        (tokens.selection_bg, egui::Stroke::new(1.5, tokens.accent))
    } else {
        (
            tokens.hover_bg,
            egui::Stroke::new(1.0, tokens.text_muted.gamma_multiply(0.5)),
        )
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        tokens.corner_radius,
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    sync_icons::paint(
        painter,
        Icon::mode(p.mode),
        egui::pos2(rect.left() + 28.0, rect.top() + 24.0),
        26.0,
        Icon::mode(p.mode).color(tokens),
    );
    let left = rect.left() + 56.0;
    let width = rect.width() - 70.0;
    let line = |text: String, size: f32, color: Color32, y: f32| {
        let mut job = egui::text::LayoutJob::single_section(
            text,
            egui::TextFormat::simple(egui::FontId::proportional(size), color),
        );
        job.wrap = egui::text::TextWrapping {
            max_width: width,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(left, rect.top() + y), galley, color);
    };
    let mode = match p.mode {
        SyncMode::Update => "Copies new and changed files",
        SyncMode::Mirror => "Makes the target an exact copy",
        SyncMode::TwoWay => "Merges changes both ways",
    };
    let suffix = if here { "  ·  open now" } else { "" };
    line(name.to_string(), tokens.font_size + 2.0, tokens.text, 8.0);
    line(
        format!("{mode}{suffix}"),
        tokens.font_size,
        tokens.accent,
        8.0 + tokens.font_size + 6.0,
    );
    let (from, to) = match (p.mode, p.source) {
        (SyncMode::TwoWay, _) => (&p.left, &p.right),
        (_, Side::Left) => (&p.left, &p.right),
        _ => (&p.right, &p.left),
    };
    let (first, second) = if p.mode == SyncMode::TwoWay {
        ("Left:  ", "Right: ")
    } else {
        ("From:  ", "To:       ")
    };
    for (n, (label, uri)) in [(first, from), (second, to)].into_iter().enumerate() {
        line(
            format!("{label}{}", shown(uri)),
            tokens.font_size - 1.0,
            tokens.text_muted,
            8.0 + (2.0 + n as f32) * (tokens.font_size + 4.0) + 2.0,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Choose between the guided questions and the all-options window.
fn workflow_switch(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    saved: &[fm_sync::Profile],
    commands: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!state.busy(), |ui| {
            let open = ui.add_enabled_ui(!saved.is_empty(), |ui| {
                sync_icons::tile(ui, tokens, Icon::Open, "Open saved", false)
            });
            let open = open.inner.on_hover_text(if saved.is_empty() {
                "No saved syncs yet. Save one on the last step."
            } else {
                "Open a sync you saved earlier"
            });
            egui::Popup::menu(&open).width(560.0).show(|ui| {
                ui.set_width(540.0);
                for (text, size, color) in [
                    ("Your saved syncs", tokens.font_size + 3.0, tokens.text),
                    (
                        "Click one to open it. Nothing is changed by opening.",
                        tokens.font_size,
                        tokens.text_muted,
                    ),
                ] {
                    let galley = ui.painter().layout_no_wrap(
                        text.to_string(),
                        egui::FontId::proportional(size),
                        color,
                    );
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), galley.size().y + 2.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().galley(rect.left_top(), galley, color);
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for p in saved {
                            let here = p.id == state.profile.id;
                            if saved_row(ui, tokens, p, here).clicked() {
                                commands.push(("sync.open", json!({ "id": p.id })));
                                ui.close();
                            }
                        }
                    });
            });
            ui.add_space(6.0);
            for (view, icon, label, mode, tip) in [
                (
                    View::Wizard,
                    Icon::List,
                    "Guided setup",
                    "wizard",
                    "Answer a few questions, review the changes, then start",
                ),
                (
                    View::Advanced,
                    Icon::Sliders,
                    "Advanced",
                    "advanced",
                    "Set every option yourself on one screen",
                ),
            ] {
                if sync_icons::tile(ui, tokens, icon, label, state.view == view)
                    .on_hover_text(tip)
                    .clicked()
                    && state.view != view
                {
                    commands.push(("sync.view", json!({ "mode": mode })));
                }
            }
        });
        let hint = match state.view {
            View::Wizard => "Answer a few questions. Nothing is changed until you press Start.",
            View::Advanced => {
                "All options on one screen. Nothing is changed until you press Synchronize."
            }
        };
        caption(ui, tokens, hint);
    });
    ui.separator();
}

enum TitleAction {
    Minimize,
    Close,
}

fn title_bar(ui: &mut egui::Ui, tokens: &Tokens, watching: bool) -> Option<TitleAction> {
    let mut action = None;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, 24.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if crate::search::title_button(
                ui,
                tokens,
                if watching {
                    "Close window (keeps syncing in the background)"
                } else {
                    "Close window"
                },
                true,
            )
            .clicked()
            {
                action = Some(TitleAction::Close);
            }
            if crate::search::title_button(ui, tokens, "Minimize to the Activity panel", false)
                .clicked()
            {
                action = Some(TitleAction::Minimize);
            }
        },
    );
    let bar = egui::Rect::from_min_size(ui.min_rect().left_top(), egui::vec2(width, 24.0));
    ui.painter().text(
        bar.center(),
        egui::Align2::CENTER_CENTER,
        "Synchronize Folders",
        egui::FontId::proportional(tokens.font_size + 2.0),
        tokens.text,
    );
    action
}

pub(crate) fn caption(ui: &mut egui::Ui, tokens: &Tokens, text: &str) -> egui::Response {
    ui.label(
        RichText::new(text)
            .font(egui::FontId::proportional(tokens.font_size - 1.0))
            .color(tokens.text_muted),
    )
}

/// A stored folder in the notation of the OS (`C:\data`, `/home/me`); remote ones stay uris.
pub(crate) fn shown(stored: &str) -> String {
    fm_sync::profile::parse_endpoint(stored)
        .map(|uri| display_path(&uri))
        .unwrap_or_else(|_| stored.to_string())
}

/// The two folder inputs with a button between them that swaps them (reverses the direction).
pub(crate) fn folder_pair(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    draft: &mut Draft,
    labels: (&str, &str),
    commands: &mut Vec<Command>,
) {
    // Leave room for the swap button and the item spacing, or the window grows every frame.
    let w = ((ui.available_width() - 100.0) / 2.0).max(120.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui(egui::vec2(w, 0.0), |ui| {
            ui.set_max_width(w);
            folder_card(ui, tokens, |ui| {
                folder_field(ui, tokens, labels.0, &mut draft.left, "left", commands);
            });
        });
        ui.vertical(|ui| {
            ui.add_space(18.0);
            let enabled = !state.busy() && !state.watching();
            if ui
                .add_enabled_ui(enabled, |ui| {
                    sync_icons::button(ui, tokens, Icon::Swap, "", false)
                        .on_hover_text("Swap the two folders: reverses the direction")
                        .clicked()
                })
                .inner
            {
                set_profile(
                    commands,
                    json!({ "left": draft.left.trim(), "right": draft.right.trim() }),
                );
                commands.push(("sync.swap", Value::Null));
                std::mem::swap(&mut draft.left, &mut draft.right);
            }
        });
        ui.allocate_ui(egui::vec2(w, 0.0), |ui| {
            ui.set_max_width(w);
            folder_card(ui, tokens, |ui| {
                folder_field(ui, tokens, labels.1, &mut draft.right, "right", commands);
            });
        });
    });
}

/// Outlined card so each folder reads as one clear choice.
fn folder_card(ui: &mut egui::Ui, tokens: &Tokens, contents: impl FnOnce(&mut egui::Ui)) {
    let margin = 12.0;
    let inner = (ui.max_rect().width() - 2.0 * margin - 4.0).max(80.0);
    egui::Frame::new()
        .stroke(egui::Stroke::new(
            1.5,
            tokens.panel.lerp_to_gamma(tokens.text, 0.3),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(inner);
            contents(ui);
        });
}

pub(crate) fn folder_field(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    label: &str,
    text: &mut String,
    field: &'static str,
    commands: &mut Vec<Command>,
) {
    ui.vertical(|ui| {
        let cap = ui
            .horizontal(|ui| {
                sync_icons::icon(ui, tokens, Icon::Folder, 16.0);
                let cap = caption(ui, tokens, label);
                if ui
                    .small_button("Browse…")
                    .on_hover_text("Choose the folder from a list instead of typing it")
                    .clicked()
                {
                    commands.push((
                        "picker.open",
                        json!({
                            "purpose": format!("sync.{field}"),
                            "kind": "folder",
                            "title": format!("Choose the {label} folder"),
                            "start": text.trim(),
                        }),
                    ));
                }
                cap
            })
            .inner;
        let r = ui
            .add(input(text).desired_width(f32::INFINITY))
            .labelled_by(cap.id);
        if r.lost_focus() {
            set_profile(commands, json!({ field: text.trim() }));
        }
    });
}

/// The All / Changes / Conflicts / Equal chips above the table.
pub(crate) struct RowFilterBar;

impl RowFilterBar {
    pub(crate) fn show(
        ui: &mut egui::Ui,
        state: &SyncState,
        commands: &mut Vec<Command>,
        expand: bool,
        tokens: &Tokens,
    ) {
        ui.horizontal(|ui| {
            let counts = row_counts(state);
            for (f, name, show, n) in [
                (RowFilter::All, "All", "all", counts[0]),
                (RowFilter::Changes, "Changes", "changes", counts[1]),
                (RowFilter::Conflicts, "Conflicts", "conflicts", counts[2]),
                (RowFilter::Equal, "Equal", "equal", counts[3]),
            ] {
                if ui
                    .selectable_label(state.filter == f, format!("{name} {n}"))
                    .clicked()
                {
                    commands.push(("sync.filter", json!({ "show": show })));
                }
            }
            if expand {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    expand_button(ui, tokens);
                });
            }
        });
    }
}

pub(crate) fn split_list(text: &str) -> Vec<&str> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect()
}

/// Rows per filter: all, changes, conflicts, equal.
pub(crate) fn row_counts(state: &SyncState) -> [usize; 4] {
    let Some(plan) = &state.plan else {
        return [0; 4];
    };
    let equal = plan.summary.equal;
    let conflicts = plan
        .items
        .iter()
        .filter(|i| matches!(i.status, Status::Conflict | Status::Unknown))
        .count();
    [
        plan.items.len(),
        plan.items.len().saturating_sub(equal),
        conflicts,
        equal,
    ]
}

/// A primary button with a painted check mark (no glyph needed from the font).
pub(crate) fn check_button(ui: &mut egui::Ui, tokens: &Tokens, label: &str) -> egui::Response {
    crate::sync_icons::button(ui, tokens, Icon::Check, label, true)
}

/// The question that must be answered before deletes that cannot be undone (or a mass delete)
/// run. A dialog of its own, above the sync window.
fn confirm_dialog(ctx: &Context, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let Some(plan) = &state.plan else { return };
    let g = &plan.guards;
    let target = match (plan.summary.delete_left, plan.summary.delete_right) {
        (l, 0) if l > 0 => display_path(&plan.left_root),
        (0, r) if r > 0 => display_path(&plan.right_root),
        _ => "both folders".to_string(),
    };
    let (title, how) = if g.permanent {
        (
            "Delete permanently?",
            "permanently. They cannot be restored.",
        )
    } else {
        ("Delete many items?", "(a large share of the folder).")
    };
    let mut open = true;
    crate::dialog_style::sign_in_window(ctx, tokens, title, &mut open)
        .id(Id::NULL.with("fm-sync-confirm"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.set_width(420.0);
            ui.horizontal(|ui| {
                sync_icons::icon(ui, tokens, Icon::Permanent, 32.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} item(s) in {target} will be deleted {how}",
                            g.deletes
                        ))
                        .color(tokens.text),
                    );
                    ui.label(
                        RichText::new("Nothing has been changed yet.").color(tokens.text_muted),
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    commands.push(("sync.confirm.dismiss", Value::Null));
                }
                if sync_icons::button(ui, tokens, Icon::Trash, "Delete and synchronize", true)
                    .clicked()
                {
                    commands.push(("sync.run", json!({ "confirmed": true })));
                }
            });
        });
    if !open {
        commands.push(("sync.confirm.dismiss", Value::Null));
    }
}

/// Closing an unsaved sync: save it, discard it, or keep the window open.
fn close_dialog(ctx: &Context, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let mut open = true;
    crate::dialog_style::sign_in_window(ctx, tokens, "Save this sync?", &mut open)
        .id(Id::NULL.with("fm-sync-close"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.set_width(440.0);
            ui.horizontal(|ui| {
                sync_icons::icon(ui, tokens, Icon::Save, 32.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(
                            "This sync is not saved. Save it to open it again later from \
                             the list of saved syncs.",
                        )
                        .color(tokens.text),
                    );
                    ui.label(
                        RichText::new(
                            "Close without saving forgets its folders, settings and comparison. \
                             No files are changed either way.",
                        )
                        .color(tokens.text_muted),
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    commands.push(("sync.close.cancel", Value::Null));
                }
                if ui.button("Close without saving").clicked() {
                    commands.push(("sync.close", json!({ "discard": true })));
                }
                let valid = state.profile.validate().is_ok();
                ui.add_enabled_ui(valid, |ui| {
                    if sync_icons::button(ui, tokens, Icon::Save, "Save and close", true)
                        .on_disabled_hover_text("Choose two folders first")
                        .clicked()
                    {
                        // A failed save leaves the sync unsaved, so the close asks again.
                        commands.push(("sync.profile.save", Value::Null));
                        commands.push(("sync.close", Value::Null));
                    }
                });
            });
        });
    if !open {
        commands.push(("sync.close.cancel", Value::Null));
    }
}

fn expanded_id() -> Id {
    Id::NULL.with("fm-sync-expanded")
}

/// Height a part (the list or a wizard step's body) of page `page` may take beyond its
/// `designed` height when the user makes the window taller, so the list grows with it.
///
/// Each page records, on the previous frame (`measure_below`), the distance from the end of
/// that part to an anchor row (the Back/Next buttons) and the height from the anchor to the
/// end of the page. The anchor then lands at the window's bottom minus the tallest tail of
/// any page in `group`, so the buttons stay in the same place on every page of a group,
/// and the page never ends below the window (which would push the window larger).
pub(crate) fn spare_height(
    ui: &egui::Ui,
    group: &'static str,
    page: &'static str,
    designed: f32,
) -> f32 {
    let get = |key: &'static str| ui.data(|d| d.get_temp::<f32>(grow_id(group, page, key)));
    let tail = ui.data(|d| d.get_temp::<f32>(grow_id(group, "", "tail-max")));
    let (Some(to_anchor), Some(tail)) = (get("to-anchor"), tail) else {
        return 0.0;
    };
    (ui.available_height() - designed - to_anchor - tail - 1.0).max(0.0)
}

/// Record where page `page`'s anchor row started (`anchor_top`) relative to the end of the
/// growing part (`part_end`, a cursor position), and how much follows the anchor; call it
/// after the page is drawn.
pub(crate) fn measure_below(
    ui: &egui::Ui,
    group: &'static str,
    page: &'static str,
    part_end: f32,
    anchor_top: f32,
) {
    let to_anchor = (anchor_top - part_end).max(0.0);
    // The cursor, not `min_rect`: a resizable window's content rect already spans the
    // whole window. The cursor includes the trailing item spacing, which errs on the safe side.
    let tail = (ui.cursor().top() - anchor_top).max(0.0);
    let ids = (
        grow_id(group, page, "to-anchor"),
        grow_id(group, page, "tail"),
        grow_id(group, "", "tail-max"),
    );
    let changed = ui.data_mut(|d| {
        let old = (d.get_temp::<f32>(ids.0), d.get_temp::<f32>(ids.1));
        d.insert_temp(ids.0, to_anchor);
        d.insert_temp(ids.1, tail);
        let max = d.get_temp::<f32>(ids.2).unwrap_or(0.0).max(tail);
        d.insert_temp(ids.2, max);
        old.0.is_none_or(|v| (v - to_anchor).abs() > 0.25)
            || old.1.is_none_or(|v| (v - tail).abs() > 0.25)
    });
    if changed {
        ui.ctx().request_repaint();
    }
}

fn grow_id(group: &'static str, page: &'static str, key: &'static str) -> Id {
    Id::NULL.with(("fm-sync-grow", group, page, key))
}

pub(crate) fn is_expanded(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(expanded_id()))
        .unwrap_or(false)
}

fn set_expanded(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(expanded_id(), on));
}

/// Square icon button that opens the table in a large window.
pub(crate) fn expand_button(ui: &mut egui::Ui, tokens: &Tokens) {
    if sync_icons::icon_button(ui, tokens, Icon::Expand, "Expand table").clicked() {
        set_expanded(ui.ctx(), true);
    }
}

/// The grid, leaving `reserve` points below it for what the caller draws next; while the large
/// window shows the table, this place says so instead (one table at a time keeps ids unique).
pub(crate) fn grid_reserving(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
    reserve: f32,
    folders_as_header: bool,
) {
    if !is_expanded(ui.ctx()) {
        table(ui, tokens, state, commands, reserve, folders_as_header);
        return;
    }
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            (ui.available_height() - reserve).max(120.0),
        ),
        egui::Sense::hover(),
    );
    ui.painter().rect_stroke(
        rect,
        tokens.corner_radius,
        egui::Stroke::new(1.0, tokens.text_muted.gamma_multiply(0.45)),
        egui::StrokeKind::Inside,
    );
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(12.0)));
    inner.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.label(RichText::new("The list is open in the large window.").color(tokens.text));
        if ui.button("Show it here again").clicked() {
            set_expanded(ui.ctx(), false);
        }
    });
}

/// The large window with the whole table, filters and status.
fn expanded_window(ctx: &Context, tokens: &Tokens, state: &SyncState, commands: &mut Vec<Command>) {
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Changes", &mut open)
        .id(Id::NULL.with("fm-sync-large"))
        .resizable(true)
        .default_size(ctx.content_rect().size() * 0.9)
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.label(RichText::new("All changes").strong().color(tokens.text));
            RowFilterBar::show(ui, state, commands, false, tokens);
            // Keep exactly the room the status line took last frame (it can wrap), so the
            // path details of a clicked row never push the window larger and move it.
            let status_id = Id::NULL.with("fm-sync-large-status");
            let reserve = ui
                .data(|d| d.get_temp::<f32>(status_id))
                .unwrap_or(40.0)
                .max(40.0);
            table(ui, tokens, state, commands, reserve, true);
            let top = ui.cursor().top();
            status_bar(ui, tokens, state, commands);
            let used = (ui.cursor().top() - top).ceil();
            if ui.data(|d| d.get_temp::<f32>(status_id)) != Some(used) {
                ui.data_mut(|d| d.insert_temp(status_id, used));
                ui.ctx().request_repaint();
            }
        });
    if !open {
        set_expanded(ctx, false);
    }
}

pub(crate) fn status_bar(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
) {
    match &state.phase {
        Phase::Running(p) => {
            let fraction = if p.bytes_total > 0 {
                p.bytes_done as f32 / p.bytes_total as f32
            } else if p.items_total > 0 {
                p.items_done as f32 / p.items_total as f32
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                    .text(format!("{} of {} items", p.items_done, p.items_total)),
            );
            ui.label(RichText::new(run_detail(p, state.run_stats())).color(tokens.text_muted));
        }
        Phase::Comparing { left, right } => {
            ui.horizontal(|ui| {
                ui.add(crate::spinner::Spinner::new());
                ui.label(format!("Scanning… {left} / {right}"));
                if ui
                    .button("Cancel scan")
                    .on_hover_text("Stop scanning; nothing is changed")
                    .clicked()
                {
                    commands.push(("sync.cancel", Value::Null));
                }
            });
        }
        _ => {
            let line = summary_line(state);
            if !line.is_empty() {
                ui.label(RichText::new(line).strong().color(tokens.text));
            }
            if let Some(plan) = &state.plan {
                if plan.guards.permanent && plan.guards.deletes > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} permanent delete(s) planned",
                            plan.guards.deletes
                        ))
                        .color(tokens.warning),
                    );
                }
                for w in plan.warnings.iter().take(3) {
                    ui.label(RichText::new(w).color(tokens.warning));
                }
            }
        }
    }
    if let Some(report) = &state.report {
        let mut text = format!(
            "Last run: {} copied, {} deleted, {} failed",
            report.copied,
            report.deleted,
            report.failed.len()
        );
        if !report.attr_warnings.is_empty() {
            text.push_str(&format!(
                ", {} with unpreserved attributes",
                report.attr_warnings.len()
            ));
        }
        let color = if report.failed.is_empty() {
            tokens.success
        } else {
            tokens.error
        };
        ui.label(RichText::new(text).color(color));
        for (path, why) in report.failed.iter().take(5) {
            ui.label(RichText::new(format!("{path}: {why}")).color(tokens.error));
        }
    }
    if let Some(note) = &state.watch_note {
        ui.label(RichText::new(note).color(tokens.warning));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_sync_reports_size_speed_and_time() {
        let p = fm_sync::SyncProgress {
            items_done: 1,
            items_total: 4,
            bytes_done: 1024,
            bytes_total: 4096,
        };
        let stats = RunStats {
            elapsed: std::time::Duration::from_secs(75),
            bytes_per_sec: Some(512.0),
            eta_secs: Some(6),
        };
        let text = run_detail(&p, Some(stats));
        assert!(text.contains("1.0 KiB of 4.0 KiB"), "{text}");
        assert!(
            text.contains("512 B/s") && text.contains("6 s left"),
            "{text}"
        );
        assert!(text.ends_with("1m 15s elapsed"), "{text}");
        assert_eq!(run_detail(&fm_sync::SyncProgress::default(), None), "");
    }
}
