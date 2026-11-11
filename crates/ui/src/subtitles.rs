//! Draws subtitle cues with the user's style. The video overlay and the Preferences
//! preview both use [`paint`], so what the preview shows is what the video shows,
//! whatever the subtitle's original format.

use egui::{Align, FontFamily, FontId, Painter, Rect, Vec2, pos2, text::LayoutJob, vec2};
use fm_engine::subtitle_style::{SubtitleEdge, SubtitleFont, SubtitleStyle, contrast_ratio};

use crate::theme::{SubtitleColors, Tokens, rgb_of, subtitle_colors};

/// Font size in points for a picture of `height` points.
pub fn font_size(style: &SubtitleStyle, height: f32) -> f32 {
    (height * 0.05 * style.size_factor()).clamp(9.0, 160.0)
}

/// Paint `text` (lines separated by `\n`) centred above the bottom of `picture`.
pub fn paint(painter: &Painter, picture: Rect, text: &str, style: &SubtitleStyle, tokens: &Tokens) {
    paint_sized(painter, picture, text, style, tokens, picture.height());
}

/// [`paint`] with the font sized as if the picture were `reference_height` tall (the
/// Preferences preview shows real-size text in a smaller box).
pub fn paint_sized(
    painter: &Painter,
    picture: Rect,
    text: &str,
    style: &SubtitleStyle,
    tokens: &Tokens,
    reference_height: f32,
) {
    if text.trim().is_empty() || picture.width() < 8.0 || picture.height() < 8.0 {
        return;
    }
    let colors = subtitle_colors(style, tokens);
    let size = font_size(style, reference_height);
    let family = match style.font {
        SubtitleFont::Sans => FontFamily::Proportional,
        SubtitleFont::Mono => FontFamily::Monospace,
    };
    let mut job = LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: FontId::new(size, family),
            color: colors.text,
            extra_letter_spacing: size * style.letter_spacing_fraction(),
            line_height: Some(size * style.line_spacing_factor()),
            ..Default::default()
        },
    );
    job.wrap.max_width = picture.width() * 0.9;
    job.halign = Align::Center;
    let painter = painter.with_clip_rect(picture);
    let galley = painter.layout_job(job);
    let pad = vec2(size * 0.3, size * 0.08);
    let bottom = (picture.bottom() - picture.height() * style.position_fraction() - pad.y)
        .max(picture.top() + galley.size().y + pad.y);
    let anchor = pos2(picture.center().x, bottom);
    let origin = anchor - galley.rect.center_bottom().to_vec2();

    if colors.background.a() > 0 {
        for row in &galley.rows {
            let rect = row.rect();
            if rect.width() > 0.0 {
                painter.rect_filled(
                    rect.translate(origin.to_vec2()).expand2(pad),
                    size * 0.12,
                    colors.background,
                );
            }
        }
    }
    let unit = size * style.edge_factor() / 40.0;
    for offset in edge_offsets(style.edge, unit.max(0.75)) {
        painter.galley_with_override_text_color(origin + offset, galley.clone(), colors.edge);
    }
    if style.bold {
        // Faux bold: the bundled fonts have no bold weight.
        let b = (size / 30.0).max(0.6);
        for dx in [-b * 0.5, b * 0.5] {
            painter.galley_with_override_text_color(
                origin + vec2(dx, 0.0),
                galley.clone(),
                colors.text,
            );
        }
    }
    painter.galley(origin, galley, colors.text);
}

/// Where copies of the text in the edge colour go, for each edge style.
fn edge_offsets(edge: SubtitleEdge, unit: f32) -> Vec<Vec2> {
    match edge {
        SubtitleEdge::None => Vec::new(),
        SubtitleEdge::Outline => {
            // Two rings keep thick outlines round instead of square.
            let mut offsets = Vec::new();
            for ring in [unit, unit * 0.5] {
                for step in 0..16 {
                    let angle = step as f32 * std::f32::consts::TAU / 16.0;
                    offsets.push(vec2(angle.cos(), angle.sin()) * ring);
                }
            }
            offsets
        }
        SubtitleEdge::Shadow => (1..=4)
            .map(|k| vec2(1.0, 1.0) * unit * 0.6 * k as f32)
            .collect(),
        SubtitleEdge::Raised => vec![vec2(-1.0, -1.0) * unit * 0.6, vec2(1.0, 1.0) * unit * 0.4],
        SubtitleEdge::Depressed => vec![vec2(1.0, 1.0) * unit * 0.6, vec2(-1.0, -1.0) * unit * 0.4],
    }
}

/// Contrast the reader actually gets: against the box when it is mostly opaque, else
/// against the edge, else against the worst case of a dark or bright scene.
pub fn effective_contrast(style: &SubtitleStyle, tokens: &Tokens) -> f64 {
    let SubtitleColors {
        text,
        background,
        edge,
    } = subtitle_colors(style, tokens);
    let text = rgb_of(text);
    if background.a() >= 191 {
        return contrast_ratio(text, rgb_of(background));
    }
    if style.edge != SubtitleEdge::None {
        return contrast_ratio(text, rgb_of(edge));
    }
    crate::theme::subtitle_sample_backdrop()
        .iter()
        .map(|scene| contrast_ratio(text, rgb_of(*scene)))
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_engine::subtitle_style::preset;

    #[test]
    fn every_preset_and_edge_paints_in_both_themes() {
        let ctx = egui::Context::default();
        for tokens in [Tokens::dark(), Tokens::light()] {
            for edge in [
                SubtitleEdge::None,
                SubtitleEdge::Outline,
                SubtitleEdge::Shadow,
                SubtitleEdge::Raised,
                SubtitleEdge::Depressed,
            ] {
                for (name, _) in fm_engine::subtitle_style::PRESETS {
                    let style = SubtitleStyle {
                        edge,
                        ..preset(name).expect("preset")
                    };
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(640.0, 360.0));
                        paint(ui.painter(), rect, "Line one\nline two", &style, &tokens);
                        // Degenerate pictures and text never panic.
                        paint(ui.painter(), Rect::NOTHING, "x", &style, &tokens);
                        paint(ui.painter(), rect, "", &style, &tokens);
                    });
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn presets_meant_for_low_vision_have_strong_contrast() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            for name in [
                "high_contrast",
                "yellow_on_black",
                "black_on_white",
                "default",
                "large",
            ] {
                let style = preset(name).expect("preset");
                assert!(effective_contrast(&style, &tokens) >= 7.0, "{name}");
            }
        }
        let faint = SubtitleStyle {
            text_color: "#777777".into(),
            edge: SubtitleEdge::None,
            ..SubtitleStyle::default()
        };
        assert!(effective_contrast(&faint, &Tokens::dark()) < 4.5);
    }

    #[test]
    fn size_follows_the_picture_and_the_style() {
        let base = SubtitleStyle::default();
        assert_eq!(font_size(&base, 400.0), 20.0);
        let big = SubtitleStyle { size: 200, ..base };
        assert_eq!(font_size(&big, 400.0), 40.0);
        assert_eq!(font_size(&big, 10.0), 9.0, "never unreadably small");
    }
}
