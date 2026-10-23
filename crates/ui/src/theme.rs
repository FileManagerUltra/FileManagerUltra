//! Design tokens — semantic colours and metrics for the whole UI.
//!
//! **Rule:** colour literals appear in this module only (enforced by
//! `cargo xtask brand-check`). Widgets take [`Tokens`]; nothing hard-codes a colour,
//! and every widget must render correctly with both [`Tokens::dark`] and
//! [`Tokens::light`].

use egui::{Color32, Context, FontId, RichText, Stroke, Vec2, Visuals};

use fm_engine::ThemePreference;

/// Semantic colours + metrics. Exhaustive constructors: adding a field means updating
/// both `dark()` and `light()` (the test below destructures every field, so the build
/// fails until you do).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// `true` for the dark palette (egui base visuals + widget styling).
    pub dark: bool,
    /// Window / central background.
    pub bg: Color32,
    /// Panels (menu bar, status bar, dialogs).
    pub panel: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary / hint text.
    pub text_muted: Color32,
    /// Accent (focus rings, active toggles, links).
    pub accent: Color32,
    /// Hairlines and separators.
    pub border: Color32,
    /// Selected row background.
    pub selection_bg: Color32,
    /// Selected row text.
    pub selection_text: Color32,
    /// Hovered row background.
    pub hover_bg: Color32,
    /// Error messages and destructive actions.
    pub error: Color32,
    /// Warnings.
    pub warning: Color32,
    /// Success / completion.
    pub success: Color32,
    /// Virtualized listing row height (px).
    pub row_height: f32,
    /// Generic inner padding (px).
    pub padding: f32,
    /// Corner radius for panels and dialogs (px).
    pub corner_radius: f32,
    /// Hairline width (px).
    pub border_width: f32,
    /// Base font size (px).
    pub font_size: f32,
    /// Terminal default background / foreground / cursor / selection.
    pub term_bg: Color32,
    pub term_fg: Color32,
    pub term_cursor: Color32,
    pub term_selection: Color32,
    /// The 16 ANSI colours (0-7 normal, 8-15 bright).
    pub ansi: [Color32; 16],
}

