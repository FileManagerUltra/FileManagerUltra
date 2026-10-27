//! The jobs panel and the status-bar indicator. Read-only over `Session::fileops`; cancel and
//! dismiss go through `job.cancel` / `job.dismiss`.

use egui::{Context, Id, RichText, Ui};
use fm_engine::{OpState, Session};
use serde_json::json;

use crate::theme::Tokens;

fn open_id() -> Id {
    Id::NULL.with("fm-jobs-open")
}

/// Whether the panel should currently be visible: explicitly toggled, or something needs the
/// user's attention (a running job or a failure report).
pub fn panel_visible(ctx: &Context, s: &Session) -> bool {
    let toggled = ctx.data(|d| d.get_temp::<bool>(open_id())).unwrap_or(false);
    toggled
        || crate::updates::available(s)
        || s.fileops.jobs().iter().any(|j| {
            j.state.is_running()
                || matches!(j.state, OpState::Failed(_))
                || j.summary.as_ref().is_some_and(|x| x.failed_count > 0)
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
    match running.as_slice() {
        [] if crate::updates::available(s) => "Update available".into(),
        [] => String::new(),
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
            egui::Button::new(RichText::new(text).color(tokens.accent).font(tokens.font()))
                .frame(false),
        )
        .clicked()
    {
        let ctx = ui.ctx().clone();
        let now = ctx.data(|d| d.get_temp::<bool>(open_id())).unwrap_or(false);
        ctx.data_mut(|d| d.insert_temp(open_id(), !now));
    }
}

/// The panel as a bottom-right window. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
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
    let mut cancel = Vec::new();
    let mut dismiss = Vec::new();
    egui::Window::new("Jobs")
        .id(Id::NULL.with("fm-jobs-panel"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-8.0, -32.0))
        .frame(
            egui::Frame::window(&egui::Style::default())
                .fill(tokens.panel)
                .stroke(egui::Stroke::new(tokens.border_width, tokens.border))
                .corner_radius(tokens.corner_radius)
                .inner_margin(egui::Margin::same(tokens.padding as i8)),
        )
        .show(ctx, |ui| {
            ui.set_min_width(300.0);
            if crate::updates::available(s) {
                crate::updates::show(ui, s, tokens);
                if !rows.is_empty() {
                    ui.separator();
                }
            }
            if rows.is_empty() && !crate::updates::available(s) {
                ui.label(tokens.muted("No jobs"));
            }
            for row in &rows {
                ui.label(
                    RichText::new(&row.label)
                        .color(tokens.text)
                        .font(tokens.font()),
                );
                if row.running {
                    let bar = egui::ProgressBar::new(row.fraction.unwrap_or(0.0) as f32)
                        .fill(tokens.accent);
                    ui.add(bar);
                    if !row.rate.is_empty() {
                        ui.label(tokens.muted(&row.rate));
                    }
                    ui.label(tokens.muted(&row.current));
                    if ui.button("Cancel").clicked() {
                        cancel.push(row.id);
                    }
                } else {
                    ui.label(tokens.muted(&row.status));
                    for (item, err) in &row.failures {
                        ui.label(
                            RichText::new(format!("{item}: {err}"))
                                .color(tokens.error)
                                .font(tokens.font()),
                        );
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss.push(row.id);
                    }
                }
                ui.separator();
            }
        });
    for id in cancel {
        s.dispatch_or_notice("job.cancel", &json!({"job": id}));
    }
    for id in dismiss {
        s.dispatch_or_notice("job.dismiss", &json!({"job": id}));
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
