//! Read-only document layout over the worker-produced Markdown tree.

use egui::text::LayoutJob;
use egui::{Align, FontId, Id, Layout, RichText, ScrollArea, Stroke, TextFormat, Ui, vec2};
use fm_layout::PaneId;
use fm_preview::markdown::{Alignment, CodeBlock, Inline, MdBlock, MdDoc, Table};

use super::{find, paged_scroll, remember_scroll, selection_job};
use crate::{markdown_fonts, theme::Tokens};

fn inline_job(ui: &Ui, tokens: &Tokens, parts: &[Inline], size: f32, strong: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    for part in parts {
        let font = if part.code {
            FontId::monospace(size * 0.9)
        } else {
            markdown_fonts::font(ui.ctx(), size, strong || part.bold)
        };
        let color = if part.link.is_some() {
            tokens.accent_text
        } else {
            tokens.text
        };
        let mut format = TextFormat::simple(font, color);
        format.italics = part.italic;
        format.line_height = Some(size * 1.55);
        if part.strike {
            format.strikethrough = Stroke::new(1.0, color);
        }
        if part.link.is_some() {
            format.underline = Stroke::new(1.0, color);
        }
        if part.code {
            format.background = tokens.hover_bg;
        }
        job.append(&part.text, 0.0, format);
    }
    selection_job(ui, tokens, job)
}

/// The link under `pointer`, found from the laid-out glyphs so wrapped and mixed-format text
/// hit-tests exactly. `origin` is where the galley's rect starts on screen.
fn link_at<'a>(
    parts: &'a [Inline],
    galley: &egui::Galley,
    origin: egui::Pos2,
    pointer: egui::Pos2,
) -> Option<&'a str> {
    let local = pointer - origin;
    let mut index = 0usize;
    for placed in &galley.rows {
        let row_hit = local.y >= placed.pos.y && local.y < placed.pos.y + placed.row.height();
        for glyph in &placed.row.glyphs {
            let left = placed.pos.x + glyph.pos.x;
            if row_hit && local.x >= left && local.x < left + glyph.advance_width {
                return link_of(parts, index);
            }
            index += 1;
        }
        if placed.ends_with_newline {
            index += 1;
        }
    }
    None
}

fn link_of(parts: &[Inline], mut index: usize) -> Option<&str> {
    for part in parts {
        let len = part.text.chars().count();
        if index < len {
            return part.link.as_deref();
        }
        index -= len;
    }
    None
}

/// Pointing-hand cursor, tooltip and click handling for the link under the pointer.
fn follow_link(
    response: &egui::Response,
    galley: &egui::Galley,
    parts: &[Inline],
    actions: &mut CodeActions,
) {
    let Some(pointer) = response.hover_pos() else {
        return;
    };
    let origin = response.rect.min - galley.rect.min.to_vec2();
    let Some(url) = link_at(parts, galley, origin, pointer) else {
        return;
    };
    response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    egui::Tooltip::always_open(
        response.ctx.clone(),
        response.layer_id,
        response.id.with("link"),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(url);
    });
    if response.clicked() {
        actions.link = Some(url.to_string());
    }
}

fn inline(
    ui: &mut Ui,
    tokens: &Tokens,
    parts: &[Inline],
    size: f32,
    strong: bool,
    actions: &mut CodeActions,
) {
    let (response, galley) = find::label_with_galley(
        ui,
        tokens,
        inline_job(ui, tokens, parts, size, strong),
        true,
    );
    follow_link(&response, &galley, parts, actions);
}