impl Tokens {
    /// Dark palette.
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(0x14, 0x16, 0x1a),
            panel: Color32::from_rgb(0x1c, 0x1f, 0x25),
            text: Color32::from_rgb(0xe6, 0xe8, 0xeb),
            text_muted: Color32::from_rgb(0x9a, 0xa0, 0xa8),
            accent: Color32::from_rgb(0x5c, 0xb2, 0xff),
            border: Color32::from_rgb(0x33, 0x38, 0x42),
            selection_bg: Color32::from_rgb(0x2b, 0x4a, 0x6e),
            selection_text: Color32::from_rgb(0xff, 0xff, 0xff),
            hover_bg: Color32::from_rgb(0x24, 0x29, 0x32),
            error: Color32::from_rgb(0xff, 0x6b, 0x6b),
            warning: Color32::from_rgb(0xff, 0xc8, 0x57),
            success: Color32::from_rgb(0x63, 0xc1, 0x74),
            row_height: 24.0,
            padding: 6.0,
            corner_radius: 6.0,
            border_width: 1.0,
            font_size: 14.0,
            term_bg: Color32::from_rgb(0x0f, 0x11, 0x14),
            term_fg: Color32::from_rgb(0xd7, 0xda, 0xde),
            term_cursor: Color32::from_rgb(0x5c, 0xb2, 0xff),
            term_selection: Color32::from_rgb(0x2b, 0x4a, 0x6e),
            ansi: [
                Color32::from_rgb(0x1d, 0x1f, 0x21),
                Color32::from_rgb(0xcc, 0x66, 0x66),
                Color32::from_rgb(0xb5, 0xbd, 0x68),
                Color32::from_rgb(0xf0, 0xc6, 0x74),
                Color32::from_rgb(0x81, 0xa2, 0xbe),
                Color32::from_rgb(0xb2, 0x94, 0xbb),
                Color32::from_rgb(0x8a, 0xbe, 0xb7),
                Color32::from_rgb(0xc5, 0xc8, 0xc6),
                Color32::from_rgb(0x66, 0x69, 0x6d),
                Color32::from_rgb(0xff, 0x6b, 0x6b),
                Color32::from_rgb(0xa5, 0xe0, 0x75),
                Color32::from_rgb(0xff, 0xd8, 0x6b),
                Color32::from_rgb(0x5c, 0xb2, 0xff),
                Color32::from_rgb(0xd0, 0x9a, 0xe0),
                Color32::from_rgb(0x6f, 0xe3, 0xd0),
                Color32::from_rgb(0xff, 0xff, 0xff),
            ],
        }
    }

    /// Light palette.
    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(0xf7, 0xf8, 0xfa),
            panel: Color32::from_rgb(0xff, 0xff, 0xff),
            text: Color32::from_rgb(0x1b, 0x1e, 0x23),
            text_muted: Color32::from_rgb(0x63, 0x69, 0x73),
            accent: Color32::from_rgb(0x0b, 0x62, 0xd6),
            border: Color32::from_rgb(0xd5, 0xd9, 0xdf),
            selection_bg: Color32::from_rgb(0xcf, 0xe4, 0xff),
            selection_text: Color32::from_rgb(0x10, 0x14, 0x18),
            hover_bg: Color32::from_rgb(0xe9, 0xec, 0xf0),
            error: Color32::from_rgb(0xc2, 0x2b, 0x2b),
            warning: Color32::from_rgb(0x9a, 0x67, 0x00),
            success: Color32::from_rgb(0x1f, 0x7a, 0x33),
            row_height: 24.0,
            padding: 6.0,
            corner_radius: 6.0,
            border_width: 1.0,
            font_size: 14.0,
            term_bg: Color32::from_rgb(0xff, 0xff, 0xff),
            term_fg: Color32::from_rgb(0x1b, 0x1e, 0x23),
            term_cursor: Color32::from_rgb(0x0b, 0x62, 0xd6),
            term_selection: Color32::from_rgb(0xcf, 0xe4, 0xff),
            ansi: [
                Color32::from_rgb(0x24, 0x29, 0x2f),
                Color32::from_rgb(0xc2, 0x2b, 0x2b),
                Color32::from_rgb(0x1f, 0x7a, 0x33),
                Color32::from_rgb(0x8a, 0x5c, 0x00),
                Color32::from_rgb(0x0b, 0x62, 0xd6),
                Color32::from_rgb(0x8a, 0x3f, 0xa8),
                Color32::from_rgb(0x0a, 0x7d, 0x7d),
                Color32::from_rgb(0x6e, 0x76, 0x81),
                Color32::from_rgb(0x57, 0x60, 0x6a),
                Color32::from_rgb(0xe0, 0x3a, 0x3a),
                Color32::from_rgb(0x2e, 0x9a, 0x46),
                Color32::from_rgb(0xb0, 0x7a, 0x00),
                Color32::from_rgb(0x1f, 0x7a, 0xff),
                Color32::from_rgb(0xa8, 0x5c, 0xc8),
                Color32::from_rgb(0x12, 0x9c, 0x9c),
                Color32::from_rgb(0x1b, 0x1e, 0x23),
            ],
        }
    }

    /// Syntax class -> colour, drawn from the semantic palette.
    pub fn syntax(&self, class: fm_preview::Class) -> Color32 {
        use fm_preview::Class as C;
        match class {
            C::Plain | C::Punct => self.text,
            C::Comment => self.text_muted,
            C::String => self.ansi[2],
            C::Number | C::Constant => self.ansi[3],
            C::Keyword => self.ansi[5],
            C::Function => self.ansi[4],
            C::Type => self.ansi[6],
            C::Key => self.accent,
            C::Tag => self.ansi[1],
        }
    }

    /// Log level -> colour.
    pub fn level(&self, level: Option<fm_preview::log::Level>) -> Color32 {
        use fm_preview::log::Level as L;
        match level {
            None => self.text,
            Some(L::Debug) => self.text_muted,
            Some(L::Info | L::Notice) => self.text,
            Some(L::Warning) => self.warning,
            Some(L::Error | L::Critical) => self.error,
        }
    }

    /// HTTP status class -> colour.
    pub fn status(&self, status: u16) -> Color32 {
        match status {
            500.. => self.error,
            400..=499 => self.warning,
            300..=399 => self.accent,
            200..=299 => self.success,
            _ => self.text_muted,
        }
    }

    /// Terminal colour -> egui colour (`fg` picks the default for `Color::Default`).
    pub fn term_color(&self, color: fm_term::Color, fg: bool) -> Color32 {
        match color {
            fm_term::Color::Default => {
                if fg {
                    self.term_fg
                } else {
                    self.term_bg
                }
            }
            fm_term::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
            fm_term::Color::Indexed(i) => match i {
                0..=15 => self
                    .ansi
                    .get(usize::from(i))
                    .copied()
                    .unwrap_or(self.term_fg),
                16..=231 => {
                    let n = i - 16;
                    let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                    Color32::from_rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
                }
                232..=255 => {
                    let v = 8 + (i - 232) * 10;
                    Color32::from_rgb(v, v, v)
                }
            },
        }
    }

    /// Resolve the stored preference against the OS theme.
    pub fn resolve(pref: ThemePreference, system_dark: bool) -> Self {
        match pref {
            ThemePreference::Dark => Self::dark(),
            ThemePreference::Light => Self::light(),
            ThemePreference::System => {
                if system_dark {
                    Self::dark()
                } else {
                    Self::light()
                }
            }
        }
    }

    /// Base font, sized from tokens.
    pub fn font(&self) -> FontId {
        FontId::monospace(self.font_size)
    }

    /// Muted text helper for status bars and hints.
    pub fn muted(&self, text: impl Into<String>) -> RichText {
        RichText::new(text.into())
            .color(self.text_muted)
            .font(self.font())
    }
}

