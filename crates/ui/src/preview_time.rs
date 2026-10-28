use egui::{Context, Id, RichText, TextEdit, Ui};
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
    date: String,
    time: String,
    calendar: Date,
}

impl Bound {
    fn new(enabled: bool, value: DateTime) -> Self {
        Self {
            enabled,
            date: value.date().to_string(),
            time: value.strftime("%H:%M:%S").to_string(),
            calendar: value.date(),
        }
    }

    fn value(&self) -> Result<Option<DateTime>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let date = self
            .date
            .trim()
            .parse::<Date>()
            .map_err(|_| "Enter a valid date as YYYY-MM-DD.".to_string())?;
        if !(1..=9999).contains(&date.year()) {
            return Err("Choose a year between 1 and 9999.".into());
        }
        let parts: Vec<_> = self.time.trim().split(':').collect();
        let invalid = || "Enter a valid time as HH:MM or HH:MM:SS (00:00 to 23:59:59).".to_string();
        if !(2..=3).contains(&parts.len())
            || parts
                .iter()
                .any(|part| part.len() != 2 || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(invalid());
        }
        let number = |i: usize| -> Result<i8, String> {
            parts
                .get(i)
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())
        };
        let time = Time::new(
            number(0)?,
            number(1)?,
            if parts.len() == 3 { number(2)? } else { 0 },
            0,
        )
        .map_err(|_| invalid())?;
        Ok(Some(date.to_datetime(time)))
    }

    fn show(&mut self, ui: &mut Ui, label: &str) -> bool {
        let mut changed = false;
        ui.push_id(label, |ui| {
            changed |= ui.checkbox(&mut self.enabled, label).changed();
            ui.add_enabled_ui(self.enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(112.0);
                        ui.label("Date");
                        let response = ui.add(
                            TextEdit::singleline(&mut self.date)
                                .desired_width(112.0)
                                .char_limit(16)
                                .hint_text("YYYY-MM-DD"),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::TextInput,
                                response.enabled(),
                                format!("{label} date"),
                            )
                        });
                        changed |= response.changed();
                    });
                    ui.vertical(|ui| {
                        ui.set_width(98.0);
                        ui.label("Time (UTC)");
                        let response = ui.add(
                            TextEdit::singleline(&mut self.time)
                                .desired_width(98.0)
                                .char_limit(16)
                                .hint_text("HH:MM[:SS]"),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::TextInput,
                                response.enabled(),
                                format!("{label} time"),
                            )
                        });
                        changed |= response.changed();
                    });
                });
                if let Ok(date) = self.date.trim().parse::<Date>()
                    && (1..=9999).contains(&date.year())
                {
                    self.calendar = date;
                }
                let year = self.calendar.year();
                if ui
                    .add(
                        egui_extras::DatePickerButton::new(&mut self.calendar)
                            .id_salt(label)
                            .format("Calendar")
                            .show_icon(false)
                            .calendar_week(false)
                            .highlight_weekends(false)
                            .start_end_years((year - 100).max(1)..=(year + 100).min(9999)),
                    )
                    .on_hover_text(format!("Choose {label} date"))
                    .changed()
                {
                    self.date = self.calendar.to_string();
                    changed = true;
                }
            });
        });
        changed
    }
}

const QUICK_RANGES: [(&str, i64); 10] = [
    ("Last 5 minutes", 5 * 60),
    ("Last 15 minutes", 15 * 60),
    ("Last 30 minutes", 30 * 60),
    ("Last 1 hour", 60 * 60),
    ("Last 3 hours", 3 * 60 * 60),
    ("Last 6 hours", 6 * 60 * 60),
    ("Last 12 hours", 12 * 60 * 60),
    ("Last 24 hours", 24 * 60 * 60),
    ("Last 3 days", 3 * 24 * 60 * 60),
    ("Last 7 days", 7 * 24 * 60 * 60),
];

#[derive(Clone)]
pub(super) struct TimePicker {
    pub open: bool,
    from: Bound,
    to: Bound,
    relative: Option<i64>,
    error: Option<String>,
}