fn code(ui: &mut Ui, tokens: &Tokens, code: &CodeBlock, wrap: bool, path: Id) -> bool {
    let mut copied = false;
    egui::Frame::NONE
        .fill(tokens.panel)
        .stroke(Stroke::new(tokens.border_width, tokens.border))
        .corner_radius(tokens.corner_radius)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width((ui.available_width()).max(1.0));
            let edge = tokens.font_size * 1.5;
            let (header, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), edge), egui::Sense::hover());
            if !code.language.is_empty() {
                ui.painter()
                    .with_clip_rect(
                        egui::Rect::from_min_max(header.min, header.max - vec2(edge + 6.0, 0.0))
                            .intersect(ui.clip_rect()),
                    )
                    .text(
                        header.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &code.language,
                        markdown_fonts::font(ui.ctx(), tokens.font_size * 0.8, true),
                        tokens.text_muted,
                    );
            }
            let rect =
                egui::Rect::from_min_size(header.right_top() - vec2(edge, 0.0), vec2(edge, edge));
            let response = ui.interact(rect, path.with("copy-code"), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, response.enabled(), "Copy code block")
            });
            let mut painter = ui.painter().clone();
            painter.multiply_opacity(if response.hovered() || response.has_focus() {
                1.0
            } else {
                0.5
            });
            painter.rect_filled(rect, tokens.corner_radius, tokens.hover_bg);
            let icon = rect.shrink(edge * 0.22);
            let shift = edge * 0.16;
            let stroke = Stroke::new(tokens.border_width.max(1.0), tokens.text);
            painter.rect_stroke(
                icon.translate(vec2(-shift * 0.4, -shift * 0.4))
                    .shrink(shift * 0.5),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect(
                icon.translate(vec2(shift * 0.4, shift * 0.4))
                    .shrink(shift * 0.5),
                1.0,
                tokens.hover_bg,
                stroke,
                egui::StrokeKind::Inside,
            );
            copied = response.clicked();
            response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Copy code block");
            ui.add_space(4.0);
            let mut job = LayoutJob::default();
            let font = FontId::monospace(tokens.font_size * 0.88);
            for (n, line) in code.text.split('\n').enumerate() {
                if n != 0 {
                    job.append("\n", 0.0, TextFormat::simple(font.clone(), tokens.text));
                }
                let highlighted = code.lines.get(n).filter(|spans| {
                    spans.iter().map(|s| s.text.len()).sum::<usize>() == line.len()
                        && spans.iter().map(|s| s.text.as_str()).collect::<String>() == line
                });
                if let Some(spans) = highlighted {
                    for span in spans {
                        let mut format =
                            TextFormat::simple(font.clone(), tokens.syntax(span.class));
                        format.line_height = Some(tokens.font_size * 1.5);
                        job.append(&span.text, 0.0, format);
                    }
                } else {
                    job.append(line, 0.0, TextFormat::simple(font.clone(), tokens.text));
                }
            }
            let job = selection_job(ui, tokens, job);
            if wrap {
                find::label(ui, tokens, job, true);
            } else {
                ScrollArea::horizontal()
                    .id_salt(path.with("code"))
                    .show(ui, |ui| {
                        find::label(ui, tokens, job, false);
                    });
            }
        });
    copied
}

#[derive(Default)]
struct CodeActions {
    next: usize,
    clicked: Option<usize>,
    link: Option<String>,
}

