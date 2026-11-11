//! Preferences > Subtitles: appearance for readability, with a live preview and a
//! contrast check. Every change dispatches `media.subtitle.style`.

use egui::{Id, Ui, vec2};
use serde_json::{Value, json};

use fm_engine::Session;
use fm_engine::subtitle_style::{
    EDGE_WIDTH_RANGE, LETTER_SPACING_RANGE, LINE_SPACING_RANGE, OPACITY_RANGE, POSITION_RANGE,
    PRESETS, SIZE_RANGE, SubtitleEdge, SubtitleFont, SubtitleStyle, TEXT_OPACITY_RANGE,
};

use crate::dialog_style::hint;
use crate::theme::Tokens;

/// Colours offered as swatches; any `#rrggbb` can be typed as well.
const SWATCHES: [(&str, &str); 8] = [
    ("White", "#ffffff"),
    ("Yellow", "#ffff00"),
    ("Cyan", "#00ffff"),
    ("Green", "#00ff00"),
    ("Magenta", "#ff00ff"),
    ("Red", "#ff4040"),
    ("Blue", "#4080ff"),
    ("Black", "#000000"),
];

const SAMPLE: &str = "The quick brown fox jumps\nover the lazy dog.";

fn style(s: &mut Session, fields: Value) {
    s.dispatch_or_notice("media.subtitle.style", &fields);
}

pub(super) fn show(ui: &mut Ui, s: &mut Session, tokens: &Tokens) {
    let current = s.prefs.subtitle_style.clone();
    hint(
        ui,
        tokens,
        "Applies to every subtitle format: SRT, WebVTT, subtitles embedded in the video and \
         ASS/SSA, so subtitles always look the same. Styling inside ASS/SSA files is replaced \
         by these settings. Picture-based subtitles (PGS, DVD) cannot be restyled.",
    );
    ui.add_space(6.0);
    preview(ui, &current, tokens);
    ui.add_space(8.0);

    super::section(ui, tokens, "Presets", None, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (name, label) in PRESETS {
                if ui.button(label).clicked() {
                    style(s, json!({"preset": name}));
                }
            }
        });
    });

    super::section(ui, tokens, "Text", None, |ui| {
        slider(ui, s, tokens, "Size", "size", current.size, SIZE_RANGE, "%");
        ui.horizontal(|ui| {
            let caption = caption(ui, "Font");
            let mut font = current.font;
            egui::ComboBox::from_id_salt("fm-subtitle-font")
                .selected_text(match font {
                    SubtitleFont::Sans => "Sans-serif",
                    SubtitleFont::Mono => "Monospace",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut font, SubtitleFont::Sans, "Sans-serif");
                    ui.selectable_value(&mut font, SubtitleFont::Mono, "Monospace");
                })
                .response
                .labelled_by(caption.id);
            if font != current.font {
                let name = if font == SubtitleFont::Mono {
                    "mono"
                } else {
                    "sans"
                };
                style(s, json!({"font": name}));
            }
            let mut bold = current.bold;
            if ui.checkbox(&mut bold, "Bold").changed() {
                style(s, json!({"bold": bold}));
            }
        });
        color_row(ui, s, "Colour", "text_color", &current.text_color, true);
        slider(
            ui,
            s,
            tokens,
            "Opacity",
            "text_opacity",
            current.text_opacity,
            TEXT_OPACITY_RANGE,
            "%",
        );
        slider(
            ui,
            s,
            tokens,
            "Letter spacing",
            "letter_spacing",
            current.letter_spacing,
            LETTER_SPACING_RANGE,
            "%",
        );
        slider(
            ui,
            s,
            tokens,
            "Line spacing",
            "line_spacing",
            current.line_spacing,
            LINE_SPACING_RANGE,
            "%",
        );
    });

    super::section(
        ui,
        tokens,
        "Background",
        Some("A solid box behind the text gives the most contrast. 0% turns it off."),
        |ui| {
            color_row(
                ui,
                s,
                "Colour",
                "background_color",
                &current.background_color,
                false,
            );
            slider(
                ui,
                s,
                tokens,
                "Opacity",
                "background_opacity",
                current.background_opacity,
                OPACITY_RANGE,
                "%",
            );
        },
    );

    super::section(ui, tokens, "Edge", None, |ui| {
        ui.horizontal(|ui| {
            let caption = caption(ui, "Style");
            let mut edge = current.edge;
            let name = |edge: SubtitleEdge| match edge {
                SubtitleEdge::None => "None",
                SubtitleEdge::Outline => "Outline",
                SubtitleEdge::Shadow => "Drop shadow",
                SubtitleEdge::Raised => "Raised",
                SubtitleEdge::Depressed => "Depressed",
            };
            egui::ComboBox::from_id_salt("fm-subtitle-edge")
                .selected_text(name(edge))
                .show_ui(ui, |ui| {
                    for option in [
                        SubtitleEdge::None,
                        SubtitleEdge::Outline,
                        SubtitleEdge::Shadow,
                        SubtitleEdge::Raised,
                        SubtitleEdge::Depressed,
                    ] {
                        ui.selectable_value(&mut edge, option, name(option));
                    }
                })
                .response
                .labelled_by(caption.id);
            if edge != current.edge {
                let value = match edge {
                    SubtitleEdge::None => "none",
                    SubtitleEdge::Outline => "outline",
                    SubtitleEdge::Shadow => "shadow",
                    SubtitleEdge::Raised => "raised",
                    SubtitleEdge::Depressed => "depressed",
                };
                style(s, json!({"edge": value}));
            }
        });
        ui.add_enabled_ui(current.edge != SubtitleEdge::None, |ui| {
            color_row(ui, s, "Colour", "edge_color", &current.edge_color, true);
            slider(
                ui,
                s,
                tokens,
                "Thickness",
                "edge_width",
                current.edge_width,
                EDGE_WIDTH_RANGE,
                "",
            );
        });
    });

    super::section(ui, tokens, "Position", None, |ui| {
        slider(
            ui,
            s,
            tokens,
            "Distance from bottom",
            "position",
            current.position,
            POSITION_RANGE,
            "%",
        );
    });

    if ui
        .add_enabled(
            current != SubtitleStyle::default(),
            egui::Button::new("Reset subtitle appearance"),
        )
        .clicked()
    {
        style(s, json!({"preset": "default"}));
    }
}

