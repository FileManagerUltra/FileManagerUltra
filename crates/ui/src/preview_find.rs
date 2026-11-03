//! Find controls and render-time highlighting over engine-produced match locations.

use std::{ops::Range, sync::Arc};

use egui::{Id, Ui, text::LayoutJob};
use fm_engine::{
    Session,
    preview_find_cmds::{MAX_QUERY, Matches, searchable},
};
use fm_layout::PaneId;
use serde_json::json;

use super::{Action, Body};
use crate::theme::Tokens;

#[derive(Clone)]
struct Frame {
    matches: Arc<Matches>,
    current: usize,
    segment: usize,
    reveal: bool,
    revealed: bool,
}

fn frame_id() -> Id {
    Id::NULL.with("pv-find-frame")
}

pub(super) fn begin(ui: &Ui, s: &Session, pane: PaneId) {
    let find = s.preview_slot(pane).map(|slot| &slot.find);
    ui.data_mut(|d| {
        d.remove::<Frame>(frame_id());
        if let Some(find) = find.filter(|find| find.open)
            && let Some(matches) = &find.matches
        {
            let last = d.get_temp::<u64>(Id::NULL.with(("pv-find-reveal", pane)));
            d.insert_temp(
                frame_id(),
                Frame {
                    matches: matches.clone(),
                    current: find.current,
                    segment: 0,
                    reveal: last != Some(find.reveal),
                    revealed: false,
                },
            );
        }
    });
}

pub(super) fn end(ui: &Ui, s: &Session, pane: PaneId) {
    ui.data_mut(|d| {
        if d.get_temp::<Frame>(frame_id())
            .is_some_and(|frame| frame.revealed)
            && let Some(slot) = s.preview_slot(pane)
        {
            d.insert_temp(Id::NULL.with(("pv-find-reveal", pane)), slot.find.reveal);
        }
        d.remove::<Frame>(frame_id());
    });
}

pub(super) fn bar(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(slot) = s.preview_slot(pane) else {
        return;
    };
    if !matches!(&slot.body, Body::Static(p) if searchable(p)) {
        return;
    }
    let find = &slot.find;
    if !find.open {
        return;
    }
    let mut options = find.options.clone();
    ui.horizontal_wrapped(|ui| {
        let query_id = Id::NULL.with(("pv-find-query", pane));
        if ui.memory(|m| m.has_focus(query_id)) {
            for (modifiers, id) in [
                (egui::Modifiers::SHIFT, "preview.find.previous"),
                (egui::Modifiers::NONE, "preview.find.next"),
            ] {
                if ui.input_mut(|i| i.consume_key(modifiers, egui::Key::Enter)) {
                    actions.push((id, json!({})));
                }
            }
        }
        let response = ui.add(
            egui::TextEdit::singleline(&mut options.query)
                .id(query_id)
                .hint_text("Find in preview")
                .desired_width(160.0)
                .char_limit(MAX_QUERY),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Find in preview")
        });
        let focus_id = Id::NULL.with(("pv-find-focus", pane));
        if ui.data(|d| d.get_temp::<u64>(focus_id)) != Some(find.focus) {
            response.request_focus();
            ui.data_mut(|d| d.insert_temp(focus_id, find.focus));
        }
        let count = find.matches.as_ref().map_or(0, |m| m.hits.len());
        let status = if find.busy {
            "Searching...".into()
        } else if options.query.is_empty() {
            "Loaded content only".into()
        } else if count == 0 {
            "No matches".into()
        } else {
            format!(
                "{} / {}{}",
                find.current + 1,
                count,
                if find.matches.as_ref().is_some_and(|m| m.capped) {
                    "+"
                } else {
                    ""
                }
            )
        };
        ui.label(tokens.muted(status));
        for (label, id) in [
            ("Previous", "preview.find.previous"),
            ("Next", "preview.find.next"),
        ] {
            if ui
                .add_enabled(count > 0, egui::Button::new(label))
                .clicked()
            {
                actions.push((id, json!({})));
            }
        }
        ui.checkbox(&mut options.case_sensitive, "Aa")
            .on_hover_text("Match case");
        ui.checkbox(&mut options.whole_word, "Word")
            .on_hover_text("Whole words only (Unicode word boundaries)");
        ui.checkbox(&mut options.regex, ".*")
            .on_hover_text("Use a regular expression");
        if ui
            .button("Close")
            .on_hover_text("Close find (Escape)")
            .clicked()
        {
            actions.push(("preview.find.close", json!({})));
            response.surrender_focus();
        }
    });
    if options != find.options {
        actions.push((
            "preview.find.set",
            json!({
                "query": options.query, "case_sensitive": options.case_sensitive,
                "whole_word": options.whole_word, "regex": options.regex,
            }),
        ));
    }
    if let Some(error) = &find.error {
        ui.colored_label(tokens.error, error);
    }
}

pub(super) fn set_segment(ui: &Ui, segment: usize) {
    ui.data_mut(|d| {
        if let Some(mut frame) = d.get_temp::<Frame>(frame_id()) {
            frame.segment = segment;
            d.insert_temp(frame_id(), frame);
        }
    });
}

pub(super) fn target_line(ui: &Ui) -> Option<usize> {
    ui.data(|d| d.get_temp::<Frame>(frame_id()))
        .and_then(|frame| {
            frame
                .reveal
                .then(|| frame.matches.hits.get(frame.current).map(|hit| hit.segment))
                .flatten()
        })
}