fn table(ui: &mut Ui, tokens: &Tokens, table: &Table, path: Id, actions: &mut CodeActions) {
    let columns = table.alignments.len().max(1);
    let padding = tokens.font_size * 0.5;
    let width = ((ui.available_width() - 2.0) / columns as f32 - padding * 2.0)
        .clamp(tokens.font_size * 9.0, tokens.font_size * 22.0);
    let cell_width = width + padding * 2.0;
    ScrollArea::horizontal()
        .id_salt(path.with("table"))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (n, row) in std::iter::once(&table.header)
                .chain(table.rows.iter())
                .enumerate()
            {
                let cells: Vec<_> = (0..columns)
                    .map(|col| {
                        let mut job = inline_job(
                            ui,
                            tokens,
                            row.get(col).map_or(&[], Vec::as_slice),
                            tokens.font_size,
                            n == 0,
                        );
                        job.wrap.max_width = width;
                        job.justify = false;
                        job.halign = match table.alignments.get(col) {
                            Some(Alignment::Center) => Align::Center,
                            Some(Alignment::Right) => Align::RIGHT,
                            _ => Align::LEFT,
                        };
                        let (job, active) = find::format(ui, tokens, job);
                        (ui.fonts_mut(|f| f.layout_job(job)), active)
                    })
                    .collect();
                let height = cells
                    .iter()
                    .map(|(g, _)| g.size().y)
                    .fold(tokens.font_size * 1.55, f32::max)
                    + padding * 2.0;
                let (rect, _) = ui.allocate_exact_size(
                    vec2(cell_width * columns as f32, height),
                    egui::Sense::hover(),
                );
                let stroke = Stroke::new(tokens.border_width, tokens.border);
                ui.painter().rect(
                    rect,
                    0.0,
                    if n == 0 {
                        tokens.hover_bg
                    } else {
                        tokens.panel
                    },
                    stroke,
                    egui::StrokeKind::Inside,
                );
                for (col, (galley, active)) in cells.into_iter().enumerate() {
                    let left = rect.left() + col as f32 * cell_width;
                    if col > 0 {
                        ui.painter().vline(left, rect.y_range(), stroke);
                    }
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(left + padding, rect.top() + padding),
                        vec2(width, height - padding * 2.0),
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt((path, n, col))
                            .max_rect(cell)
                            .layout(Layout::top_down(galley.job.halign)),
                    );
                    child.set_clip_rect(cell.intersect(ui.clip_rect()));
                    let response = child.add(egui::Label::new(galley.clone()).selectable(true));
                    find::reveal(&child, response.rect, &galley, active);
                    let parts = if n == 0 {
                        &table.header
                    } else {
                        &table.rows[n - 1]
                    };
                    follow_link(
                        &response,
                        &galley,
                        parts.get(col).map_or(&[], Vec::as_slice),
                        actions,
                    );
                }
            }
        });
}

fn blocks(
    ui: &mut Ui,
    tokens: &Tokens,
    doc: &[MdBlock],
    wrap: bool,
    path: Id,
    actions: &mut CodeActions,
) {
    for (n, block) in doc.iter().enumerate() {
        let path = path.with(n);
        match block {
            MdBlock::Heading(level, parts) => {
                ui.add_space(tokens.font_size * if n == 0 { 0.25 } else { 0.9 });
                let scale = match level {
                    1 => 2.0,
                    2 => 1.6,
                    3 => 1.3,
                    _ => 1.1,
                };
                inline(ui, tokens, parts, tokens.font_size * scale, true, actions);
                ui.add_space(tokens.font_size * 0.25);
            }
            MdBlock::Para(parts) => {
                inline(ui, tokens, parts, tokens.font_size, false, actions);
                ui.add_space(tokens.font_size * 0.4);
            }
            MdBlock::Code(c) => {
                if code(ui, tokens, c, wrap, path) {
                    actions.clicked = Some(actions.next);
                }
                actions.next += 1;
                ui.add_space(tokens.font_size * 0.5);
            }
            MdBlock::List { start, items } => {
                for (i, item) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        let marker = start.map_or_else(
                            || "•".into(),
                            |start| format!("{}.", start.saturating_add(i as u64)),
                        );
                        let marker_width = (marker.len() as f32 * tokens.font_size * 0.65)
                            .max(tokens.font_size * 1.4);
                        if let Some(checked) = item.checked {
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(marker_width, tokens.font_size * 1.55),
                                egui::Sense::hover(),
                            );
                            let rect = egui::Rect::from_center_size(
                                rect.center(),
                                vec2(tokens.font_size * 0.85, tokens.font_size * 0.85),
                            );
                            ui.painter().rect(
                                rect,
                                2.0,
                                if checked { tokens.accent } else { tokens.panel },
                                Stroke::new(tokens.border_width, tokens.border),
                                egui::StrokeKind::Inside,
                            );
                            if checked {
                                let points = vec![
                                    rect.min + rect.size() * vec2(0.2, 0.5),
                                    rect.min + rect.size() * vec2(0.42, 0.72),
                                    rect.min + rect.size() * vec2(0.82, 0.25),
                                ];
                                ui.painter().add(egui::Shape::line(
                                    points,
                                    Stroke::new(1.5, tokens.selection_text),
                                ));
                            }
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::Role::CheckBox,
                                    false,
                                    checked,
                                    "Read-only task",
                                )
                            });
                        } else {
                            ui.add_sized(
                                vec2(marker_width, tokens.font_size * 1.55),
                                egui::Label::new(RichText::new(marker).font(markdown_fonts::font(
                                    ui.ctx(),
                                    tokens.font_size,
                                    false,
                                ))),
                            );
                        }
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width().max(1.0));
                            blocks(ui, tokens, &item.blocks, wrap, path.with(i), actions);
                        });
                    });
                }
                ui.add_space(tokens.font_size * 0.3);
            }
            MdBlock::Quote(children) => {
                let out = egui::Frame::NONE
                    .fill(tokens.hover_bg)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width().max(1.0));
                        blocks(ui, tokens, children, wrap, path, actions);
                    });
                ui.painter().line_segment(
                    [
                        out.response.rect.left_top(),
                        out.response.rect.left_bottom(),
                    ],
                    Stroke::new(3.0, tokens.accent),
                );
                ui.add_space(tokens.font_size * 0.5);
            }
            MdBlock::Table(t) => {
                table(ui, tokens, t, path, actions);
                ui.add_space(tokens.font_size * 0.6);
            }
            MdBlock::Rule => {
                ui.add_space(tokens.font_size * 0.5);
                ui.separator();
                ui.add_space(tokens.font_size * 0.5);
            }
        }
    }
}

