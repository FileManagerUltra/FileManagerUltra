//! The Activity panel (jobs, recent messages, hints, updates, minimised searches) and the
//! status-bar indicator. Read-only over the session; every button dispatches a command id.

use egui::{Context, Id, RichText, Ui};
use fm_engine::{OpState, Session};
use serde_json::json;

use crate::theme::Tokens;

fn open_id() -> Id {
    Id::NULL.with("fm-jobs-open")
}

/// A new message opens the panel for this long; hovering it keeps it open.
const MESSAGE_SHOWN: std::time::Duration = std::time::Duration::from_secs(5);
/// A one-time hint has buttons to read and choose from, so it stays longer.
const HINT_SHOWN: std::time::Duration = std::time::Duration::from_secs(15);

fn shown_for(s: &Session) -> std::time::Duration {
    if s.messages.newest_is_hint() {
        HINT_SHOWN
    } else {
        MESSAGE_SHOWN
    }
}

fn until_id() -> Id {
    Id::NULL.with("fm-jobs-until")
}

fn seen_id() -> Id {
    Id::NULL.with("fm-jobs-seen-pushes")
}

fn message_deadline(ctx: &Context) -> Option<std::time::Instant> {
    ctx.data(|d| d.get_temp::<std::time::Instant>(until_id()))
}

/// Explicit visibility overrides automatic opening for jobs, failures, updates and new
/// messages (which close again by themselves).
pub fn panel_visible(ctx: &Context, s: &Session) -> bool {
    ctx.data(|d| d.get_temp::<bool>(open_id()))
        .unwrap_or_else(|| {
            message_deadline(ctx).is_some_and(|t| std::time::Instant::now() < t)
                || crate::updates::available(s)
                || s.search
                    .as_ref()
                    .is_some_and(|x| x.minimized && x.status().running)
                || s.fileops.jobs().iter().any(|j| {
                    j.state.is_running()
                        || matches!(j.state, OpState::Failed(_))
                        || j.summary.as_ref().is_some_and(|x| x.failed_count > 0)
                })
        })
}

/// `12.3 MiB/s · 45 s left`; empty until a rate is known.
pub fn rate_text(bytes_per_sec: Option<f64>, eta_secs: Option<u64>) -> String {
    let Some(rate) = bytes_per_sec else {
        return String::new();
    };
    let speed = format!("{}/s", crate::listing::human_size(rate.max(0.0) as u64));
    match eta_secs {
        Some(s) if s >= 3600 => format!("{speed} · {}h {}m left", s / 3600, (s % 3600) / 60),
        Some(s) if s >= 60 => format!("{speed} · {}m {}s left", s / 60, s % 60),
        Some(s) => format!("{speed} · {s} s left"),
        None => speed,
    }
}

/// Status-bar text for running jobs, e.g. `2 jobs · 41%`; empty when idle.
pub fn indicator_text(s: &Session) -> String {
    let running: Vec<_> = s
        .fileops
        .jobs()
        .iter()
        .filter(|j| j.state.is_running())
        .collect();
    if let Some(status) = s
        .search
        .as_ref()
        .map(fm_engine::search_cmds::Search::status)
        .filter(|status| status.running)
    {
        return match running.len() {
            0 => format!("Finding files · {} found", status.found),
            n => format!("{} jobs", n + 1),
        };
    }
    match running.as_slice() {
        [] if crate::updates::available(s) => "Update available".into(),
        [] => {
            let failed = s
                .fileops
                .jobs()
                .iter()
                .filter(|j| {
                    matches!(j.state, OpState::Failed(_))
                        || j.summary.as_ref().is_some_and(|x| x.failed_count > 0)
                })
                .count();
            match failed {
                0 => String::new(),
                1 => "1 job needs attention".into(),
                n => format!("{n} jobs need attention"),
            }
        }
        [one] => match one.progress.bytes.fraction() {
            Some(f) => format!("{} · {:.0}%", one.label, f * 100.0),
            None => one.label.clone(),
        },
        many => format!("{} jobs", many.len()),
    }
}

