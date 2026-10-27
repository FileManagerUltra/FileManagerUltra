use egui::{Context, DragValue, Id, RichText, Ui};
use fm_engine::preview_cmds::LogView;
use fm_layout::PaneId;
use jiff::civil::{Date, DateTime, Time};
use jiff::{Timestamp, tz::TimeZone};
use serde_json::{Value, json};

use super::Action;
use crate::theme::Tokens;

#[derive(Clone)]
struct Bound {
    enabled: bool,
    date: Date,
    hour: i8,
    minute: i8,
    second: i8,
}

impl Bound {
    fn new(enabled: bool, value: DateTime) -> Self {
        Self {
            enabled,
            date: value.date(),
            hour: value.hour(),
            minute: value.minute(),
            second: value.second(),
        }
    }

    fn value(&self) -> Result<Option<DateTime>, String> {
        if !self.enabled {
            return Ok(None);
        }
        if !(1..=9999).contains(&self.date.year()) {
            return Err("Choose a year between 1 and 9999.".into());
        }
        Time::new(self.hour, self.minute, self.second, 0)
            .map(|time| Some(self.date.to_datetime(time)))
            .map_err(|e| format!("Invalid time: {e}"))
    }

    fn show(&mut self, ui: &mut Ui, label: &str) {
        ui.push_id(label, |ui| {
            ui.checkbox(&mut self.enabled, label);
            ui.add_enabled_ui(self.enabled, |ui| {
                ui.horizontal(|ui| {
                    let year = self.date.year();
                    ui.add(
                        egui_extras::DatePickerButton::new(&mut self.date)
                            .id_salt(label)
                            .show_icon(false)
                            .calendar_week(false)
                            .highlight_weekends(false)
                            .start_end_years((year - 100).max(1)..=(year + 100).min(9999)),
                    )
                    .on_hover_text("Choose a date");
                    for (name, value, max) in [
                        ("Hour", &mut self.hour, 23),
                        ("Minute", &mut self.minute, 59),
                        ("Second", &mut self.second, 59),
                    ] {
                        let response = ui.add(
                            DragValue::new(value)
                                .range(0..=max)
                                .min_decimals(0)
                                .max_decimals(0)
                                .prefix(format!("{name}: ")),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::SpinButton,
                                response.enabled(),
                                format!("{label} {name}"),
                            )
                        });
                    }
                });
            });
        });
    }
}

#[derive(Clone)]
pub(super) struct TimePicker {
    pub open: bool,
    from: Bound,
    to: Bound,
    error: Option<String>,
}

impl TimePicker {
    pub fn new(view: &LogView) -> Self {
        let now = Timestamp::now().to_zoned(TimeZone::UTC).datetime();
        let mut picker = Self {
            open: true,
            from: Bound::new(view.filter.from.is_some(), now),
            to: Bound::new(view.filter.to.is_some(), now),
            error: None,
        };
        let initial = || -> Result<(DateTime, DateTime), String> {
            let doc = view
                .doc
                .try_read()
                .map_err(|e| format!("Cannot read log dates: {e}"))?;
            let convert = |seconds: Option<i64>| {
                seconds
                    .map(|s| {
                        Timestamp::from_second(s)
                            .map(|ts| ts.to_zoned(TimeZone::UTC).datetime())
                            .map_err(|e| format!("Cannot use log date: {e}"))
                    })
                    .transpose()
                    .map(|date| date.unwrap_or(now))
            };
            Ok((
                convert(
                    view.filter
                        .from
                        .or_else(|| doc.records.iter().filter_map(|r| r.ts).min()),
                )?,
                convert(
                    view.filter
                        .to
                        .or_else(|| doc.records.iter().filter_map(|r| r.ts).max()),
                )?,
            ))
        };
        match initial() {
            Ok((from, to)) => {
                picker.from = Bound::new(view.filter.from.is_some(), from);
                picker.to = Bound::new(view.filter.to.is_some(), to);
            }
            Err(error) => picker.error = Some(error),
        }
        picker
    }

