//! Subtitle appearance, chosen for readability (including low vision) and applied the
//! same way to every text subtitle type: SRT, WebVTT, embedded text tracks and
//! ASS/SSA (whose own styling is deliberately ignored). Plain data; `fm-ui` resolves it
//! to colours and fonts.

use fm_vfs::FmError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::prefs::parse_hex_color;
use crate::{Session, file_cmds::object, registry::CommandSpec};

pub const SIZE_RANGE: std::ops::RangeInclusive<u64> = 50..=400;
pub const POSITION_RANGE: std::ops::RangeInclusive<u64> = 0..=45;
/// Text below a quarter opacity would be unreadable; refuse it.
pub const TEXT_OPACITY_RANGE: std::ops::RangeInclusive<u64> = 25..=100;
pub const OPACITY_RANGE: std::ops::RangeInclusive<u64> = 0..=100;
pub const EDGE_WIDTH_RANGE: std::ops::RangeInclusive<u64> = 1..=10;
pub const LETTER_SPACING_RANGE: std::ops::RangeInclusive<u64> = 0..=30;
pub const LINE_SPACING_RANGE: std::ops::RangeInclusive<u64> = 100..=200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleFont {
    #[default]
    Sans,
    Mono,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleEdge {
    None,
    #[default]
    Outline,
    Shadow,
    Raised,
    Depressed,
}

/// Persisted in `prefs.toml` as the `subtitle_style` table. Colours are `#rrggbb`, or
/// `"default"` for the theme's subtitle colours. Out-of-range hand edits are clamped
/// by the accessors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubtitleStyle {
    /// Percent of the default size (5 % of the picture height).
    pub size: u64,
    /// Distance above the picture's bottom edge, in percent of its height.
    pub position: u64,
    pub font: SubtitleFont,
    pub bold: bool,
    pub text_color: String,
    pub text_opacity: u64,
    /// Box behind each cue; opacity 0 turns it off.
    pub background_color: String,
    pub background_opacity: u64,
    pub edge: SubtitleEdge,
    pub edge_color: String,
    /// Edge thickness, 1 (thin) to 10 (heavy).
    pub edge_width: u64,
    /// Extra space between letters, in percent of the font size.
    pub letter_spacing: u64,
    /// Line height in percent of the font size.
    pub line_spacing: u64,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            size: 100,
            position: 5,
            font: SubtitleFont::Sans,
            bold: false,
            text_color: "default".into(),
            text_opacity: 100,
            background_color: "#000000".into(),
            background_opacity: 0,
            edge: SubtitleEdge::Outline,
            edge_color: "default".into(),
            edge_width: 4,
            letter_spacing: 0,
            line_spacing: 115,
        }
    }
}

/// Ready-made styles; each sets every field, so a preset always looks the same.
pub const PRESETS: [(&str, &str); 5] = [
    ("default", "Default"),
    ("high_contrast", "High contrast (white on black)"),
    ("yellow_on_black", "Yellow on black"),
    ("large", "Large with heavy outline"),
    ("black_on_white", "Black on white"),
];

pub fn preset(name: &str) -> Option<SubtitleStyle> {
    let base = SubtitleStyle::default();
    Some(match name {
        "default" => base,
        "high_contrast" => SubtitleStyle {
            size: 130,
            bold: true,
            text_color: "#ffffff".into(),
            background_opacity: 100,
            edge: SubtitleEdge::None,
            ..base
        },
        "yellow_on_black" => SubtitleStyle {
            size: 130,
            bold: true,
            text_color: "#ffff00".into(),
            background_opacity: 100,
            edge: SubtitleEdge::None,
            ..base
        },
        "large" => SubtitleStyle {
            size: 180,
            bold: true,
            text_color: "#ffffff".into(),
            edge_color: "#000000".into(),
            edge_width: 7,
            letter_spacing: 5,
            ..base
        },
        "black_on_white" => SubtitleStyle {
            size: 130,
            bold: true,
            text_color: "#000000".into(),
            background_color: "#ffffff".into(),
            background_opacity: 100,
            edge: SubtitleEdge::None,
            ..base
        },
        _ => return None,
    })
}

fn clamp(value: u64, range: &std::ops::RangeInclusive<u64>) -> u64 {
    value.clamp(*range.start(), *range.end())
}

/// `None` = the theme's default for that colour.
fn rgb(text: &str) -> Option<[u8; 3]> {
    parse_hex_color(text)
}

