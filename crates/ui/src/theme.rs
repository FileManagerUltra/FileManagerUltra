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
        FontId::proportional(self.font_size)
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
            ] {
                assert_ne!(color.a(), 0, "{name} must be opaque");
            }
            assert!(row_height > 0.0 && padding > 0.0 && font_size > 0.0);
            assert!(corner_radius >= 0.0 && border_width > 0.0);
            let _ = dark;
        }
        check(Tokens::dark());
        check(Tokens::light());
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
