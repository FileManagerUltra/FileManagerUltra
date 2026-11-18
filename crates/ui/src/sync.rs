//! The Synchronize Folders window: folders and mode on top, a virtualized comparison grid with a
//! per-row action, and a summary bar. Everything is a command (`sync.*`); this module only
//! reads the engine's plan and never touches a backend.

use egui::{Color32, Context, Id, RichText};
use fm_engine::Session;
use fm_engine::item_cmds::display_path;
use fm_engine::sync_hub::{Arming, Phase, RowFilter, SortKey, SyncState, View};
use fm_sync::{Action, Item, NodeKind, Side, Status, SyncMode};
use fm_vfs::VfsUri;
use serde_json::{Value, json};

use crate::listing::{format_mtime, human_size};
use crate::sync_icons::{self, Icon};
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

/// What the marker column shows for an item: a short symbol, its meaning and its colour.
pub(crate) fn marker(item: &Item, tokens: &Tokens) -> (&'static str, &'static str, Color32) {
    match item.action {
        Action::CopyToRight => ("->", "Copy to the right", tokens.git_added),
        Action::CopyToLeft => ("<-", "Copy to the left", tokens.git_added),
        Action::DeleteLeft => ("del left", "Delete on the left", tokens.git_removed),
        Action::DeleteRight => ("del right", "Delete on the right", tokens.git_removed),
        Action::Skip => ("skip", "Skipped", tokens.text_muted),
        Action::None => match item.status {
            Status::Equal => ("=", "Identical", tokens.text_muted),
            Status::Conflict => ("!=", "Conflict: choose an action", tokens.git_conflict),
            Status::Unknown => ("?", "Could not be read", tokens.warning),
            _ => ("·", "Nothing to do", tokens.text_muted),
        },
    }
}

/// What each choice in the action menu means, for its tooltip.
fn action_hint(name: &str) -> &'static str {
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

/// The icon that stands for what will happen to the item.
pub(crate) fn marker_icon(item: &Item) -> Icon {
    match item.action {
        Action::CopyToRight => Icon::CopyRight,
        Action::CopyToLeft => Icon::CopyLeft,
        Action::DeleteLeft => Icon::DeleteLeft,
        Action::DeleteRight => Icon::DeleteRight,
        Action::Skip => Icon::Skip,
        Action::None => match item.status {
            Status::Equal => Icon::Equal,
            Status::Conflict => Icon::Conflict,
            Status::Unknown => Icon::Unknown,
            _ => Icon::NoAction,
        },
    }
}

/// The actions the plan accepts for this item (same rules as `Plan::set_action`), as
/// `(command name, label)`.
fn choices(item: &Item, mode: SyncMode, source: Side) -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    let mixed = matches!((&item.left, &item.right), (Some(l), Some(r)) if l.kind != r.kind);
    if item.status != Status::Unknown && !mixed {
        let two_way = mode == SyncMode::TwoWay;
        let may_touch = |side: Side| two_way || side != source;
        let may_delete = mode != SyncMode::Update;
        if item.left.is_some() && may_touch(Side::Right) {
            out.push(("copy_to_right", "Copy to the right"));
        }
        if item.right.is_some() && may_touch(Side::Left) {
            out.push(("copy_to_left", "Copy to the left"));
        }
        if may_delete && item.left.is_some() && may_touch(Side::Left) {
            out.push(("delete_left", "Delete on the left"));
        }
        if may_delete && item.right.is_some() && may_touch(Side::Right) {
            out.push(("delete_right", "Delete on the right"));
        }
    }
    out.push(("skip", "Skip this item"));
    out.push(("none", "Do nothing"));
    out
}