impl SubtitleStyle {
    pub fn size_factor(&self) -> f32 {
        clamp(self.size, &SIZE_RANGE) as f32 / 100.0
    }
    pub fn position_fraction(&self) -> f32 {
        clamp(self.position, &POSITION_RANGE) as f32 / 100.0
    }
    pub fn text_rgb(&self) -> Option<[u8; 3]> {
        rgb(&self.text_color)
    }
    pub fn text_alpha(&self) -> f32 {
        clamp(self.text_opacity, &TEXT_OPACITY_RANGE) as f32 / 100.0
    }
    pub fn background_rgb(&self) -> Option<[u8; 3]> {
        rgb(&self.background_color)
    }
    pub fn background_alpha(&self) -> f32 {
        clamp(self.background_opacity, &OPACITY_RANGE) as f32 / 100.0
    }
    pub fn edge_rgb(&self) -> Option<[u8; 3]> {
        rgb(&self.edge_color)
    }
    pub fn edge_factor(&self) -> f32 {
        clamp(self.edge_width, &EDGE_WIDTH_RANGE) as f32
    }
    pub fn letter_spacing_fraction(&self) -> f32 {
        clamp(self.letter_spacing, &LETTER_SPACING_RANGE) as f32 / 100.0
    }
    pub fn line_spacing_factor(&self) -> f32 {
        clamp(self.line_spacing, &LINE_SPACING_RANGE) as f32 / 100.0
    }

    /// Validate and apply one field; nothing changes on error.
    fn set_field(&mut self, key: &str, value: &Value) -> Result<(), FmError> {
        let bad = |expected: &str| FmError::InvalidParams(format!("{key}: expected {expected}"));
        let ranged = |range: &std::ops::RangeInclusive<u64>| {
            value
                .as_u64()
                .filter(|n| range.contains(n))
                .ok_or_else(|| bad(&format!("{}-{}", range.start(), range.end())))
        };
        let color = |allow_default: bool| -> Result<String, FmError> {
            match value.as_str() {
                Some("default") if allow_default => Ok("default".into()),
                Some(text) if parse_hex_color(text).is_some() => Ok(text.to_ascii_lowercase()),
                _ if allow_default => Err(bad("\"default\" or #rrggbb")),
                _ => Err(bad("#rrggbb")),
            }
        };
        let choice = |allowed: &[&str]| -> Result<String, FmError> {
            value
                .as_str()
                .filter(|v| allowed.contains(v))
                .map(str::to_string)
                .ok_or_else(|| bad(&format!("one of {}", allowed.join(", "))))
        };
        match key {
            "size" => self.size = ranged(&SIZE_RANGE)?,
            "position" => self.position = ranged(&POSITION_RANGE)?,
            "font" => {
                self.font = match choice(&["sans", "mono"])?.as_str() {
                    "mono" => SubtitleFont::Mono,
                    _ => SubtitleFont::Sans,
                }
            }
            "bold" => self.bold = value.as_bool().ok_or_else(|| bad("true or false"))?,
            "text_color" => self.text_color = color(true)?,
            "text_opacity" => self.text_opacity = ranged(&TEXT_OPACITY_RANGE)?,
            "background_color" => self.background_color = color(false)?,
            "background_opacity" => self.background_opacity = ranged(&OPACITY_RANGE)?,
            "edge" => {
                self.edge =
                    match choice(&["none", "outline", "shadow", "raised", "depressed"])?.as_str() {
                        "none" => SubtitleEdge::None,
                        "shadow" => SubtitleEdge::Shadow,
                        "raised" => SubtitleEdge::Raised,
                        "depressed" => SubtitleEdge::Depressed,
                        _ => SubtitleEdge::Outline,
                    }
            }
            "edge_color" => self.edge_color = color(true)?,
            "edge_width" => self.edge_width = ranged(&EDGE_WIDTH_RANGE)?,
            "letter_spacing" => self.letter_spacing = ranged(&LETTER_SPACING_RANGE)?,
            "line_spacing" => self.line_spacing = ranged(&LINE_SPACING_RANGE)?,
            _ => {
                return Err(FmError::InvalidParams(format!(
                    "unknown subtitle style field {key}"
                )));
            }
        }
        Ok(())
    }

    /// Apply `{preset}` and/or individual fields atomically.
    pub fn apply(&self, fields: &Map<String, Value>) -> Result<Self, FmError> {
        let mut next = match fields.get("preset") {
            Some(name) => name
                .as_str()
                .and_then(preset)
                .ok_or_else(|| FmError::InvalidParams("preset: unknown name".into()))?,
            None => self.clone(),
        };
        for (key, value) in fields {
            if key != "preset" {
                next.set_field(key, value)?;
            }
        }
        Ok(next)
    }
}