pub(super) fn show(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    doc: &MdDoc,
    wrap: bool,
    link: &mut Option<String>,
) -> Option<usize> {
    let mut actions = CodeActions::default();
    markdown_fonts::install(ui.ctx());
    let mut document_tokens = *tokens;
    document_tokens.font_size *= 1.14;
    let tokens = &document_tokens;
    let output = paged_scroll(ui, ScrollArea::vertical(), ("pv-md", pane))
        .auto_shrink(false)
        .show(ui, |ui| {
            let padding = (tokens.font_size * 1.2).min(ui.available_width() * 0.06);
            egui::Frame::NONE.inner_margin(padding).show(ui, |ui| {
                let width = ui.available_width().min(tokens.font_size * 52.0).max(1.0);
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    ui.allocate_ui_with_layout(
                        vec2(width, 0.0),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            ui.set_width(width);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            blocks(
                                ui,
                                tokens,
                                &doc.blocks,
                                wrap,
                                ui.make_persistent_id(("md-doc", pane)),
                                &mut actions,
                            );
                            if doc.truncated {
                                ui.label(tokens.muted("… truncated"));
                            }
                        },
                    );
                });
            });
        });
    remember_scroll(ui, &output);
    *link = actions.link;
    actions.clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_button_is_translucent_until_hovered_and_stays_inside_the_box() {
        use egui_kittest::{Harness, kittest::Queryable};
        for tokens in [Tokens::dark(), Tokens::light()] {
            for language in ["", "rust"] {
                let block = CodeBlock {
                    language: language.into(),
                    text: "content".into(),
                    lines: Vec::new(),
                };
                let mut harness =
                    Harness::builder()
                        .with_size(vec2(320.0, 180.0))
                        .build_ui(move |ui| {
                            crate::theme::apply_to(ui.ctx(), &tokens);
                            code(ui, &tokens, &block, false, Id::NULL.with("test-code"));
                        });
                harness.run();
                let rect = harness.get_by_label("Copy code block").rect();
                assert!(rect.right() <= 320.0 && rect.top() >= 0.0);
                assert!(rect.left() > 250.0, "button belongs at the top right");
                let fill = |harness: &Harness<'_, ()>| {
                    harness
                        .output()
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Rect(shape) if shape.rect == rect => Some(shape.fill),
                            _ => None,
                        })
                        .expect("button background")
                };
                let idle = fill(&harness);
                harness
                    .input_mut()
                    .events
                    .push(egui::Event::PointerMoved(rect.center()));
                harness.run();
                let hovered = fill(&harness);
                assert!(idle.a() < hovered.a());
                assert_eq!(hovered, tokens.hover_bg);
            }
        }
    }

    #[test]
    fn inline_formats_keep_bold_italic_strike_links_and_selection() {
        let ctx = egui::Context::default();
        markdown_fonts::install(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let parts = vec![Inline {
                text: "styled".into(),
                bold: true,
                italic: true,
                strike: true,
                code: false,
                link: Some("https://example.org".into()),
            }];
            let job = inline_job(ui, &Tokens::light(), &parts, 16.0, false);
            let format = &job.sections[0].format;
            assert_ne!(format.font_id.family, egui::FontFamily::Proportional);
            assert!(format.italics);
            assert!(format.strikethrough.width > 0.0 && format.underline.width > 0.0);
            assert_eq!(format.line_height, Some(24.8));
            ui.data_mut(|d| d.insert_temp(Id::NULL.with("pv-all-selected"), true));
            let selected = inline_job(ui, &Tokens::light(), &parts, 16.0, false);
            assert_eq!(
                selected.sections[0].format.background,
                Tokens::light().selection_bg
            );
        });
        out.textures_delta.clear();
    }

    #[test]
    fn clicking_a_link_reports_its_url_and_plain_text_does_not() {
        use egui_kittest::{Harness, kittest::Queryable};
        let doc = MdDoc {
            blocks: fm_preview::markdown::parse(
                "plain words and [the link](https://example.org/x) end\n\n| a | b |\n| - | - |\n| [cell](mailto:a@b.c) | x |",
            ),
            ..Default::default()
        };
        let tokens = Tokens::dark();
        let mut harness = Harness::builder()
            .with_size(vec2(600.0, 300.0))
            .build_ui_state(
                move |ui, got: &mut Vec<String>| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    let mut link = None;
                    show(ui, 1, &tokens, &doc, true, &mut link);
                    got.extend(link);
                },
                Vec::new(),
            );
        harness.run();
        let click = |harness: &mut Harness<'_, Vec<String>>, pos: egui::Pos2| {
            let events = &mut harness.input_mut().events;
            events.push(egui::Event::PointerMoved(pos));
            for pressed in [true, false] {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            harness.run();
        };
        let para = harness
            .query_all_by_label_contains("plain words")
            .next()
            .expect("paragraph")
            .rect();
        click(&mut harness, para.left_center() + vec2(6.0, 0.0));
        assert!(harness.state().is_empty(), "plain text is not a link");
        click(&mut harness, para.left_center() + vec2(130.0, 0.0));
        assert_eq!(harness.state(), &["https://example.org/x".to_string()]);
        let cell = harness.get_by_label("cell").rect();
        click(&mut harness, cell.center());
        assert_eq!(
            harness.state().last().map(String::as_str),
            Some("mailto:a@b.c")
        );
    }

    #[test]
    fn link_lookup_follows_character_order_across_parts() {
        let parts = vec![
            Inline {
                text: "ab ".into(),
                ..Default::default()
            },
            Inline {
                text: "é\nlink".into(),
                link: Some("https://x.y".into()),
                ..Default::default()
            },
            Inline {
                text: " z".into(),
                ..Default::default()
            },
        ];
        assert_eq!(link_of(&parts, 2), None);
        assert_eq!(link_of(&parts, 3), Some("https://x.y"));
        assert_eq!(link_of(&parts, 8), Some("https://x.y"));
        assert_eq!(link_of(&parts, 9), None);
        assert_eq!(link_of(&parts, 100), None);
    }
}