fn node_text(item: &Item, side: Side) -> (String, String) {
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

/// A sync that is working, watching or minimised, as the Activity panel and status bar show it.
pub struct Line {
    pub id: String,
    pub name: String,
    pub text: String,
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
    let (mut running, mut attention, mut fraction) = (false, false, None);
    match &state.phase {
        Phase::Running(p) => {
            running = true;
            text = format!("Copying {} of {} items", p.items_done, p.items_total);
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
        && !egui::Popup::is_any_open(ctx);
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
    let shown = |uri: &str| {
        fm_vfs::VfsUri::parse(uri)
            .map(|u| display_path(&u))
            .unwrap_or_else(|_| uri.to_string())
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
                commands.push((
                    "sync.profile.set",
                    json!({ "patch": { "left": draft.left.trim(), "right": draft.right.trim() } }),
                ));
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
            .add(
                egui::TextEdit::singleline(text)
                    .margin(egui::vec2(8.0, 6.0))
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(cap.id);
        if r.lost_focus() {
            commands.push((
                "sync.profile.set",
                json!({ "patch": { field: text.trim() } }),
            ));
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

pub(crate) fn confirmation(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
) {
    let Some(plan) = &state.plan else { return };
    let g = &plan.guards;
    let target = match (plan.summary.delete_left, plan.summary.delete_right) {
        (l, 0) if l > 0 => display_path(&plan.left_root),
        (0, r) if r > 0 => display_path(&plan.right_root),
        _ => "both folders".to_string(),
    };
    let how = if g.permanent {
        "permanently (they cannot be restored)"
    } else {
        "(a large share of the folder)"
    };
    egui::Frame::new()
        .fill(tokens.diff_del_bg)
        .corner_radius(tokens.corner_radius)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!(
                    "{} item(s) in {target} will be deleted {how}.",
                    g.deletes
                ))
                .color(tokens.text),
            );
            ui.horizontal(|ui| {
                if ui.button("Delete and synchronize").clicked() {
                    commands.push(("sync.run", json!({ "confirmed": true })));
                }
                if ui.button("Cancel").clicked() {
                    commands.push(("sync.confirm.dismiss", Value::Null));
                }
            });
        });
}

fn expanded_id() -> Id {
    Id::NULL.with("fm-sync-expanded")
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
            table(ui, tokens, state, commands, 40.0, true);
            status_bar(ui, tokens, state, commands);
        });
    if !open {
        set_expanded(ctx, false);
    }
}

/// The grid, leaving `reserve` points below it for what the caller draws next. With
/// `folders_as_header` the two side headers name the folders (and source/target) being compared.
fn table(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
    reserve: f32,
    folders_as_header: bool,
) {
    let row_h = tokens.row_height;
    let Some(plan) = &state.plan else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let msg = match &state.phase {
                Phase::Comparing { left, right } => {
                    format!("Scanning… {left} items on the left, {right} on the right")
                }
                Phase::Failed(e) => e.clone(),
                _ => "Choose two folders and press Compare.".to_string(),
            };
            ui.label(RichText::new(msg).color(tokens.text_muted));
        });
        ui.add_space(24.0);
        return;
    };
    let (mode, source) = (state.profile.mode, state.profile.source);
    let width = ui.available_width();
    let side = ((width - 90.0) / 2.0).max(120.0);
    let (name_w, size_w, time_w) = (side * 0.5, side * 0.2, side * 0.3);
    let marker_w = 90.0;
    let tree = state.sort.is_tree();
    let (bar, _) = ui.allocate_exact_size(egui::vec2(width, row_h + 2.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(bar, tokens.corner_radius, tokens.hover_bg);
    let mut x = bar.left();
    let side_title = |left: bool| -> String {
        if !folders_as_header {
            return if left { "Left" } else { "Right" }.to_string();
        }
        let root = if left {
            &plan.left_root
        } else {
            &plan.right_root
        };
        let role = match (mode, source == Side::Left) {
            (SyncMode::TwoWay, _) => "",
            (_, from_left) if from_left == left => "Source: ",
            _ => "Target: ",
        };
        let path = display_path(root);
        let room = ((side - 24.0) / (tokens.font_size * 0.55)) as usize;
        let room = room.saturating_sub(role.len()).max(8);
        let count = path.chars().count();
        let tail: String = if count > room {
            let keep: String = path.chars().skip(count - (room - 1)).collect();
            format!("…{keep}")
        } else {
            path
        };
        format!("{role}{tail}")
    };
    let columns = [
        (side_title(true), name_w, Some(SortKey::Name), false),
        ("Size".to_string(), size_w, Some(SortKey::Size), true),
        (
            "Modified".to_string(),
            time_w,
            Some(SortKey::Modified),
            false,
        ),
        ("Action".to_string(), marker_w, Some(SortKey::Action), false),
        (side_title(false), name_w, Some(SortKey::Name), false),
        ("Size".to_string(), size_w, Some(SortKey::Size), true),
        (
            "Modified".to_string(),
            time_w,
            Some(SortKey::Modified),
            false,
        ),
    ];
    for (n, (text, w, key, right)) in columns.into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(x, bar.top()), egui::vec2(w, bar.height()));
        x += w;
        let Some(key) = key else { continue };
        let response = ui.interact(
            rect,
            Id::NULL.with(("fm-sync-head", n)),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Sort by {text}"))
        });
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, tokens.corner_radius, tokens.selection_bg);
        }
        let active = state.sort == key;
        let color = if active {
            tokens.text
        } else {
            tokens.text_muted
        };
        let font = egui::FontId::proportional(tokens.font_size);
        let (pos, anchor) = if right {
            (
                egui::pos2(rect.right() - 14.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
            )
        } else if key == SortKey::Action {
            (
                rect.center() - egui::vec2(14.0, 0.0),
                egui::Align2::CENTER_CENTER,
            )
        } else {
            (
                egui::pos2(rect.left() + 4.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
            )
        };
        let galley_rect = ui
            .painter()
            .with_clip_rect(rect)
            .text(pos, anchor, &text, font, color);
        if active {
            // A small triangle: up is ascending.
            let c = egui::pos2(galley_rect.right() + 8.0, rect.center().y);
            let up = !state.descending;
            let (a, b, d) = if up {
                (
                    c + egui::vec2(-4.0, 2.5),
                    c + egui::vec2(4.0, 2.5),
                    c + egui::vec2(0.0, -3.0),
                )
            } else {
                (
                    c + egui::vec2(-4.0, -2.5),
                    c + egui::vec2(4.0, -2.5),
                    c + egui::vec2(0.0, 3.0),
                )
            };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![a, b, d],
                color,
                egui::Stroke::NONE,
            ));
        }
        if key == SortKey::Action {
            action_filter_menu(ui, tokens, state, rect, commands);
        }
        let next = if !active {
            (key, false, "Sort ascending")
        } else if !state.descending {
            (key, true, "Sort descending")
        } else {
            (SortKey::Path, false, "Back to folder order")
        };
        if response.on_hover_text(next.2).clicked() {
            let name = match next.0 {
                SortKey::Path => "path",
                SortKey::Name => "name",
                SortKey::Size => "size",
                SortKey::Modified => "modified",
                SortKey::Action => "action",
            };
            commands.push(("sync.sort", json!({ "key": name, "descending": next.1 })));
        }
    }
    if state.rows.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            let msg = if plan.items.is_empty() {
                "Both folders are empty."
            } else if state.filter == RowFilter::All {
                "Nothing to show."
            } else {
                "No rows match this filter."
            };
            ui.label(RichText::new(msg).color(tokens.text_muted));
        });
    }
    let view_id = Id::NULL.with(("fm-sync-tree", &state.profile.id));
    let mut view: TreeView = ui.data_mut(|d| d.get_temp(view_id)).unwrap_or_default();
    let shown = visible_rows(state, tree.then_some(&view.collapsed));
    let selected_item = view
        .selected
        .and_then(|i| plan.items.get(i as usize))
        .filter(|i| tree || !i.rel.is_empty());
    let detail_h = if selected_item.is_some() { 52.0 } else { 0.0 };
    let mut toggled: Option<String> = None;
    let mut picked: Option<u32> = None;
    // show_rows spaces rows by the outer item spacing; rows are drawn flush, so it must be zero.
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let out = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .max_height((ui.available_height() - reserve - detail_h).max(120.0))
        .show_rows(ui, row_h, shown.len(), |ui, range| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            for n in range {
                let Some((index, item)) = shown.get(n).and_then(|i| {
                    plan.items
                        .get(usize::try_from(*i).unwrap_or(usize::MAX))
                        .map(|item| (*i, item))
                }) else {
                    continue;
                };
                let is_dir = tree && item.kind() == NodeKind::Dir;
                let shape = RowShape {
                    policy: (mode, source),
                    tree,
                    collapsed: is_dir && view.collapsed.contains(&item.rel),
                    rollup: is_dir.then(|| rollup(plan, index as usize)),
                    selected: view.selected == Some(index),
                    left_root: &plan.left_root,
                    right_root: &plan.right_root,
                };
                let event = row(
                    ui,
                    tokens,
                    item,
                    &shape,
                    row_h,
                    [name_w, size_w, time_w, marker_w],
                    commands,
                );
                if event.toggle {
                    toggled = Some(item.rel.clone());
                }
                if event.select {
                    picked = Some(index);
                }
            }
        });
    ui.spacing_mut().item_spacing.y = spacing;
    if let Some(rel) = toggled
        && !view.collapsed.remove(&rel)
    {
        view.collapsed.insert(rel);
    }
    if let Some(i) = picked {
        view.selected = if view.selected == Some(i) {
            None
        } else {
            Some(i)
        };
    }
    ui.data_mut(|d| d.insert_temp(view_id, view));
    // Outline and column dividers make the list read as a scrollable table.
    let table = egui::Rect::from_min_max(bar.left_top(), out.inner_rect.right_bottom());
    let line = egui::Stroke::new(1.0, tokens.text_muted.gamma_multiply(0.45));
    let painter = ui.painter();
    let mut x = bar.left();
    for w in [name_w, size_w, time_w, marker_w, name_w, size_w] {
        x += w;
        painter.vline(
            x,
            table.y_range(),
            egui::Stroke::new(1.0, line.color.gamma_multiply(0.5)),
        );
    }
    painter.rect_stroke(table, tokens.corner_radius, line, egui::StrokeKind::Inside);
    if let Some(item) = selected_item {
        detail_strip(ui, tokens, plan, item);
    }
}