/// A dark and a bright sample scene with the sample text, plus the contrast verdict.
fn preview(ui: &mut Ui, current: &SubtitleStyle, tokens: &Tokens) {
    let width = ui.available_width().clamp(200.0, 640.0);
    let height = 170.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Subtitle preview"));
    let [dark, bright] = crate::theme::subtitle_sample_backdrop();
    let (left, right) = rect.split_left_right_at_fraction(0.5);
    let painter = ui.painter();
    painter.rect_filled(left, 0.0, dark);
    painter.rect_filled(right, 0.0, bright);
    // Size the text as on a 360-point-high video, a typical Media pane.
    crate::subtitles::paint_sized(painter, rect, SAMPLE, current, tokens, 360.0);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(tokens.border_width, tokens.border),
        egui::StrokeKind::Outside,
    );
    let ratio = crate::subtitles::effective_contrast(current, tokens);
    let (verdict, color) = if ratio >= 7.0 {
        ("excellent (WCAG AAA)", tokens.success)
    } else if ratio >= 4.5 {
        ("good (WCAG AA)", tokens.text_muted)
    } else {
        (
            "low: hard to read for many people. Add a background or an edge, or pick a stronger colour.",
            tokens.warning,
        )
    };
    ui.label(egui::RichText::new(format!("Contrast {ratio:.1}:1, {verdict}")).color(color));
}

/// Left-aligned label of a fixed width, so the controls of every row line up.
fn caption(ui: &mut Ui, label: &str) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        vec2(150.0, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(150.0);
            ui.label(label)
        },
    )
    .inner
}

/// A labelled slider that dispatches `key` when it changes.
#[allow(clippy::too_many_arguments)]
fn slider(
    ui: &mut Ui,
    s: &mut Session,
    tokens: &Tokens,
    label: &str,
    key: &str,
    current: u64,
    range: std::ops::RangeInclusive<u64>,
    suffix: &str,
) {
    let mut value = current.clamp(*range.start(), *range.end());
    ui.horizontal(|ui| {
        let caption = caption(ui, label);
        ui.spacing_mut().slider_width = (ui.available_width() - 90.0).clamp(120.0, 320.0);
        // The theme keeps idle widgets flat; the track needs to be visible here.
        ui.visuals_mut().widgets.inactive.bg_fill = tokens.border;
        let response = ui
            .add(
                egui::Slider::new(&mut value, range)
                    .suffix(suffix)
                    .trailing_fill(true),
            )
            .labelled_by(caption.id);
        if response.changed() && value != current {
            style(s, json!({ key: value }));
        }
    });
}

/// Swatches, an optional theme default, and a custom `#rrggbb` field.
fn color_row(
    ui: &mut Ui,
    s: &mut Session,
    label: &str,
    key: &str,
    current: &str,
    allow_default: bool,
) {
    ui.horizontal_wrapped(|ui| {
        caption(ui, label);
        for (name, hex) in SWATCHES {
            let Some(color) = crate::theme::swatch(hex) else {
                continue;
            };
            let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label} {name}"))
            });
            ui.painter().rect_filled(rect.shrink(2.0), 4.0, color);
            // White and black swatches must stay visible on either theme.
            ui.painter().rect_stroke(
                rect.shrink(2.0),
                4.0,
                egui::Stroke::new(
                    1.0,
                    ui.visuals()
                        .widgets
                        .noninteractive
                        .fg_stroke
                        .color
                        .gamma_multiply(0.5),
                ),
                egui::StrokeKind::Inside,
            );
            if current == hex || response.hovered() {
                ui.painter().rect_stroke(
                    rect,
                    5.0,
                    egui::Stroke::new(2.0, ui.visuals().text_color()),
                    egui::StrokeKind::Inside,
                );
            }
            if response.on_hover_text(name).clicked() {
                style(s, json!({ key: hex }));
            }
        }
        if allow_default
            && ui
                .add_enabled(current != "default", egui::Button::new("Default"))
                .clicked()
        {
            style(s, json!({ key: "default" }));
        }
        let id = Id::NULL.with(("fm-subtitle-hex", key));
        let shown = if current == "default" { "" } else { current };
        let mut text = ui
            .data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| shown.to_string());
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("#rrggbb")
                    .char_limit(7)
                    .desired_width(80.0),
            )
            .on_hover_text(format!("Custom {} colour", label.to_lowercase()));
        if response.changed() && fm_engine::prefs::parse_hex_color(&text).is_some() {
            style(s, json!({ key: text.to_ascii_lowercase() }));
        }
        if !response.has_focus() {
            text = shown.to_string();
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    });
}