/// Split syntax sections only at hit boundaries, preserving all other formatting.
pub(super) fn format(
    ui: &Ui,
    tokens: &Tokens,
    mut job: LayoutJob,
) -> (LayoutJob, Option<Range<usize>>) {
    let Some(mut frame) = ui.data(|d| d.get_temp::<Frame>(frame_id())) else {
        return (job, None);
    };
    let start = frame
        .matches
        .hits
        .partition_point(|hit| hit.segment < frame.segment);
    let end = frame
        .matches
        .hits
        .partition_point(|hit| hit.segment <= frame.segment);
    let active = frame
        .matches
        .hits
        .get(frame.current)
        .filter(|hit| hit.segment == frame.segment);
    let reveal = if frame.reveal {
        active.map(|hit| hit.bytes.clone())
    } else {
        None
    };
    let hits = frame.matches.hits.get(start..end).unwrap_or_default();
    if !hits.is_empty() {
        let mut sections = Vec::new();
        for section in &job.sections {
            let mut cursor = section.byte_range.start;
            let first = hits.partition_point(|hit| hit.bytes.end <= section.byte_range.start.0);
            for hit in hits
                .iter()
                .skip(first)
                .take_while(|hit| hit.bytes.start < section.byte_range.end.0)
            {
                let from = egui::text::ByteIndex(hit.bytes.start).max(section.byte_range.start);
                let to = egui::text::ByteIndex(hit.bytes.end).min(section.byte_range.end);
                if from >= to {
                    continue;
                }
                if cursor < from {
                    let mut before = section.clone();
                    before.byte_range = cursor..from;
                    sections.push(before);
                }
                let mut highlighted = section.clone();
                highlighted.byte_range = from..to;
                let current = active.is_some_and(|active| active.bytes == hit.bytes);
                highlighted.format.background = if current {
                    tokens.accent
                } else {
                    tokens.selection_bg
                };
                highlighted.format.color = if current {
                    tokens.bg
                } else {
                    tokens.selection_text
                };
                sections.push(highlighted);
                cursor = to;
            }
            if cursor < section.byte_range.end {
                let mut after = section.clone();
                after.byte_range = cursor..section.byte_range.end;
                sections.push(after);
            }
        }
        job.sections = sections;
    }
    frame.segment += 1;
    ui.data_mut(|d| d.insert_temp(frame_id(), frame));
    (job, reveal)
}

pub(super) fn reveal(
    ui: &Ui,
    rect: egui::Rect,
    galley: &egui::Galley,
    active: Option<Range<usize>>,
) {
    let Some(active) = active else { return };
    let index = galley
        .job
        .text
        .get(..active.start)
        .map_or(0, |text| text.chars().count());
    let cursor = egui::text::CCursor::new(index);
    let target = galley.pos_from_cursor(cursor).translate(rect.min.to_vec2());
    ui.scroll_to_rect(target, Some(egui::Align::Center));
    ui.data_mut(|d| {
        if let Some(mut frame) = d.get_temp::<Frame>(frame_id()) {
            frame.revealed = true;
            d.insert_temp(frame_id(), frame);
        }
    });
}

pub(super) fn label(ui: &mut Ui, tokens: &Tokens, job: LayoutJob, wrap: bool) -> egui::Response {
    label_with_galley(ui, tokens, job, wrap).0
}

pub(super) fn label_with_galley(
    ui: &mut Ui,
    tokens: &Tokens,
    job: LayoutJob,
    wrap: bool,
) -> (egui::Response, Arc<egui::Galley>) {
    let (mut job, active) = format(ui, tokens, job);
    job.wrap.max_width = if wrap {
        ui.available_width()
    } else {
        f32::INFINITY
    };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let label = egui::Label::new(galley.clone());
    let response = ui.add(if wrap { label.wrap() } else { label.extend() });
    reveal(ui, response.rect, &galley, active);
    (response, galley)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_engine::preview_find_cmds::Hit;

    #[test]
    fn highlighting_preserves_unicode_syntax_sections_and_plain_text() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let mut job = LayoutJob::default();
                for text in ["é ne", "ed", "le tail"] {
                    job.append(
                        text,
                        0.0,
                        egui::TextFormat::simple(egui::FontId::monospace(14.0), tokens.text),
                    );
                }
                ui.data_mut(|d| {
                    d.insert_temp(
                        frame_id(),
                        Frame {
                            matches: Arc::new(Matches {
                                hits: vec![Hit {
                                    segment: 0,
                                    bytes: 3..9,
                                }],
                                capped: false,
                            }),
                            current: 0,
                            segment: 0,
                            reveal: true,
                            revealed: false,
                        },
                    )
                });
                let original = job.text.clone();
                let (job, active) = format(ui, &tokens, job);
                assert_eq!(job.text, original);
                assert_eq!(active, Some(3..9));
                let highlighted: String = job
                    .sections
                    .iter()
                    .filter(|section| section.format.background == tokens.accent)
                    .map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0])
                    .collect();
                assert_eq!(highlighted, "needle");
                let combined: String = job
                    .sections
                    .iter()
                    .map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0])
                    .collect();
                assert_eq!(combined, original);
            });
            output.textures_delta.clear();
        }
    }
}