/// Folders the user has folded shut and the row whose details are shown below the table.
#[derive(Clone, Default)]
struct TreeView {
    collapsed: std::collections::HashSet<String>,
    selected: Option<u32>,
}

/// What `row` needs to know beyond the item itself.
struct RowShape<'a> {
    policy: (SyncMode, Side),
    tree: bool,
    collapsed: bool,
    rollup: Option<String>,
    selected: bool,
    left_root: &'a VfsUri,
    right_root: &'a VfsUri,
}

/// How a cell's text is placed; names are shortened in the middle when they do not fit.
#[derive(Clone, Copy, PartialEq)]
enum Align {
    Name,
    Left,
    Right,
}

#[derive(Default)]
struct RowEvent {
    toggle: bool,
    select: bool,
}

/// Plan indices to list: the filtered rows without the contents of folded folders.
/// A funnel at the right of the Action header: show only rows that will do one kind of thing.
fn action_filter_menu(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    header: egui::Rect,
    commands: &mut Vec<Command>,
) {
    let rect = egui::Rect::from_center_size(
        egui::pos2(header.right() - 13.0, header.center().y),
        egui::vec2(22.0, header.height()),
    );
    let response = ui.interact(
        rect,
        Id::NULL.with("fm-sync-action-filter"),
        egui::Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Filter by action"));
    let on = state.action_filter.is_some();
    if on || response.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(1.0, 3.0)),
            tokens.corner_radius,
            tokens.selection_bg,
        );
    }
    let color = if on { tokens.accent } else { tokens.text_muted };
    sync_icons::paint(ui.painter(), Icon::Funnel, rect.center(), 13.0, color);
    let response = response
        .on_hover_text("Filter by action")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&response).show(|ui| {
        let options = [
            (None, "all", "All actions"),
            (
                Some(Action::CopyToRight),
                "copy_to_right",
                "Copy to the right",
            ),
            (Some(Action::CopyToLeft), "copy_to_left", "Copy to the left"),
            (
                Some(Action::DeleteLeft),
                "delete_left",
                "Delete on the left",
            ),
            (
                Some(Action::DeleteRight),
                "delete_right",
                "Delete on the right",
            ),
            (Some(Action::Skip), "skip", "Skipped"),
            (Some(Action::None), "none", "No action"),
        ];
        for (action, name, label) in options {
            let icon = if name == "all" {
                Icon::Funnel
            } else {
                Icon::for_action(name)
            };
            if sync_icons::menu_item(ui, tokens, icon, label, state.action_filter == action)
                .clicked()
            {
                commands.push(("sync.filter_action", json!({ "action": name })));
            }
        }
    });
}

