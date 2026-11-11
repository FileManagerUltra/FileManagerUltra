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
    /// Accent text adjusted for at least 4.5:1 contrast on normal UI surfaces.
    pub accent_text: Color32,
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
    /// Git status colours (name tint and gutter letter).
    pub git_modified: Color32,
    pub git_added: Color32,
    pub git_removed: Color32,
    pub git_untracked: Color32,
    pub git_ignored: Color32,
    pub git_conflict: Color32,
    /// Diff pane row backgrounds and the stronger intra-line highlights.
    pub diff_add_bg: Color32,
    pub diff_del_bg: Color32,
    pub diff_add_word: Color32,
    pub diff_del_word: Color32,
    /// Subtitle text drawn over video, and the outline that keeps it readable on any picture.
    pub subtitle_text: Color32,
    pub subtitle_outline: Color32,
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
    /// Terminal text size.
    pub term_font_size: f32,
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
            accent_text: Color32::from_rgb(0x5c, 0xb2, 0xff),
            border: Color32::from_rgb(0x33, 0x38, 0x42),
            selection_bg: Color32::from_rgb(0x2b, 0x4a, 0x6e),
            selection_text: Color32::from_rgb(0xff, 0xff, 0xff),
            hover_bg: Color32::from_rgb(0x24, 0x29, 0x32),
            error: Color32::from_rgb(0xff, 0x6b, 0x6b),
            warning: Color32::from_rgb(0xff, 0xc8, 0x57),
            success: Color32::from_rgb(0x63, 0xc1, 0x74),
            git_modified: Color32::from_rgb(0xe5, 0xc0, 0x7b),
            git_added: Color32::from_rgb(0x63, 0xc1, 0x74),
            git_removed: Color32::from_rgb(0xff, 0x6b, 0x6b),
            git_untracked: Color32::from_rgb(0x6f, 0xe3, 0xd0),
            git_ignored: Color32::from_rgb(0x7a, 0x80, 0x88),
            git_conflict: Color32::from_rgb(0xff, 0x8f, 0xd0),
            diff_add_bg: Color32::from_rgb(0x1d, 0x3a, 0x26),
            diff_del_bg: Color32::from_rgb(0x44, 0x22, 0x26),
            diff_add_word: Color32::from_rgb(0x2b, 0x62, 0x3c),
            diff_del_word: Color32::from_rgb(0x7a, 0x30, 0x38),
            subtitle_text: Color32::from_rgb(0xff, 0xff, 0xff),
            subtitle_outline: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0xe0),
            row_height: 24.0,
            padding: 6.0,
            corner_radius: 6.0,
            border_width: 1.0,
            font_size: 14.0,
            term_font_size: 14.0,
            term_bg: Color32::from_rgb(0x0f, 0x11, 0x14),
            term_fg: Color32::from_rgb(0xd7, 0xda, 0xde),
            term_cursor: Color32::from_rgb(0x5c, 0xb2, 0xff),
            term_selection: Color32::from_rgb(0x2b, 0x4a, 0x6e),
            ansi: [
                // Normal colours follow the Tango palette (GNOME Terminal's default) so
                // curses/dialog backgrounds look as they do elsewhere.
                Color32::from_rgb(0x2e, 0x34, 0x36),
                Color32::from_rgb(0xcc, 0x00, 0x00),
                Color32::from_rgb(0x4e, 0x9a, 0x06),
                Color32::from_rgb(0xc4, 0xa0, 0x00),
                Color32::from_rgb(0x34, 0x65, 0xa4),
                Color32::from_rgb(0x75, 0x50, 0x7b),
                Color32::from_rgb(0x06, 0x98, 0x9a),
                Color32::from_rgb(0xd3, 0xd7, 0xcf),
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
            accent_text: Color32::from_rgb(0x0b, 0x62, 0xd6),
            border: Color32::from_rgb(0xd5, 0xd9, 0xdf),
            selection_bg: Color32::from_rgb(0xcf, 0xe4, 0xff),
            selection_text: Color32::from_rgb(0x10, 0x14, 0x18),
            hover_bg: Color32::from_rgb(0xe9, 0xec, 0xf0),
            error: Color32::from_rgb(0xc2, 0x2b, 0x2b),
            warning: Color32::from_rgb(0x9a, 0x67, 0x00),
            success: Color32::from_rgb(0x1f, 0x7a, 0x33),
            git_modified: Color32::from_rgb(0x8a, 0x5c, 0x00),
            git_added: Color32::from_rgb(0x1f, 0x7a, 0x33),
            git_removed: Color32::from_rgb(0xc2, 0x2b, 0x2b),
            git_untracked: Color32::from_rgb(0x0a, 0x7d, 0x7d),
            git_ignored: Color32::from_rgb(0x80, 0x86, 0x90),
            git_conflict: Color32::from_rgb(0xa8, 0x2a, 0x8a),
            diff_add_bg: Color32::from_rgb(0xdf, 0xf5, 0xe3),
            diff_del_bg: Color32::from_rgb(0xfd, 0xe3, 0xe3),
            diff_add_word: Color32::from_rgb(0xa6, 0xe3, 0xb2),
            diff_del_word: Color32::from_rgb(0xf6, 0xb5, 0xb5),
            // Drawn on the video, not on the light UI, so it stays light-on-dark.
            subtitle_text: Color32::from_rgb(0xff, 0xff, 0xfa),
            subtitle_outline: Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 0xe0),
            row_height: 24.0,
            padding: 6.0,
            corner_radius: 6.0,
            border_width: 1.0,
            font_size: 14.0,
            term_font_size: 14.0,
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

    /// Colour of a git status mark.
    pub fn git(&self, mark: fm_engine::git::Mark) -> Color32 {
        use fm_engine::git::Mark as M;
        match mark {
            M::Modified => self.git_modified,
            M::Added | M::Renamed => self.git_added,
            M::Removed => self.git_removed,
            M::Untracked => self.git_untracked,
            M::Ignored => self.git_ignored,
            M::Conflicted => self.git_conflict,
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
            C::Key => self.accent_text,
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
            300..=399 => self.accent_text,
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

    /// Resolve the preference and apply the user's look: accent, density and text sizes.
    pub fn resolve_with(prefs: &fm_engine::Prefs, system_dark: bool) -> Self {
        let mut t = Self::resolve(prefs.theme, system_dark);
        if let Some([r, g, b]) = prefs.accent_rgb() {
            let accent = Color32::from_rgb(r, g, b);
            t.accent = accent;
            t.accent_text = readable_accent(accent, t.text, [t.bg, t.panel, t.hover_bg], 4.5);
            t.term_cursor = accent;
            // Selection reads as a tint of the accent over the panel.
            let mix = |a: u8, b: u8| ((u16::from(a) * 35 + u16::from(b) * 65) / 100) as u8;
            t.selection_bg = Color32::from_rgb(
                mix(r, t.panel.r()),
                mix(g, t.panel.g()),
                mix(b, t.panel.b()),
            );
        }
        let (row, pad) = match prefs.density {
            fm_engine::Density::Compact => (20.0, 4.0),
            fm_engine::Density::Normal => (t.row_height, t.padding),
            fm_engine::Density::Comfortable => (30.0, 8.0),
        };
        let scale = prefs.ui_font_size() / t.font_size;
        t.row_height = (row * scale.max(1.0)).round();
        t.padding = pad;
        t.font_size = prefs.ui_font_size();
        t.term_font_size = prefs.term_font_size();
        t
    }

    /// Terminal font.
    pub fn term_font(&self) -> FontId {
        FontId::monospace(self.term_font_size)
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

fn luminance(color: Color32) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn contrast(a: Color32, b: Color32) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn readable_accent(
    accent: Color32,
    text: Color32,
    surfaces: [Color32; 3],
    minimum: f64,
) -> Color32 {
    for step in 0..=255u16 {
        let mix = |a: u8, b: u8| ((u16::from(a) * (255 - step) + u16::from(b) * step) / 255) as u8;
        let candidate = Color32::from_rgb(
            mix(accent.r(), text.r()),
            mix(accent.g(), text.g()),
            mix(accent.b(), text.b()),
        );
        if surfaces
            .iter()
            .all(|bg| contrast(candidate, *bg) >= minimum)
        {
            return candidate;
        }
    }
    text
}

/// Seed egui's per-theme visuals and spacing from tokens, overriding only
/// token-backed fields.
pub fn apply_to(ctx: &Context, tokens: &Tokens) {
    crate::markdown_fonts::install(ctx);
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
    visuals.hyperlink_color = tokens.accent_text;
    visuals.warn_fg_color = tokens.warning;
    visuals.error_fg_color = tokens.error;
    ctx.set_visuals_of(theme, visuals);

    let padding = tokens.padding;
    ctx.style_mut_of(theme, |style| {
        style.always_scroll_the_only_direction = false;
        style.spacing.item_spacing = Vec2::new(padding, padding);
        // Scale from egui's defaults, never from the current style, or repeated
        // applications would compound.
        let scale = tokens.font_size / 14.0;
        let base = egui::Style::default().text_styles;
        for (text_style, font) in style.text_styles.iter_mut() {
            font.family = egui::FontFamily::Monospace;
            if let Some(default) = base.get(text_style) {
                font.size = default.size * scale;
            }
        }
        style.spacing.button_padding = Vec2::new(padding, padding * 0.6);
    });
}

/// Accent presets offered in Preferences: label and `#rrggbb`.
pub const ACCENT_PRESETS: [(&str, &str); 7] = [
    ("Blue", "#5cb2ff"),
    ("Teal", "#2fbfa5"),
    ("Green", "#63c174"),
    ("Orange", "#f0913a"),
    ("Red", "#e5534b"),
    ("Pink", "#e56aa8"),
    ("Purple", "#a78bfa"),
];

/// A colour from `#rrggbb`, for swatches.
pub fn swatch(hex: &str) -> Option<Color32> {
    fm_engine::prefs::parse_hex_color(hex).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// Resolved colours of a subtitle style. `background` is transparent when the box is off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubtitleColors {
    pub text: Color32,
    pub background: Color32,
    pub edge: Color32,
}

/// Colours for `style`; `"default"` entries take the theme's subtitle tokens.
pub fn subtitle_colors(
    style: &fm_engine::subtitle_style::SubtitleStyle,
    tokens: &Tokens,
) -> SubtitleColors {
    let rgb = |c: Option<[u8; 3]>, fallback: Color32| {
        c.map_or(fallback, |[r, g, b]| Color32::from_rgb(r, g, b))
    };
    let text = rgb(style.text_rgb(), tokens.subtitle_text).gamma_multiply(style.text_alpha());
    let background =
        rgb(style.background_rgb(), Color32::BLACK).gamma_multiply(style.background_alpha());
    let edge = rgb(style.edge_rgb(), tokens.subtitle_outline);
    SubtitleColors {
        text,
        background,
        edge,
    }
}

/// Opaque RGB of a colour, for contrast checks.
pub fn rgb_of(color: Color32) -> [u8; 3] {
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    [r, g, b]
}

/// A dark and a bright sample "scene" behind the subtitle preview, so readability can
/// be judged on both.
pub fn subtitle_sample_backdrop() -> [Color32; 2] {
    [
        Color32::from_rgb(0x1a, 0x22, 0x30),
        Color32::from_rgb(0xd8, 0xdc, 0xc8),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn accent_text_has_readable_contrast_without_changing_the_chosen_accent() {
        for theme in [ThemePreference::Light, ThemePreference::Dark] {
            for r in [0, 64, 128, 192, 255] {
                for g in [0, 64, 128, 192, 255] {
                    for b in [0, 64, 128, 192, 255] {
                        let prefs = fm_engine::Prefs {
                            theme,
                            accent: Some(format!("#{r:02x}{g:02x}{b:02x}")),
                            ..Default::default()
                        };
                        let t = Tokens::resolve_with(&prefs, false);
                        assert_eq!(t.accent, Color32::from_rgb(r, g, b));
                        for surface in [t.bg, t.panel, t.hover_bg] {
                            assert!(
                                contrast(t.accent_text, surface) >= 4.5,
                                "{theme:?} {r} {g} {b}"
                            );
                        }
                    }
                }
            }
        }
        for t in [Tokens::light(), Tokens::dark()] {
            for surface in [t.bg, t.panel, t.hover_bg] {
                assert!(contrast(t.accent_text, surface) >= 4.5);
            }
            assert_eq!(
                readable_accent(t.accent, t.text, [t.bg, t.panel, t.hover_bg], 4.5),
                t.accent
            );
        }
    }

    use super::*;

    #[test]
    fn shift_wheel_scrolls_only_the_hovered_area_horizontally() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            for (shift, horizontal, delta) in [
                (true, true, Vec2::new(0.0, -80.0)),
                (true, true, Vec2::new(-80.0, 0.0)),
                (true, false, Vec2::new(0.0, -80.0)),
                (false, true, Vec2::new(0.0, -80.0)),
            ] {
                let ctx = Context::default();
                apply_to(&ctx, &tokens);
                ctx.set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let mut offsets = [Vec2::ZERO; 2];
                for frame in 0..30 {
                    let mut events = vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))];
                    if frame == 3 {
                        events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta,
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: if shift {
                                egui::Modifiers::SHIFT
                            } else {
                                egui::Modifiers::NONE
                            },
                        });
                    }
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                Vec2::new(700.0, 300.0),
                            )),
                            time: Some(frame as f64 / 60.0),
                            events,
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                ui.columns(2, |uis| {
                                    for (index, ui) in uis.iter_mut().enumerate() {
                                        let area = if horizontal {
                                            egui::ScrollArea::both()
                                        } else {
                                            egui::ScrollArea::vertical()
                                        };
                                        let out =
                                            area.id_salt(index).auto_shrink(false).show(ui, |ui| {
                                                ui.allocate_space(Vec2::new(1200.0, 1200.0));
                                            });
                                        offsets[index] = out.state.clamped_offset();
                                    }
                                });
                            });
                        },
                    );
                    output.textures_delta.clear();
                }
                assert_eq!(offsets[1], Vec2::ZERO, "unhovered area does not scroll");
                if shift {
                    assert_eq!(offsets[0].y, 0.0, "Shift-wheel never becomes vertical");
                    if horizontal {
                        assert!(offsets[0].x > 20.0, "Shift-wheel scrolls horizontally");
                    }
                } else {
                    assert_eq!(offsets[0].x, 0.0);
                    assert!(offsets[0].y > 20.0, "ordinary wheel remains vertical");
                }
            }
        }
    }

    /// Destructuring without `..` is exhaustive at compile time: adding a field to
    /// `Tokens` fails this test until both palettes cover it.
    #[test]
    fn font_size_changes_do_not_compound() {
        let ctx = Context::default();
        let size = |ctx: &Context| ctx.global_style().text_styles[&egui::TextStyle::Body].size;
        for pref in [20.0, 10.0, 14.0, 20.0, 14.0] {
            let mut t = Tokens::dark();
            t.font_size = pref;
            apply_to(&ctx, &t);
            ctx.set_theme(egui::Theme::Dark);
            let expected =
                egui::Style::default().text_styles[&egui::TextStyle::Body].size * pref / 14.0;
            assert!((size(&ctx) - expected).abs() < 0.01, "{pref}");
        }
    }

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
                accent_text,
                border,
                selection_bg,
                selection_text,
                hover_bg,
                error,
                warning,
                success,
                git_modified,
                git_added,
                git_removed,
                git_untracked,
                git_ignored,
                git_conflict,
                diff_add_bg,
                diff_del_bg,
                diff_add_word,
                diff_del_word,
                subtitle_text,
                subtitle_outline,
                row_height,
                padding,
                corner_radius,
                border_width,
                font_size,
                term_font_size,
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
                ("accent_text", accent_text),
                ("border", border),
                ("selection_bg", selection_bg),
                ("selection_text", selection_text),
                ("hover_bg", hover_bg),
                ("error", error),
                ("warning", warning),
                ("success", success),
                ("git_modified", git_modified),
                ("git_added", git_added),
                ("git_removed", git_removed),
                ("git_untracked", git_untracked),
                ("git_ignored", git_ignored),
                ("git_conflict", git_conflict),
                ("diff_add_bg", diff_add_bg),
                ("diff_del_bg", diff_del_bg),
                ("diff_add_word", diff_add_word),
                ("diff_del_word", diff_del_word),
                ("term_bg", term_bg),
                ("term_fg", term_fg),
                ("term_cursor", term_cursor),
                ("term_selection", term_selection),
                ("subtitle_text", subtitle_text),
                ("subtitle_outline", subtitle_outline),
            ] {
                assert_ne!(color.a(), 0, "{name} must be opaque");
            }
            assert!(row_height > 0.0 && padding > 0.0 && font_size > 0.0);
            assert!(term_font_size > 0.0);
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