impl TimePicker {
    pub fn new(view: &LogView) -> Self {
        let now = Timestamp::now().to_zoned(TimeZone::UTC).datetime();
        let mut picker = Self {
            open: true,
            from: Bound::new(view.filter.from.is_some(), now),
            to: Bound::new(view.filter.to.is_some(), now),
            relative: None,
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
        self.params_at(Timestamp::now())
    }

    fn relative_start(seconds: i64, now: Timestamp) -> Result<DateTime, String> {
        let start = now
            .checked_sub(std::time::Duration::from_secs(
                u64::try_from(seconds).map_err(|_| "Invalid quick range.".to_string())?,
            ))
            .map_err(|e| format!("Cannot calculate time range: {e}"))?;
        Ok(start.to_zoned(TimeZone::UTC).datetime())
    }

    fn select_relative(&mut self, seconds: i64, now: Timestamp) -> Result<(), String> {
        let from = Self::relative_start(seconds, now)?;
        self.from = Bound::new(true, from);
        self.to.enabled = false;
        self.relative = Some(seconds);
        self.error = None;
        Ok(())
    }

    fn params_at(&self, now: Timestamp) -> Result<Value, String> {
        let (from, to) = if let Some(seconds) = self.relative {
            (Some(Self::relative_start(seconds, now)?), None)
        } else {
            (
                self.from.value().map_err(|e| format!("From: {e}"))?,
                self.to.value().map_err(|e| format!("To: {e}"))?,
            )
        };
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
        crate::dialog_style::window(ctx, tokens, "Log time range", &mut open)
            .id(Id::NULL.with(("pv-time", pane)))
            .show(ctx, |ui| {
                crate::dialog_style::apply(ui, tokens);
                crate::dialog_style::body(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(152.0);
                        crate::dialog_style::section_heading(ui, tokens, "Quick ranges");
                        ui.add_space(tokens.padding);
                        for (label, seconds) in QUICK_RANGES {
                            if ui
                                .selectable_label(self.relative == Some(seconds), label)
                                .clicked()
                                && let Err(error) = self.select_relative(seconds, Timestamp::now())
                            {
                                self.error = Some(error);
                            }
                        }
                        ui.separator();
                        if ui
                            .selectable_label(self.relative.is_none(), "Custom range")
                            .clicked()
                        {
                            self.relative = None;
                            self.error = None;
                        }
                    });
                    let (divider, _) =
                        ui.allocate_exact_size(egui::vec2(1.0, 276.0), egui::Sense::hover());
                    ui.painter().line_segment(
                        [divider.center_top(), divider.center_bottom()],
                        egui::Stroke::new(tokens.border_width, tokens.border),
                    );
                    ui.vertical(|ui| {
                        ui.set_width(292.0);
                        crate::dialog_style::section_heading(ui, tokens, "Custom time range");
                        ui.label("UTC (UTC+00:00)");
                        ui.label("Log timestamps keep their original offsets.");
                        ui.add_space(tokens.padding);
                        let from_changed = self.from.show(ui, "From");
                        ui.add_space(tokens.padding);
                        let to_changed = self.to.show(ui, "To");
                        if from_changed || to_changed {
                            self.relative = None;
                            self.error = None;
                        }
                        ui.add_space(tokens.padding);
                        crate::dialog_style::hint(ui, tokens, if self.relative.is_some() {
                            "From is calculated when you Apply.\nTo stays open to include new rows with Live tail."
                        } else {
                            "Use HH:MM or HH:MM:SS.\nUncheck a bound for an open-ended range."
                        });
                    });
                });
                });
                ui.separator();
                crate::dialog_style::hint(ui, tokens, "Filters apply to loaded records only.");
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
            relative: None,
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
        picker.to.time = "09:00".into();
        assert_eq!(
            picker.params().expect_err("reversed"),
            "From must be before or equal to To."
        );
        picker.to.time = "10:00".into();
        assert!(picker.params().is_ok(), "equal bounds are inclusive");
        picker.from.enabled = false;
        assert_eq!(picker.params().expect("to only")["from"], "");
        picker.to.enabled = false;
        assert_eq!(
            picker.params().expect("unbounded"),
            json!({"from": "", "to": ""})
        );
        picker.from.enabled = true;
        picker.from.time = "10:60".into();
        assert!(picker.params().is_err());
    }

    #[test]
    fn typed_dates_and_times_validate_without_losing_open_ends() {
        let mut bound = Bound::new(true, jiff::civil::date(2024, 2, 29).at(0, 0, 0, 0));
        for (input, expected) in [
            ("00:00", "00:00:00"),
            ("23:59:59", "23:59:59"),
            (" 09:30 ", "09:30:00"),
        ] {
            bound.time = input.into();
            assert_eq!(
                bound
                    .value()
                    .expect("valid")
                    .expect("enabled")
                    .strftime("%H:%M:%S")
                    .to_string(),
                expected
            );
        }
        for input in [
            "",
            "9:30",
            "09:3",
            "24:00",
            "12:60",
            "12:30:60",
            "12:30:00:00",
            "aa:bb",
        ] {
            bound.time = input.into();
            assert!(bound.value().is_err(), "{input}");
        }
        bound.time = "09:30".into();
        for input in ["2023-02-29", "2024-04-31", "not a date", "0000-01-01"] {
            bound.date = input.into();
            assert!(bound.value().is_err(), "{input}");
        }
        bound.enabled = false;
        assert_eq!(
            bound.value().expect("disabled invalid bound is ignored"),
            None
        );
    }

    #[test]
    fn every_quick_range_starts_before_apply_time_and_leaves_to_open() {
        for end in ["2024-03-01T00:15:00Z", "2025-01-01T00:15:00Z"] {
            let selected: Timestamp = end.parse().expect("timestamp");
            let applied = selected
                .checked_add(std::time::Duration::from_secs(90))
                .expect("later");
            for (_, seconds) in QUICK_RANGES {
                let mut picker = fixture();
                picker.select_relative(seconds, selected).expect("preset");
                assert!(picker.from.enabled && !picker.to.enabled);
                let params = picker.params_at(applied).expect("params");
                let from = fm_preview::log::parse_bound(params["from"].as_str().expect("from"))
                    .expect("parsed");
                assert_eq!(params["to"], "");
                assert_eq!(applied.as_second() - from, seconds);
            }
        }
    }

    #[test]
    fn quick_ranges_stage_until_apply_and_custom_edits_override_them() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 650.0))
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, state: &mut (TimePicker, Vec<Action>)| {
                    state.0.show(ui.ctx(), 1, &Tokens::dark(), &mut state.1);
                },
                (fixture(), Vec::new()),
            );
        harness.run();
        harness.get_by_label("Last 1 hour").click();
        harness.run();
        assert_eq!(harness.state().0.relative, Some(3600));
        assert!(harness.state().0.open && harness.state().1.is_empty());
        harness.get_by_label("From date").click();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run();
        harness.get_by_label("From date").type_text("2024-02-29");
        harness.run();
        assert_eq!(harness.state().0.relative, None);
        assert_eq!(harness.state().0.from.date, "2024-02-29");
        for (label, seconds) in [("Last 5 minutes", 300), ("Last 15 minutes", 900)] {
            harness.get_by_label(label).click();
            harness.run();
            assert_eq!(harness.state().0.relative, Some(seconds));
            assert!(!harness.state().0.to.enabled);
            assert!(harness.state().1.is_empty());
        }
        harness.run();
        let before = Timestamp::now().as_second();
        harness.get_by_label("Apply").click();
        harness.run();
        let after = Timestamp::now().as_second();
        let (_, params) = harness.state().1.first().expect("Apply dispatch");
        let from =
            fm_preview::log::parse_bound(params["from"].as_str().expect("from")).expect("parsed");
        assert_eq!(params["to"], "");
        assert!((before - 900..=after - 900).contains(&from));
        assert!(!harness.state().0.open);
    }

    #[test]
    fn title_bar_close_does_not_apply_time_edits() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut harness = Harness::builder()
                .with_size(egui::vec2(900.0, 750.0))
                .with_accessibility_check(false)
                .build_ui_state(
                    |ui, (picker, actions): &mut (TimePicker, Vec<Action>)| {
                        if picker.open {
                            picker.show(ui.ctx(), 1, &tokens, actions);
                        }
                    },
                    (fixture(), Vec::new()),
                );
            harness.get_by_label("Last 1 hour").click();
            harness.run();
            harness.get_by_label("Close window").click();
            harness.run();
            assert!(!harness.state().0.open);
            assert!(harness.state().1.is_empty());
        }
    }

    #[test]
    fn time_window_keeps_its_position_when_the_viewport_grows() {
        let ctx = Context::default();
        let mut picker = fixture();
        let mut previous = None;
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
            if let Some(previous) = previous {
                assert!(
                    window.center().distance(previous) < 2.0,
                    "dialog keeps its position"
                );
            } else {
                assert!(
                    window.center().distance(rect.center()) < 2.0,
                    "initially centered: {window:?}, viewport {rect:?}"
                );
            }
            previous = Some(window.center());
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
            harness.run_steps(4);
            harness.get_by_label("From time").click();
            harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            harness.run();
            harness.get_by_label("From time").type_text("12:00");
            harness.key_press(egui::Key::Enter);
            harness.run();
            assert!(harness.state().0.open);
            assert_eq!(harness.state().0.from.time, "12:00");
            assert!(harness.state().1.is_empty());
            harness
                .get_all_by_label("Calendar")
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
        picker.to.time = "09:00".into();
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