/// Seed egui's per-theme visuals and spacing from tokens, overriding only
/// token-backed fields.
pub fn apply_to(ctx: &Context, tokens: &Tokens) {
    let theme = if tokens.dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    let mut visuals: Visuals = if tokens.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.override_text_color = Some(tokens.text);
    visuals.panel_fill = tokens.panel;
    visuals.window_fill = tokens.panel;
    visuals.extreme_bg_color = tokens.bg;
    visuals.faint_bg_color = tokens.hover_bg;
    visuals.selection.bg_fill = tokens.selection_bg;
    visuals.selection.stroke = Stroke::new(tokens.border_width.max(1.0), tokens.selection_text);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(tokens.border_width, tokens.border);
    visuals.widgets.inactive.bg_stroke = Stroke::new(tokens.border_width, tokens.border);
    visuals.widgets.hovered.bg_stroke = Stroke::new(tokens.border_width, tokens.accent);
    visuals.widgets.active.bg_stroke = Stroke::new(tokens.border_width, tokens.accent);
    visuals.widgets.inactive.weak_bg_fill = tokens.panel;
    visuals.widgets.inactive.bg_fill = tokens.panel;
    let rounding = egui::CornerRadius::same(tokens.corner_radius.round() as u8);
    visuals.window_corner_radius = rounding;
    visuals.menu_corner_radius = rounding;
    visuals.hyperlink_color = tokens.accent;
    visuals.warn_fg_color = tokens.warning;
    visuals.error_fg_color = tokens.error;
    ctx.set_visuals_of(theme, visuals);

    let padding = tokens.padding;
    ctx.style_mut_of(theme, |style| {
        style.spacing.item_spacing = Vec2::new(padding, padding);
        for font in style.text_styles.values_mut() {
            font.family = egui::FontFamily::Monospace;
        }
        style.spacing.button_padding = Vec2::new(padding, padding * 0.6);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Destructuring without `..` is exhaustive at compile time: adding a field to
    /// `Tokens` fails this test until both palettes cover it.
    #[test]
    fn both_palettes_cover_every_field() {
        fn check(tokens: Tokens) {
            let Tokens {
                dark,
                bg,
                panel,
                text,
                text_muted,
                accent,
                border,
                selection_bg,
                selection_text,
                hover_bg,
                error,
                warning,
                success,
                row_height,
                padding,
                corner_radius,
                border_width,
                font_size,
                term_bg,
                term_fg,
                term_cursor,
                term_selection,
                ansi,
            } = tokens;
            for (name, color) in [
                ("bg", bg),
                ("panel", panel),
                ("text", text),
                ("text_muted", text_muted),
                ("accent", accent),
                ("border", border),
                ("selection_bg", selection_bg),
                ("selection_text", selection_text),
                ("hover_bg", hover_bg),
                ("error", error),
                ("warning", warning),
                ("success", success),
                ("term_bg", term_bg),
                ("term_fg", term_fg),
                ("term_cursor", term_cursor),
                ("term_selection", term_selection),
            ] {
                assert_ne!(color.a(), 0, "{name} must be opaque");
            }
            assert!(row_height > 0.0 && padding > 0.0 && font_size > 0.0);
            assert!(corner_radius >= 0.0 && border_width > 0.0);
            let _ = dark;
            assert!(ansi.iter().all(|c| c.a() == 255));
            assert_ne!(ansi[1], ansi[2], "ANSI colours must be distinct");
        }
        check(Tokens::dark());
        check(Tokens::light());
    }

    #[test]
    fn terminal_colours_map_defaults_cube_and_greys() {
        let t = Tokens::dark();
        assert_eq!(t.term_color(fm_term::Color::Default, true), t.term_fg);
        assert_eq!(t.term_color(fm_term::Color::Default, false), t.term_bg);
        assert_eq!(t.term_color(fm_term::Color::Indexed(1), true), t.ansi[1]);
        assert_eq!(
            t.term_color(fm_term::Color::Indexed(16), true),
            Color32::from_rgb(0, 0, 0)
        );
        assert_eq!(
            t.term_color(fm_term::Color::Indexed(231), true),
            Color32::from_rgb(255, 255, 255)
        );
        assert_eq!(
            t.term_color(fm_term::Color::Indexed(232), true),
            Color32::from_rgb(8, 8, 8)
        );
        assert_eq!(
            t.term_color(fm_term::Color::Rgb(1, 2, 3), false),
            Color32::from_rgb(1, 2, 3)
        );
    }

    #[test]
    fn palettes_differ() {
        assert_ne!(Tokens::dark().bg, Tokens::light().bg);
        assert!(Tokens::dark().dark);
        assert!(!Tokens::light().dark);
    }

    #[test]
    fn resolve_honours_system() {
        assert!(Tokens::resolve(ThemePreference::System, true).dark);
        assert!(!Tokens::resolve(ThemePreference::System, false).dark);
        assert!(Tokens::resolve(ThemePreference::Dark, false).dark);
        assert!(!Tokens::resolve(ThemePreference::Light, true).dark);
    }
}