fn visible_rows(
    state: &SyncState,
    collapsed: Option<&std::collections::HashSet<String>>,
) -> Vec<u32> {
    let (Some(plan), Some(collapsed)) = (&state.plan, collapsed) else {
        return state.rows.clone();
    };
    if collapsed.is_empty() {
        return state.rows.clone();
    }
    let prefixes: Vec<String> = collapsed.iter().map(|rel| format!("{rel}/")).collect();
    state
        .rows
        .iter()
        .copied()
        .filter(|&i| {
            plan.items
                .get(i as usize)
                .is_some_and(|item| !prefixes.iter().any(|p| item.rel.starts_with(p.as_str())))
        })
        .collect()
}

/// "812 files · 398 to the right · 2 conflicts" for the folder at `index` and everything in it.
fn rollup(plan: &fm_sync::Plan, index: usize) -> String {
    let Some(dir) = plan.items.get(index) else {
        return String::new();
    };
    let prefix = format!("{}/", dir.rel);
    let (mut files, mut right, mut left, mut deleted, mut conflicts) = (0, 0, 0, 0, 0);
    // Sorted by path, so everything under a folder is one contiguous run.
    let start = plan
        .items
        .partition_point(|i| i.rel.as_str() < prefix.as_str());
    for item in plan.items.iter().skip(start) {
        if !item.rel.starts_with(&prefix) {
            break;
        }
        if item.kind() == NodeKind::Dir {
            continue;
        }
        files += 1;
        match item.action {
            Action::CopyToRight => right += 1,
            Action::CopyToLeft => left += 1,
            Action::DeleteLeft | Action::DeleteRight => deleted += 1,
            _ => {}
        }
        if matches!(item.status, Status::Conflict | Status::Unknown) {
            conflicts += 1;
        }
    }
    let mut parts = vec![format!("{files} file{}", if files == 1 { "" } else { "s" })];
    for (n, what) in [
        (right, "to the right"),
        (left, "to the left"),
        (deleted, "deleted"),
        (conflicts, "in conflict"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    parts.join("  ·  ")
}

/// Full name and both full paths of the selected row, wrapped so nothing is cut off.
fn detail_strip(ui: &mut egui::Ui, tokens: &Tokens, plan: &fm_sync::Plan, item: &Item) {
    let full = |root: &VfsUri, present: bool| full_path(root, item, present);
    egui::Frame::new()
        .stroke(egui::Stroke::new(
            1.0,
            tokens.text_muted.gamma_multiply(0.45),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(&item.rel).color(tokens.text)).wrap());
            let side = |name: &str, text: String| {
                RichText::new(format!("{name} {text}")).color(tokens.text_muted)
            };
            ui.add(
                egui::Label::new(side("Left:", full(&plan.left_root, item.left.is_some())))
                    .truncate(),
            );
            ui.add(
                egui::Label::new(side("Right:", full(&plan.right_root, item.right.is_some())))
                    .truncate(),
            );
        });
}