/// WCAG contrast ratio of two opaque colours (1.0 to 21.0).
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    fn luminance([r, g, b]: [u8; 3]) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "media.subtitle.style",
        label: "Subtitle Appearance",
        menu: &["Media", "Subtitle appearance"],
        shortcut: None,
        params: "{preset?: default|high_contrast|yellow_on_black|large|black_on_white, size? 50-400, position? 0-45, font? sans|mono, bold?, text_color? \"default\"|#rrggbb, text_opacity? 25-100, background_color? #rrggbb, background_opacity? 0-100, edge? none|outline|shadow|raised|depressed, edge_color? \"default\"|#rrggbb, edge_width? 1-10, letter_spacing? 0-30, line_spacing? 100-200}: applied to every text subtitle type; all-or-nothing",
        enabled: |_| true,
        run: |s: &mut Session, params| {
            let map = object(params)?
                .filter(|map| !map.is_empty())
                .ok_or_else(|| {
                    FmError::InvalidParams("a preset or style fields required".into())
                })?;
            if map.len() > 16 {
                return Err(FmError::InvalidParams("too many fields".into()));
            }
            s.prefs.subtitle_style = s.prefs.subtitle_style.apply(map)?;
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_and_presets_apply_and_persist() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.subtitle.style",
            &json!({"size": 200, "text_color": "#FFFF00", "background_opacity": 80, "edge": "shadow", "font": "mono", "bold": true}),
        )
        .expect("style");
        let style = &s.prefs.subtitle_style;
        assert_eq!(style.size_factor(), 2.0);
        assert_eq!(style.text_rgb(), Some([255, 255, 0]));
        assert_eq!(style.edge, SubtitleEdge::Shadow);
        assert_eq!(style.font, SubtitleFont::Mono);
        let encoded = toml::to_string(&s.prefs).expect("serialize");
        let decoded: crate::prefs::Prefs = toml::from_str(&encoded).expect("deserialize");
        assert_eq!(&decoded.subtitle_style, style);
        // A preset replaces everything, then later fields refine it.
        s.dispatch(
            "media.subtitle.style",
            &json!({"preset": "yellow_on_black", "size": 150}),
        )
        .expect("preset");
        let style = &s.prefs.subtitle_style;
        assert_eq!(style.text_rgb(), Some([255, 255, 0]));
        assert_eq!(style.background_alpha(), 1.0);
        assert_eq!(style.size, 150);
        assert_eq!(style.font, SubtitleFont::Sans);
        for (name, _) in PRESETS {
            assert!(preset(name).is_some(), "{name}");
        }
    }

    #[test]
    fn bad_values_change_nothing() {
        let mut s = Session::new().expect("session");
        let before = s.prefs.subtitle_style.clone();
        for params in [
            json!({"size": 49}),
            json!({"size": 401}),
            json!({"size": "big"}),
            json!({"text_opacity": 10}),
            json!({"text_color": "yellow"}),
            json!({"background_color": "default"}),
            json!({"edge": "glow"}),
            json!({"font": "comic"}),
            json!({"bold": 1}),
            json!({"preset": "neon"}),
            json!({"size": 120, "edge_width": 99}),
            json!({"colour": "#ffffff"}),
            json!({}),
            json!(null),
            json!([1, 2]),
        ] {
            assert!(
                s.dispatch("media.subtitle.style", &params).is_err(),
                "{params}"
            );
            assert_eq!(s.prefs.subtitle_style, before, "{params}");
        }
    }

    #[test]
    fn hand_edited_values_are_clamped() {
        let style: SubtitleStyle =
            toml::from_str("size = 9999\nposition = 90\ntext_opacity = 0\ntext_color = \"nope\"")
                .expect("parse");
        assert_eq!(style.size_factor(), 4.0);
        assert_eq!(style.position_fraction(), 0.45);
        assert_eq!(style.text_alpha(), 0.25);
        assert_eq!(
            style.text_rgb(),
            None,
            "invalid colour falls back to the theme"
        );
        assert_eq!(style.line_spacing, 115, "missing fields take defaults");
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast_ratio([255, 255, 255], [0, 0, 0]) - 21.0).abs() < 0.01);
        assert!((contrast_ratio([0, 0, 0], [0, 0, 0]) - 1.0).abs() < 0.01);
        assert!(contrast_ratio([255, 255, 0], [0, 0, 0]) > 19.0);
        assert!(contrast_ratio([119, 119, 119], [255, 255, 255]) < 4.5);
    }
}