    fn params(&self) -> Result<Value, String> {
        let (from, to) = (self.from.value()?, self.to.value()?);
        if from.zip(to).is_some_and(|(from, to)| from > to) {
            return Err("From must be before or equal to To.".into());
        }
        let format = |date: Option<DateTime>| {
            date.map(|d| d.strftime("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default()
        };
        Ok(json!({ "from": format(from), "to": format(to) }))
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        pane: PaneId,
        tokens: &Tokens,
        actions: &mut Vec<Action>,
    ) {
        let mut open = self.open;
        egui::Window::new("Log time range")
            .id(Id::NULL.with(("pv-time", pane)))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Times are UTC. Enable either bound to limit the preview.");
                self.from.show(ui, "From");
                ui.add_space(tokens.padding);
                self.to.show(ui, "To");
                ui.separator();
                if let Some(error) = &self.error {
                    ui.label(RichText::new(error).color(tokens.error));
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Apply")
                        .on_hover_text("Apply these UTC time bounds to the preview.")
                        .clicked()
                    {
                        match self.params() {
                            Ok(params) => {
                                actions.push(("preview.log.time_range", params));
                                self.open = false;
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if ui
                        .button("Clear")
                        .on_hover_text("Remove both time bounds; keep the other filters.")
                        .clicked()
                    {
                        actions.push(("preview.log.time_range", json!({})));
                        self.open = false;
                    }
                    if ui
                        .button("Cancel")
                        .on_hover_text("Close without changing the applied time bounds.")
                        .clicked()
                    {
                        self.open = false;
                    }
                });
            });
        self.open &= open;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use fm_preview::log::parse_bound;

    fn fixture() -> TimePicker {
        TimePicker {
            open: true,
            from: Bound::new(true, jiff::civil::date(2024, 5, 1).at(10, 0, 0, 0)),
            to: Bound::new(false, jiff::civil::date(2024, 5, 1).at(11, 0, 0, 0)),
            error: None,
        }
    }

    #[test]
    fn bounds_round_trip_validate_and_allow_open_ends() {
        let mut picker = fixture();
        let params = picker.params().expect("valid");
        assert_eq!(params["from"], "2024-05-01 10:00:00");
        assert_eq!(params["to"], "");
        assert!(parse_bound(params["from"].as_str().expect("date")).is_some());
        picker.to.enabled = true;
        picker.to.hour = 9;
        assert_eq!(
            picker.params().expect_err("reversed"),
            "From must be before or equal to To."
        );
        picker.to.hour = 10;
        assert!(picker.params().is_ok(), "equal bounds are inclusive");
        picker.from.enabled = false;
        assert_eq!(picker.params().expect("to only")["from"], "");
        picker.to.enabled = false;
        assert_eq!(
            picker.params().expect("unbounded"),
            json!({"from": "", "to": ""})
        );
        picker.from.enabled = true;
        picker.from.minute = 60;
        assert!(picker.params().is_err());
    }

    #[test]
    fn time_window_stays_centered_when_the_viewport_resizes() {
        let ctx = Context::default();
        let mut picker = fixture();
        for size in [egui::vec2(900.0, 650.0), egui::vec2(1100.0, 750.0)] {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(rect),
                        ..Default::default()
                    },
                    |ctx| picker.show(ctx, 1, &Tokens::dark(), &mut Vec::new()),
                );
                output.textures_delta.clear();
            }
            let window = ctx
                .memory(|m| m.area_rect(Id::NULL.with(("pv-time", 1_u64))))
                .expect("window");
            assert!(
                window.center().distance(rect.center()) < 1.0,
                "dialog remains centered"
            );
        }
    }

    #[test]
    fn calendar_and_time_edits_stay_open_until_apply_in_both_themes() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut harness = Harness::builder()
                .with_size(egui::vec2(900.0, 650.0))
                // The calendar's internal combo boxes lack accessible names.
                .with_accessibility_check(false)
                .build_ui_state(
                    |ui, (picker, actions): &mut (TimePicker, Vec<Action>)| {
                        crate::theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        if picker.open {
                            picker.show(ui.ctx(), 1, &tokens, actions);
                        }
                    },
                    (fixture(), Vec::new()),
                );
            harness.get_by_label("From Hour").click();
            harness.run();
            harness.get_by_label("From Hour").type_text("12");
            harness.key_press(egui::Key::Enter);
            harness.run();
            assert!(harness.state().0.open);
            assert_eq!(harness.state().0.from.hour, 12);
            assert!(harness.state().1.is_empty());
            harness
                .get_all_by_label("2024-05-01")
                .next()
                .expect("From date")
                .click();
            harness.run();
            harness.get_all_by_label("2").next().expect("day 2").click();
            harness.run();
            harness.get_by_label("Save").click();
            harness.run();
            assert!(harness.state().0.open);
            assert!(harness.state().1.is_empty());
            harness.get_by_label("Apply").click();
            harness.run();
            assert!(!harness.state().0.open);
            assert_eq!(
                harness.state().1,
                vec![(
                    "preview.log.time_range",
                    json!({"from": "2024-05-02 12:00:00", "to": ""})
                )]
            );
        }
    }

    #[test]
    fn invalid_ranges_stay_open_and_cancel_does_not_apply() {
        let mut picker = fixture();
        picker.to.enabled = true;
        picker.to.hour = 9;
        let mut harness = Harness::builder().build_ui_state(
            |ui, (picker, actions): &mut (TimePicker, Vec<Action>)| {
                if picker.open {
                    picker.show(ui.ctx(), 1, &Tokens::dark(), actions);
                }
            },
            (picker, Vec::new()),
        );
        harness.get_by_label("Apply").click();
        harness.run();
        harness.get_by_label("From must be before or equal to To.");
        assert!(harness.state().0.open);
        assert!(harness.state().1.is_empty());
        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(!harness.state().0.open);
        assert!(harness.state().1.is_empty());
    }
}