/// The clickable indicator; toggles the panel.
pub fn indicator(ui: &mut Ui, s: &Session, tokens: &Tokens) {
    let text = indicator_text(s);
    if text.is_empty() {
        return;
    }
    if ui
        .add(
            egui::Button::new(
                RichText::new(text)
                    .color(tokens.accent_text)
                    .font(tokens.font()),
            )
            .frame(false),
        )
        .clicked()
    {
        let ctx = ui.ctx().clone();
        let now = panel_visible(&ctx, s);
        ctx.data_mut(|d| d.insert_temp(open_id(), !now));
    }
}

/// Always-visible status-bar icon (a list with a badge dot while there is something to see);
/// toggles the panel.
pub fn icon_button(ui: &mut Ui, s: &Session, tokens: &Tokens) {
    let size = egui::vec2(tokens.font_size + 6.0, tokens.font_size + 6.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let ctx = ui.ctx().clone();
    let open = panel_visible(&ctx, s);
    let attention = !s.messages.is_empty()
        || s.fileops.jobs().iter().any(|j| {
            matches!(j.state, OpState::Failed(_))
                || j.summary.as_ref().is_some_and(|x| x.failed_count > 0)
        });
    let busy = s.fileops.running() > 0;
    let color = if open || response.hovered() {
        tokens.accent_text
    } else {
        tokens.text_muted
    };
    let painter = ui.painter();
    let inner = rect.shrink(5.0);
    for i in 0..3 {
        let y = inner.top() + inner.height() * (0.15 + 0.35 * i as f32);
        painter.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            egui::Stroke::new(1.5, color),
        );
    }
    if attention || busy {
        let dot = if attention {
            tokens.error
        } else {
            tokens.accent
        };
        painter.circle_filled(rect.right_top() + egui::vec2(-3.0, 4.0), 3.0, dot);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Activity"));
    if response
        .on_hover_text("Activity: jobs and recent messages")
        .clicked()
    {
        ctx.data_mut(|d| d.insert_temp(open_id(), !open));
    }
}

/// The panel as a bottom-right window. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    if s.drain_notices() {
        ctx.request_repaint();
    }
    let pushes = s.messages.pushes();
    if ctx.data(|d| d.get_temp::<u64>(seen_id())) != Some(pushes) {
        // First frame after startup or a new message: show it, even if the panel was closed.
        let fresh = ctx.data(|d| d.get_temp::<u64>(seen_id())).is_some();
        ctx.data_mut(|d| d.insert_temp(seen_id(), pushes));
        if fresh && !s.messages.is_empty() {
            ctx.data_mut(|d| {
                d.remove_temp::<bool>(open_id());
                d.insert_temp(until_id(), std::time::Instant::now() + shown_for(s));
            });
        }
    }
    if let Some(left) = message_deadline(ctx)
        .map(|t| t.saturating_duration_since(std::time::Instant::now()))
        .filter(|left| !left.is_zero())
    {
        ctx.request_repaint_after(left);
    }
    if !panel_visible(ctx, s) {
        return;
    }
    struct Row {
        id: u64,
        label: String,
        running: bool,
        fraction: Option<f64>,
        current: String,
        rate: String,
        status: String,
        failures: Vec<(String, String)>,
    }
    let rows: Vec<Row> = s
        .fileops
        .jobs()
        .iter()
        .map(|j| {
            let (status, failures) = match (&j.state, &j.summary) {
                (OpState::Failed(msg), _) => (format!("Failed: {msg}"), Vec::new()),
                (OpState::Cancelled, _) => ("Cancelled".to_string(), Vec::new()),
                (_, Some(sum)) => (
                    format!(
                        "{} done, {} skipped, {} failed",
                        sum.done, sum.skipped, sum.failed_count
                    ),
                    sum.failed.clone(),
                ),
                _ => (String::new(), Vec::new()),
            };
            Row {
                id: j.id,
                label: j.label.clone(),
                running: j.state.is_running(),
                fraction: j.progress.bytes.fraction().or(j.progress.items.fraction()),
                current: j.progress.current.clone(),
                rate: if j.state.is_running() {
                    rate_text(
                        j.speed.bytes_per_sec(),
                        j.speed
                            .eta_secs(j.progress.bytes.total.saturating_sub(j.progress.bytes.done)),
                    )
                } else {
                    String::new()
                },
                status,
                failures,
            }
        })
        .collect();
    // A minimised Find Files search is listed like a job.
    let search_row = s.search.as_ref().filter(|x| x.minimized).map(|x| {
        let status = x.status();
        let saved = fm_engine::search_cmds::SavedSearch {
            root: x
                .query
                .root
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            name: x.query.name.clone(),
            content: x.query.content.clone(),
            hidden: x.query.hidden,
        };
        (
            format!("Find {}", saved.label()),
            crate::search::status_line(&status),
            status.running,
        )
    });
    if search_row.as_ref().is_some_and(|row| row.2) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    let mut search_action: Option<&'static str> = None;
    let mut cancel = Vec::new();
    let mut dismiss = Vec::new();
    struct Line {
        id: u64,
        text: String,
        age: u64,
        hint: bool,
        info: bool,
        /// Button labels and whether each only dismisses; empty once one was used.
        actions: Vec<(String, bool)>,
    }
    let messages: Vec<Line> = s
        .messages
        .recent()
        .map(|m| Line {
            id: m.id,
            text: m.text.clone(),
            age: m.at.elapsed().as_secs(),
            hint: m.hint.is_some(),
            info: m.info,
            actions: if m.spent {
                Vec::new()
            } else {
                m.actions
                    .iter()
                    .map(|a| {
                        let dismiss = a.commands.iter().all(|(id, _)| id == "hint.dismiss");
                        (a.label.clone(), dismiss)
                    })
                    .collect()
            },
        })
        .collect();
    let mut message_action: Option<(u64, usize)> = None;
    let mut clear_messages = false;
    let mut open = true;
    let shown = crate::dialog_style::window(ctx, tokens, "Activity", &mut open)
        .id(Id::NULL.with("fm-jobs-panel"))
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(ctx.content_rect().right_bottom() + egui::vec2(-8.0, -32.0))
        .show(ctx, |ui| {
            use crate::activity_card::{self as card, Tone};
            crate::dialog_style::apply(ui, tokens);
            ui.set_min_width(340.0);
            ui.set_max_width(420.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            if crate::updates::available(s) {
                crate::updates::show(ui, s, tokens);
            }
            if rows.is_empty()
                && search_row.is_none()
                && messages.is_empty()
                && !crate::updates::available(s)
            {
                crate::dialog_style::hint(ui, tokens, "No jobs or messages");
            }
            if let Some((label, status, running)) = &search_row {
                card::card(ui, tokens, Tone::Info, "Search", None, |ui| {
                    ui.add(egui::Label::new(RichText::new(label).color(tokens.text)).truncate());
                    ui.horizontal(|ui| {
                        if *running {
                            ui.spinner();
                        }
                        // The status carries the current path: never let it widen the panel.
                        ui.add(
                            egui::Label::new(
                                RichText::new(status)
                                    .font(egui::FontId::proportional(tokens.font_size - 1.0))
                                    .color(tokens.text_muted),
                            )
                            .truncate(),
                        );
                    });
                    card::buttons(ui, |ui| {
                        if card::primary(ui, tokens, "Show", true)
                            .on_hover_text("Open the Find Files window")
                            .clicked()
                        {
                            search_action = Some("search.restore");
                        }
                        if *running {
                            if card::secondary(ui, "Stop").clicked() {
                                search_action = Some("search.stop");
                            }
                        } else {
                            card::end(ui, |ui| {
                                if card::quiet(ui, tokens, "Dismiss").clicked() {
                                    search_action = Some("search.close");
                                }
                            });
                        }
                    });
                });
            }
            if !messages.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Messages")
                            .font(egui::FontId::proportional(tokens.font_size - 1.0))
                            .color(tokens.text_muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if card::quiet(ui, tokens, "Clear all").clicked() {
                            clear_messages = true;
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .id_salt("fm-jobs-messages")
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        for line in &messages {
                            let (tone, heading) = if line.hint {
                                (Tone::Info, "Tip")
                            } else if line.info {
                                (Tone::Info, "Info")
                            } else {
                                (Tone::Error, "Error")
                            };
                            let age = format!("{} s ago", line.age);
                            card::card(ui, tokens, tone, heading, Some(&age), |ui| {
                                card::text(ui, tokens, &line.text);
                                if line.actions.is_empty() {
                                    return;
                                }
                                let main: Vec<_> = line
                                    .actions
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, (_, dismiss))| !dismiss)
                                    .collect();
                                let quiet: Vec<_> = line
                                    .actions
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, (_, dismiss))| *dismiss)
                                    .collect();
                                card::buttons(ui, |ui| {
                                    for (n, (index, (label, _))) in main.iter().enumerate() {
                                        let clicked = if n == 0 {
                                            card::primary(ui, tokens, label, true)
                                        } else {
                                            card::secondary(ui, label)
                                        }
                                        .clicked();
                                        if clicked {
                                            message_action = Some((line.id, *index));
                                        }
                                    }
                                    if !quiet.is_empty() {
                                        card::end(ui, |ui| {
                                            for (index, (label, _)) in quiet.iter().rev() {
                                                if card::quiet(ui, tokens, label).clicked() {
                                                    message_action = Some((line.id, *index));
                                                }
                                            }
                                        });
                                    }
                                });
                            });
                        }
                    });
            }
            for row in &rows {
                let failed = row.status.starts_with("Failed") || !row.failures.is_empty();
                let tone = if failed { Tone::Error } else { Tone::Info };
                card::card(ui, tokens, tone, &row.label, None, |ui| {
                    if row.running {
                        let bar = egui::ProgressBar::new(row.fraction.unwrap_or(0.0) as f32)
                            .fill(tokens.accent);
                        ui.add(bar);
                        if !row.rate.is_empty() {
                            card::detail(ui, tokens, &row.rate, None);
                        }
                        card::detail(ui, tokens, &row.current, None);
                        card::buttons(ui, |ui| {
                            if card::secondary(ui, "Cancel").clicked() {
                                cancel.push(row.id);
                            }
                        });
                    } else {
                        card::detail(ui, tokens, &row.status, None);
                        for (item, err) in &row.failures {
                            card::detail(ui, tokens, &format!("{item}: {err}"), Some(tokens.error));
                        }
                        card::buttons(ui, |ui| {
                            card::end(ui, |ui| {
                                if card::quiet(ui, tokens, "Dismiss").clicked() {
                                    dismiss.push(row.id);
                                }
                            });
                        });
                    }
                });
            }
        });
    if !open {
        ctx.data_mut(|d| d.insert_temp(open_id(), false));
    }
    if shown.is_some_and(|r| r.response.hovered() || r.response.contains_pointer())
        && message_deadline(ctx).is_some()
    {
        let keep = shown_for(s);
        ctx.data_mut(|d| d.insert_temp(until_id(), std::time::Instant::now() + keep));
    }
    if let Some((id, index)) = message_action
        && let Some(commands) = s.messages.use_action(id, index)
    {
        // An action may be several commands (e.g. set a preference, then play), in order.
        for (command, params) in commands {
            s.dispatch_or_notice(&command, &params);
        }
    }
    if clear_messages {
        s.dispatch_or_notice("messages.clear", &serde_json::Value::Null);
    }
    for id in cancel {
        s.dispatch_or_notice("job.cancel", &json!({"job": id}));
    }
    for id in dismiss {
        s.dispatch_or_notice("job.dismiss", &json!({"job": id}));
    }
    if let Some(id) = search_action {
        s.dispatch_or_notice(id, &serde_json::Value::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_session_has_no_indicator_or_panel() {
        let s = Session::default();
        assert!(indicator_text(&s).is_empty());
        assert!(!panel_visible(&Context::default(), &s));
    }

    #[test]
    fn only_available_undismissed_updates_open_the_jobs_panel() {
        use fm_engine::updates::UpdateStatus;
        let ctx = Context::default();
        let mut s = Session::new().expect("session");
        s.updates.visible = true;
        for status in [
            UpdateStatus::Idle,
            UpdateStatus::Checking,
            UpdateStatus::UpToDate,
            UpdateStatus::Failed("offline".into()),
        ] {
            s.updates.status = status;
            assert!(!panel_visible(&ctx, &s));
            assert!(indicator_text(&s).is_empty());
        }
        s.updates.status = UpdateStatus::Available {
            version: "0.2.0".into(),
            url: String::new(),
        };
        assert!(panel_visible(&ctx, &s));
        assert_eq!(indicator_text(&s), "Update available");
        s.dispatch("app.updates.dismiss", &serde_json::Value::Null)
            .expect("dismiss");
        assert!(!panel_visible(&ctx, &s));
        assert!(indicator_text(&s).is_empty());
    }

    #[test]
    fn dismissing_an_update_keeps_file_jobs_visible_and_cancellable() {
        let ctx = Context::default();
        let mut s = Session::new().expect("session");
        s.backends
            .register(std::sync::Arc::new(fm_testkit::MemFs::new()));
        let job = s
            .submit_op(fm_engine::OpRequest::Mkdir {
                parent: fm_vfs::VfsUri::root_file(),
                name: "test-folder".into(),
            })
            .expect("submit");
        s.updates.visible = true;
        s.updates.status = fm_engine::updates::UpdateStatus::Available {
            version: "0.2.0".into(),
            url: String::new(),
        };
        assert!(panel_visible(&ctx, &s));
        let before = indicator_text(&s);
        assert!(before.contains("test-folder"), "{before}");
        s.dispatch("app.updates.dismiss", &serde_json::Value::Null)
            .expect("dismiss update");
        assert!(panel_visible(&ctx, &s), "file job still needs the panel");
        assert_eq!(indicator_text(&s), before);
        assert!(s.fileops.get(job).is_some());
        s.dispatch("job.cancel", &json!({"job": job}))
            .expect("cancel file job");
    }

    #[test]
    fn new_messages_open_the_panel_briefly_and_stay_reachable() {
        let ctx = Context::default();
        let tokens = Tokens::dark();
        let mut s = Session::new().expect("session");
        let frame = |ctx: &Context, s: &mut Session| {
            let mut out = ctx.run_ui(Default::default(), |ui| show(ui.ctx(), s, &tokens));
            out.textures_delta.clear();
        };
        frame(&ctx, &mut s);
        assert!(!panel_visible(&ctx, &s));
        s.notice = Some("Cannot open it".into());
        frame(&ctx, &mut s);
        assert!(s.notice.is_none());
        assert!(panel_visible(&ctx, &s));
        assert!(
            indicator_text(&s).is_empty(),
            "messages live behind the icon"
        );
        let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
        ctx.data_mut(|d| d.insert_temp(until_id(), past));
        assert!(!panel_visible(&ctx, &s), "closes by itself");
        assert_eq!(s.messages.len(), 1);
        ctx.data_mut(|d| d.insert_temp(open_id(), true));
        assert!(panel_visible(&ctx, &s), "the icon reopens it");
    }

    #[test]
    fn rate_text_formats_speed_and_eta() {
        assert_eq!(rate_text(None, Some(5)), "");
        assert_eq!(rate_text(Some(2_097_152.0), None), "2.0 MiB/s");
        assert_eq!(rate_text(Some(1024.0), Some(45)), "1.0 KiB/s · 45 s left");
        assert_eq!(rate_text(Some(1024.0), Some(125)), "1.0 KiB/s · 2m 5s left");
        assert_eq!(
            rate_text(Some(1024.0), Some(7300)),
            "1.0 KiB/s · 2h 1m left"
        );
    }
}