/// `text` shortened in the middle so both its start and its extension stay visible.
fn fit_middle(
    painter: &egui::Painter,
    text: &str,
    font: &egui::FontId,
    color: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let whole = painter.layout_no_wrap(text.to_string(), font.clone(), color);
    if whole.size().x <= max_w {
        return whole;
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (1usize, chars.len());
    let mut best = painter.layout_no_wrap("…".to_string(), font.clone(), color);
    while lo < hi {
        let keep = (lo + hi).div_ceil(2);
        let tail = (keep * 2 / 5).min(keep.saturating_sub(1)).max(1);
        let head = keep - tail;
        let cut: String = chars
            .iter()
            .take(head)
            .chain(['…'].iter())
            .chain(chars.iter().skip(chars.len().saturating_sub(tail)))
            .collect();
        let galley = painter.layout_no_wrap(cut, font.clone(), color);
        if galley.size().x <= max_w {
            best = galley;
            lo = keep;
        } else {
            hi = keep - 1;
        }
    }
    best
}

fn row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    item: &Item,
    shape: &RowShape,
    row_h: f32,
    widths: [f32; 4],
    commands: &mut Vec<Command>,
) -> RowEvent {
    let [name_w, size_w, time_w, marker_w] = widths;
    let tree = shape.tree;
    let is_dir = tree && item.kind() == NodeKind::Dir;
    let mut event = RowEvent::default();
    let (rect, row_response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    if is_dir {
        ui.painter()
            .rect_filled(rect, 0.0, tokens.panel.lerp_to_gamma(tokens.text, 0.1));
    }
    if row_response.hovered() || shape.selected {
        ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
    }
    if shape.selected {
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, tokens.accent),
            egui::StrokeKind::Inside,
        );
    }
    let changing = matches!(
        item.action,
        Action::CopyToRight | Action::CopyToLeft | Action::DeleteLeft | Action::DeleteRight
    );
    let deleting = matches!(item.action, Action::DeleteLeft | Action::DeleteRight);
    let equal = item.status == Status::Equal;
    if matches!(item.status, Status::Conflict | Status::Unknown) {
        ui.painter().rect_filled(rect, 0.0, tokens.sync_conflict_bg);
    } else if changing && !is_dir {
        let tint = if deleting {
            tokens.diff_del_bg
        } else {
            tokens.diff_add_bg
        };
        ui.painter().rect_filled(rect, 0.0, tint);
    }
    if changing && !is_dir {
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, row_h));
        let (_, _, color) = marker(item, tokens);
        ui.painter().rect_filled(bar, 0.0, color);
    }
    let font = egui::FontId::proportional(tokens.font_size);
    let indent = if tree {
        4.0 + (item.depth() as f32 * 16.0).min(name_w * 0.5)
    } else {
        0.0
    };
    let leaf = if tree {
        item.rel.rsplit('/').next().unwrap_or(&item.rel)
    } else {
        item.rel.as_str()
    };
    let mut x = rect.left();
    let painter = ui.painter().clone();
    let cell = |x: f32, text: &str, w: f32, color: Color32, pad: f32, align: Align| {
        let cell = egui::Rect::from_min_size(
            egui::pos2(x + pad, rect.top()),
            egui::vec2((w - pad - 4.0).max(0.0), row_h),
        );
        let galley = if align != Align::Name {
            painter.layout_no_wrap(text.to_string(), font.clone(), color)
        } else {
            fit_middle(&painter, text, &font, color, cell.width())
        };
        let left = if align == Align::Right {
            (cell.right() - galley.size().x - 10.0).max(cell.left())
        } else {
            cell.left()
        };
        painter.with_clip_rect(cell).galley(
            egui::pos2(left, cell.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    };
    for s in [Side::Left, Side::Right] {
        let (size, when) = node_text(item, s);
        let present = item.node(s).is_some();
        let name = if present {
            if item.kind() == NodeKind::Dir {
                format!("{leaf}/")
            } else {
                leaf.to_string()
            }
        } else {
            String::new()
        };
        let name_color = if equal {
            tokens.text_muted
        } else {
            tokens.text
        };
        if is_dir && s == Side::Left {
            let c = egui::pos2(rect.left() + indent + 6.0, rect.center().y);
            let pts = if shape.collapsed {
                vec![
                    c + egui::vec2(-2.5, -4.0),
                    c + egui::vec2(-2.5, 4.0),
                    c + egui::vec2(3.0, 0.0),
                ]
            } else {
                vec![
                    c + egui::vec2(-4.0, -2.5),
                    c + egui::vec2(4.0, -2.5),
                    c + egui::vec2(0.0, 3.0),
                ]
            };
            painter.add(egui::Shape::convex_polygon(
                pts,
                tokens.text,
                egui::Stroke::NONE,
            ));
            cell(x, &name, name_w, tokens.text, indent + 16.0, Align::Name);
            x += name_w;
            if let Some(text) = &shape.rollup {
                cell(
                    x,
                    text,
                    size_w + time_w,
                    tokens.text_muted,
                    0.0,
                    Align::Left,
                );
            }
            x += size_w + time_w;
        } else {
            cell(x, &name, name_w, name_color, indent, Align::Name);
            x += name_w;
            cell(x, &size, size_w, tokens.text_muted, 0.0, Align::Right);
            x += size_w;
            cell(x, &when, time_w, tokens.text_muted, 0.0, Align::Left);
            x += time_w;
        }
        if s == Side::Left {
            let marker_rect =
                egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(marker_w, row_h));
            let (_, meaning, color) = marker(item, tokens);
            let response = ui.interact(
                marker_rect,
                Id::NULL.with(("fm-sync-marker", &item.rel)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    format!("{}: {meaning}", item.rel),
                )
            });
            if response.hovered() {
                ui.painter()
                    .rect_filled(marker_rect, tokens.corner_radius, tokens.hover_bg);
            }
            let icon_color = if changing { tokens.bg } else { color };
            if changing {
                let pill = egui::Rect::from_center_size(
                    marker_rect.center(),
                    egui::vec2(38.0, row_h - 6.0),
                );
                ui.painter().rect_filled(pill, row_h, color);
            }
            sync_icons::paint(
                ui.painter(),
                marker_icon(item),
                marker_rect.center(),
                16.0,
                icon_color,
            );
            let note = item.note.clone();
            let response = response.on_hover_text(match &note {
                Some(n) => format!("{meaning} — {n}"),
                None => meaning.to_string(),
            });
            egui::Popup::menu(&response).show(|ui| {
                for (name, label) in choices(item, shape.policy.0, shape.policy.1) {
                    let label = if is_dir {
                        label
                            .replace("Skip this item", "Skip everything in this folder")
                            .replace("Do nothing", "Do nothing with this folder")
                            .replace("Copy to", "Copy everything to")
                            .replace("Delete on", "Delete everything on")
                    } else {
                        label.to_string()
                    };
                    let clicked =
                        sync_icons::menu_item(ui, tokens, Icon::for_action(name), &label, false)
                            .on_hover_text(action_hint(name))
                            .clicked();
                    if clicked {
                        commands.push((
                            "sync.set_action",
                            json!({ "rel": item.rel, "action": name, "recursive": is_dir }),
                        ));
                        ui.close();
                    }
                }
                if !item.rel.is_empty() && !item.rel.contains(['*', '?']) {
                    ui.separator();
                    let label = if is_dir {
                        "Add folder to ignore list"
                    } else {
                        "Add to ignore list"
                    };
                    if sync_icons::menu_item(ui, tokens, Icon::Funnel, label, false)
                        .on_hover_text(
                            "Leaves this out of every future comparison of this sync (step 5, \
                             Leave out) and compares again.",
                        )
                        .clicked()
                    {
                        commands
                            .push(("sync.ignore", json!({ "rel": item.rel, "folder": is_dir })));
                        ui.close();
                    }
                }
            });
            x += marker_w;
        }
    }
    if is_dir {
        let folder =
            egui::Rect::from_min_size(rect.min, egui::vec2(name_w + size_w + time_w, row_h));
        let fold = ui.interact(
            folder,
            Id::NULL.with(("fm-sync-fold", &item.rel)),
            egui::Sense::click(),
        );
        fold.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!(
                    "{} folder {}",
                    if shape.collapsed {
                        "Expand"
                    } else {
                        "Collapse"
                    },
                    item.rel
                ),
            )
        });
        if fold.clicked() {
            event.toggle = true;
            event.select = true;
        }
    }
    if row_response.clicked() {
        event.select = true;
    }
    row_response.on_hover_text(format!(
        "{}\nLeft: {}\nRight: {}",
        item.rel,
        full_path(shape.left_root, item, item.left.is_some()),
        full_path(shape.right_root, item, item.right.is_some()),
    ));
    event
}

fn full_path(root: &VfsUri, item: &Item, present: bool) -> String {
    if present {
        format!("{}/{}", display_path(root).trim_end_matches('/'), item.rel)
    } else {
        "(does not exist yet)".to_string()
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
