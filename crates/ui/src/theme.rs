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
    /// Edit mode of the Preview pane: the frame around the editor and its badge, so
    /// editing is never mistaken for viewing (orange, distinct from `warning`).
    pub editing: Color32,
    /// Text and icons on an `editing` fill.
    pub editing_text: Color32,
    /// Success / completion.
    pub success: Color32,
    /// Git status colours (name tint and gutter letter).
    pub git_modified: Color32,
    pub git_added: Color32,
    pub git_removed: Color32,
    pub git_untracked: Color32,
    pub git_ignored: Color32,
    pub git_conflict: Color32,
    /// Archives and everything inside them (name tint in the tree, listing and
    /// breadcrumb; add-archive-vfs D14). Readable on `bg`, `panel` and `hover_bg`.
    pub archive: Color32,
    /// Diff pane row backgrounds and the stronger intra-line highlights.
    pub diff_add_bg: Color32,
    pub diff_del_bg: Color32,
    pub diff_add_word: Color32,
    pub diff_del_word: Color32,
    /// Folder sync: tint behind rows that need a decision (conflicts, unreadable items).
    pub sync_conflict_bg: Color32,
    /// Subtitle text drawn over video, and the outline that keeps it readable on any picture.
    pub subtitle_text: Color32,
    pub subtitle_outline: Color32,
    /// The two squares behind transparent pictures (SVG drawings). Light in both themes so
    /// dark artwork stays visible, as in image editors.
    pub checker: [Color32; 2],
    /// PDF preview: the backdrop between pages and the hairline around each page.
    pub pdf_gap: Color32,
    pub pdf_page_border: Color32,
    /// PDF form fields while filling: translucent fill over the page, and the focus ring.
    pub field_fill: Color32,
    pub field_focus: Color32,
    /// PDF signature states: valid, changed after signing, broken or unverifiable.
    pub sig_valid: Color32,
    pub sig_changed: Color32,
    pub sig_broken: Color32,
    /// Ink on the signature pad and signature previews (drawn on a light pad in both themes).
    pub sig_ink: Color32,
    /// PDF Fill & Sign action tiles: their text and icon colour, and one fill per action
    /// (sign, place, remember, neutral, save, save as, replace, discard). Every fill keeps
    /// at least 4.5:1 contrast with `action_text` (tested).
    pub action_text: Color32,
    pub action_sign: Color32,
    pub action_place: Color32,
    pub action_remember: Color32,
    pub action_neutral: Color32,
    pub action_save: Color32,
    pub action_save_as: Color32,
    pub action_replace: Color32,
    pub action_discard: Color32,
    /// "High contrast" PDF pages: text colour and page colour.
    pub pdf_contrast_fg: Color32,
    pub pdf_contrast_bg: Color32,
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
    /// Zoom of the pane being drawn (1.0 outside zoomed panes); fixed-size widgets such as
    /// icon buttons multiply their size by it. Set by [`Tokens::scaled`].
    pub zoom: f32,
    /// Terminal default background / foreground / cursor / selection.
    pub term_bg: Color32,
    pub term_fg: Color32,
    pub term_cursor: Color32,
    pub term_selection: Color32,
    /// The 16 ANSI colours (0-7 normal, 8-15 bright).
    pub ansi: [Color32; 16],
    /// Rainbow brackets: one colour per nesting depth, cycling (unmatched brackets use
    /// `error`). Each keeps 4.5:1 contrast on `bg` and `hover_bg` (tested).
    pub rainbow: [Color32; 6],
    /// Colours for code: the editor and the code preview (the user's editor scheme).
    pub code: CodePalette,
}

/// One colour scheme for code. Every colour keeps 4.5:1 contrast on `bg` and `line_bg`
/// (7:1 for the high-contrast scheme), tested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CodePalette {
    /// Editor and code-view background.
    pub bg: Color32,
    /// The caret's line.
    pub line_bg: Color32,
    /// Plain text and identifiers.
    pub text: Color32,
    /// Line numbers (the caret's line number uses `text`).
    pub gutter: Color32,
    pub keyword: Color32,
    pub string: Color32,
    pub escape: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub function: Color32,
    pub type_name: Color32,
    pub property: Color32,
    pub tag: Color32,
    pub punctuation: Color32,
    pub operator: Color32,
    pub heading: Color32,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl CodePalette {
    /// The app's palette, dark.
    pub const fn app_dark() -> Self {
        Self {
            bg: rgb(0x14161a),
            line_bg: rgb(0x242932),
            text: rgb(0xe6e8eb),
            gutter: rgb(0x7d8594),
            keyword: rgb(0xc49bf0),
            string: rgb(0xa6d189),
            escape: rgb(0x7fd8e8),
            comment: rgb(0x9aa2b4),
            number: rgb(0xf2a36b),
            function: rgb(0x82aaff),
            type_name: rgb(0xe8c47c),
            property: rgb(0x79d4c4),
            tag: rgb(0xf28b9b),
            punctuation: rgb(0xa3abbd),
            operator: rgb(0x7fd8e8),
            heading: rgb(0x82aaff),
        }
    }

    /// The app's palette, light.
    pub const fn app_light() -> Self {
        Self {
            bg: rgb(0xf7f8fa),
            line_bg: rgb(0xe9ecf0),
            text: rgb(0x1b1e23),
            gutter: rgb(0x6e7681),
            keyword: rgb(0x6d28d9),
            string: rgb(0x2f6b12),
            escape: rgb(0x0a6577),
            comment: rgb(0x596273),
            number: rgb(0xa43f06),
            function: rgb(0x1a55bd),
            type_name: rgb(0x7a5600),
            property: rgb(0x0b6b64),
            tag: rgb(0xb02a43),
            punctuation: rgb(0x4b5563),
            operator: rgb(0x0a6577),
            heading: rgb(0x1a55bd),
        }
    }

    /// Warm: soft grey background, orange keywords, green strings, teal numbers.
    pub const fn warm_dark() -> Self {
        Self {
            bg: rgb(0x1e1f22),
            line_bg: rgb(0x26282e),
            text: rgb(0xbcbec4),
            gutter: rgb(0x6f737a),
            keyword: rgb(0xcf8e6d),
            string: rgb(0x6aab73),
            escape: rgb(0xcf8e6d),
            comment: rgb(0x8f939a),
            number: rgb(0x2aacb8),
            function: rgb(0x56a8f5),
            type_name: rgb(0xbcbec4),
            property: rgb(0xc77dbb),
            tag: rgb(0xd5b778),
            punctuation: rgb(0xa1a3ab),
            operator: rgb(0xbcbec4),
            heading: rgb(0x56a8f5),
        }
    }

    /// Warm, light.
    pub const fn warm_light() -> Self {
        Self {
            bg: rgb(0xffffff),
            line_bg: rgb(0xf5f8fe),
            text: rgb(0x080808),
            gutter: rgb(0x8c8f94),
            keyword: rgb(0x0033b3),
            string: rgb(0x067d17),
            escape: rgb(0x0037a6),
            comment: rgb(0x6b6f75),
            number: rgb(0x1750eb),
            function: rgb(0x00627a),
            type_name: rgb(0x080808),
            property: rgb(0x871094),
            tag: rgb(0x0033b3),
            punctuation: rgb(0x3c3f45),
            operator: rgb(0x080808),
            heading: rgb(0x0033b3),
        }
    }

    /// High contrast, dark: black background, 7:1 or more for every colour.
    pub const fn contrast_dark() -> Self {
        Self {
            bg: rgb(0x000000),
            line_bg: rgb(0x1a1a1a),
            text: rgb(0xffffff),
            gutter: rgb(0xbdbdbd),
            keyword: rgb(0xffb86b),
            string: rgb(0xa6f28a),
            escape: rgb(0x7fe3ff),
            comment: rgb(0xc8c8c8),
            number: rgb(0xffd866),
            function: rgb(0x8cc8ff),
            type_name: rgb(0x7ff0d8),
            property: rgb(0xffa8d8),
            tag: rgb(0xff9e9e),
            punctuation: rgb(0xe6e6e6),
            operator: rgb(0xe6e6e6),
            heading: rgb(0x8cc8ff),
        }
    }

    /// High contrast, light: white background, 7:1 or more for every colour.
    pub const fn contrast_light() -> Self {
        Self {
            bg: rgb(0xffffff),
            line_bg: rgb(0xeef0f3),
            text: rgb(0x000000),
            gutter: rgb(0x4b5563),
            keyword: rgb(0x5b16b8),
            string: rgb(0x174f04),
            escape: rgb(0x004c61),
            comment: rgb(0x3f4652),
            number: rgb(0x8a3300),
            function: rgb(0x0a3fa8),
            type_name: rgb(0x004d47),
            property: rgb(0x7a1060),
            tag: rgb(0x9b1028),
            punctuation: rgb(0x1f2937),
            operator: rgb(0x1f2937),
            heading: rgb(0x0a3fa8),
        }
    }

    /// Rainbow brackets for the high-contrast scheme: 7:1 or more, like its other colours.
    pub const fn contrast_rainbow(dark: bool) -> [Color32; 6] {
        if dark {
            [
                rgb(0xffd866),
                rgb(0xffa8d8),
                rgb(0x8cc8ff),
                rgb(0xa6f28a),
                rgb(0xffb86b),
                rgb(0x7ff0d8),
            ]
        } else {
            [
                rgb(0x8a3300),
                rgb(0x5b16b8),
                rgb(0x0a3fa8),
                rgb(0x174f04),
                rgb(0x9b1028),
                rgb(0x004d47),
            ]
        }
    }

    /// The scheme's version for a dark or light app theme.
    pub const fn of(scheme: fm_engine::editor_prefs::EditorScheme, dark: bool) -> Self {
        use fm_engine::editor_prefs::EditorScheme as S;
        match (scheme, dark) {
            (S::App, true) => Self::app_dark(),
            (S::App, false) => Self::app_light(),
            (S::Warm, true) => Self::warm_dark(),
            (S::Warm, false) => Self::warm_light(),
            (S::HighContrast, true) => Self::contrast_dark(),
            (S::HighContrast, false) => Self::contrast_light(),
        }
    }

    /// Every foreground colour with its name (tests and the Preferences sample).
    pub fn foregrounds(&self) -> [(&'static str, Color32); 13] {
        [
            ("text", self.text),
            ("keyword", self.keyword),
            ("string", self.string),
            ("escape", self.escape),
            ("comment", self.comment),
            ("number", self.number),
            ("function", self.function),
            ("type", self.type_name),
            ("property", self.property),
            ("tag", self.tag),
            ("punctuation", self.punctuation),
            ("operator", self.operator),
            ("heading", self.heading),
        ]
    }
}

/// How far one wheel notch scrolls in the Preview pane and its editor, relative to egui's
/// default (before the user's Preferences › Preview › Scroll speed).
pub const WHEEL_BOOST: f32 = 3.0;

/// Scroll bars of the Preview pane and its editor: slim, rounded handles that stay faintly
/// visible (so the position is always readable) and widen under the pointer.
pub fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating: true,
        bar_width: SCROLL_BAR_WIDE,
        floating_width: SCROLL_BAR_SLIM,
        floating_allocated_width: 0.0,
        handle_min_length: 28.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 2.0,
        foreground_color: true,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.12,
        interact_background_opacity: 0.2,
        dormant_handle_opacity: SCROLL_HANDLE_REST,
        active_handle_opacity: 0.6,
        interact_handle_opacity: 0.85,
        ..egui::style::ScrollStyle::floating()
    }
}

/// Scroll handle width at rest and under the pointer (shared with the editor's own bars).
pub const SCROLL_BAR_SLIM: f32 = 5.0;
pub const SCROLL_BAR_WIDE: f32 = 10.0;
/// Handle opacity at rest (of the text colour).
pub const SCROLL_HANDLE_REST: f32 = 0.35;

/// Fixed backgrounds for pictures (SVG): the same in every theme, unlike the tokens.
pub const PICTURE_WHITE: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const PICTURE_BLACK: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
/// 50% grey.
pub const PICTURE_GREY: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);

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
            editing: Color32::from_rgb(0xf5, 0xa0, 0x4a),
            editing_text: Color32::from_rgb(0x1b, 0x1d, 0x23),
            success: Color32::from_rgb(0x63, 0xc1, 0x74),
            git_modified: Color32::from_rgb(0xe5, 0xc0, 0x7b),
            git_added: Color32::from_rgb(0x63, 0xc1, 0x74),
            git_removed: Color32::from_rgb(0xff, 0x6b, 0x6b),
            git_untracked: Color32::from_rgb(0x6f, 0xe3, 0xd0),
            git_ignored: Color32::from_rgb(0x7a, 0x80, 0x88),
            git_conflict: Color32::from_rgb(0xff, 0x8f, 0xd0),
            archive: Color32::from_rgb(0xb9, 0xa3, 0xff),
            diff_add_bg: Color32::from_rgb(0x1d, 0x3a, 0x26),
            diff_del_bg: Color32::from_rgb(0x44, 0x22, 0x26),
            diff_add_word: Color32::from_rgb(0x2b, 0x62, 0x3c),
            diff_del_word: Color32::from_rgb(0x7a, 0x30, 0x38),
            sync_conflict_bg: Color32::from_rgb(0x4a, 0x2a, 0x44),
            subtitle_text: Color32::from_rgb(0xff, 0xff, 0xff),
            subtitle_outline: Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0xe0),
            checker: [
                Color32::from_rgb(0xd4, 0xd6, 0xda),
                Color32::from_rgb(0xb4, 0xb8, 0xbe),
            ],
            pdf_gap: Color32::from_rgb(0x0e, 0x10, 0x13),
            pdf_page_border: Color32::from_rgb(0x44, 0x4a, 0x55),
            field_fill: Color32::from_rgba_unmultiplied(0x5c, 0xb2, 0xff, 0x38),
            field_focus: Color32::from_rgb(0x5c, 0xb2, 0xff),
            sig_valid: Color32::from_rgb(0x63, 0xc1, 0x74),
            sig_changed: Color32::from_rgb(0xff, 0xc8, 0x57),
            sig_broken: Color32::from_rgb(0xff, 0x6b, 0x6b),
            sig_ink: Color32::from_rgb(0x10, 0x1a, 0x3a),
            action_text: Color32::from_rgb(0x10, 0x12, 0x16),
            action_sign: Color32::from_rgb(0xb3, 0x9d, 0xff),
            action_place: Color32::from_rgb(0x6f, 0xe3, 0xd0),
            action_remember: Color32::from_rgb(0xff, 0x9b, 0xd2),
            action_neutral: Color32::from_rgb(0xb8, 0xbe, 0xc6),
            action_save: Color32::from_rgb(0x63, 0xc1, 0x74),
            action_save_as: Color32::from_rgb(0xa5, 0xd8, 0x8a),
            action_replace: Color32::from_rgb(0xff, 0xc8, 0x57),
            action_discard: Color32::from_rgb(0xff, 0x8a, 0x8a),
            pdf_contrast_fg: Color32::from_rgb(0xff, 0xff, 0x00),
            pdf_contrast_bg: Color32::from_rgb(0x00, 0x00, 0x00),
            row_height: 24.0,
            padding: 6.0,
            corner_radius: 6.0,
            border_width: 1.0,
            font_size: 14.0,
            term_font_size: 14.0,
            zoom: 1.0,
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
            rainbow: [
                Color32::from_rgb(0xe5, 0xc0, 0x7b),
                Color32::from_rgb(0xd1, 0x9c, 0xf0),
                Color32::from_rgb(0x6c, 0xb6, 0xff),
                Color32::from_rgb(0x8f, 0xd1, 0x6a),
                Color32::from_rgb(0xf0, 0xa3, 0x5e),
                Color32::from_rgb(0x56, 0xd4, 0xdd),
            ],
            code: CodePalette::app_dark(),
        }
        .with_archive_tint()
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
            editing: Color32::from_rgb(0xa0, 0x4e, 0x00),
            editing_text: Color32::from_rgb(0xff, 0xff, 0xff),
            success: Color32::from_rgb(0x1f, 0x7a, 0x33),
            git_modified: Color32::from_rgb(0x8a, 0x5c, 0x00),
            git_added: Color32::from_rgb(0x1f, 0x7a, 0x33),
            git_removed: Color32::from_rgb(0xc2, 0x2b, 0x2b),
            git_untracked: Color32::from_rgb(0x0a, 0x7d, 0x7d),
            git_ignored: Color32::from_rgb(0x80, 0x86, 0x90),
            git_conflict: Color32::from_rgb(0xa8, 0x2a, 0x8a),
            archive: Color32::from_rgb(0x5b, 0x3c, 0xc4),
            diff_add_bg: Color32::from_rgb(0xdf, 0xf5, 0xe3),
            diff_del_bg: Color32::from_rgb(0xfd, 0xe3, 0xe3),
            diff_add_word: Color32::from_rgb(0xa6, 0xe3, 0xb2),
            diff_del_word: Color32::from_rgb(0xf6, 0xb5, 0xb5),
            sync_conflict_bg: Color32::from_rgb(0xf6, 0xdc, 0xf0),
            // Drawn on the video, not on the light UI, so it stays light-on-dark.
            subtitle_text: Color32::from_rgb(0xff, 0xff, 0xfa),
            subtitle_outline: Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 0xe0),
            checker: [
                Color32::from_rgb(0xff, 0xff, 0xff),
                Color32::from_rgb(0xe2, 0xe4, 0xe8),
            ],
            pdf_gap: Color32::from_rgb(0xdc, 0xdf, 0xe4),
            pdf_page_border: Color32::from_rgb(0xb4, 0xb9, 0xc1),
            field_fill: Color32::from_rgba_unmultiplied(0x0b, 0x62, 0xd6, 0x26),
            field_focus: Color32::from_rgb(0x0b, 0x62, 0xd6),
            sig_valid: Color32::from_rgb(0x1f, 0x7a, 0x33),
            sig_changed: Color32::from_rgb(0x9a, 0x67, 0x00),
            sig_broken: Color32::from_rgb(0xc2, 0x2b, 0x2b),
            sig_ink: Color32::from_rgb(0x10, 0x1a, 0x3a),
            action_text: Color32::from_rgb(0xff, 0xff, 0xff),
            action_sign: Color32::from_rgb(0x5b, 0x3c, 0xc4),
            action_place: Color32::from_rgb(0x0a, 0x6e, 0x6e),
            action_remember: Color32::from_rgb(0xa8, 0x24, 0x6e),
            action_neutral: Color32::from_rgb(0x55, 0x5d, 0x69),
            action_save: Color32::from_rgb(0x1f, 0x7a, 0x33),
            action_save_as: Color32::from_rgb(0x2c, 0x5e, 0x3c),
            action_replace: Color32::from_rgb(0x8f, 0x55, 0x00),
            action_discard: Color32::from_rgb(0xc2, 0x2b, 0x2b),
            pdf_contrast_fg: Color32::from_rgb(0x00, 0x00, 0x00),
            pdf_contrast_bg: Color32::from_rgb(0xff, 0xff, 0xff),
            row_height: 24.0,
            padding: 6.0,
            corner_radius: 6.0,
            border_width: 1.0,
            font_size: 14.0,
            term_font_size: 14.0,
            zoom: 1.0,
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
            rainbow: [
                Color32::from_rgb(0x8a, 0x5a, 0x00),
                Color32::from_rgb(0x8a, 0x3f, 0xbf),
                Color32::from_rgb(0x0b, 0x5c, 0xc4),
                Color32::from_rgb(0x2a, 0x6e, 0x0e),
                Color32::from_rgb(0xa6, 0x4a, 0x0a),
                Color32::from_rgb(0x00, 0x72, 0x7a),
            ],
            code: CodePalette::app_light(),
        }
        .with_archive_tint()
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

    /// Syntax class -> colour, from the user's code scheme (the code preview).
    pub fn syntax(&self, class: fm_preview::Class) -> Color32 {
        use fm_preview::Class as C;
        let c = &self.code;
        match class {
            C::Plain => c.text,
            C::Punct => c.punctuation,
            C::Comment => c.comment,
            C::String => c.string,
            C::Number | C::Constant => c.number,
            C::Keyword => c.keyword,
            C::Function => c.function,
            C::Type => c.type_name,
            C::Key => c.property,
            C::Tag => c.tag,
            C::Escape => c.escape,
            C::Operator => c.operator,
            C::Heading => c.heading,
        }
    }

    /// Editor syntax token -> colour: the same scheme as the read-only code preview, so a
    /// file looks alike in both (capability `text-editor`).
    pub fn editor_syntax(&self, kind: fm_editor::Kind) -> Color32 {
        use fm_editor::Kind as K;
        let c = &self.code;
        match kind {
            K::Keyword => c.keyword,
            K::String => c.string,
            K::Escape | K::Link => c.escape,
            K::Comment => c.comment,
            K::Number | K::Constant | K::Emphasis => c.number,
            K::Function => c.function,
            K::Type => c.type_name,
            K::Property | K::Attribute => c.property,
            K::Variable => c.text,
            K::Operator => c.operator,
            K::Punctuation => c.punctuation,
            K::Tag => c.tag,
            K::Heading => c.heading,
        }
    }

    /// Rainbow bracket colour for a nesting depth (unmatched: the error colour).
    pub fn bracket(&self, depth: usize, unmatched: bool) -> Color32 {
        if unmatched {
            return self.error;
        }
        self.rainbow
            .get(depth % self.rainbow.len())
            .copied()
            .unwrap_or(self.text)
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
        // The archive colour follows the (readable) accent.
        t = t.with_archive_tint();
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
        t.code = CodePalette::of(prefs.editor.scheme, t.dark);
        if prefs.editor.scheme == fm_engine::editor_prefs::EditorScheme::HighContrast {
            t.rainbow = CodePalette::contrast_rainbow(t.dark);
        }
        t
    }

    /// Derive `archive` from `accent_text` (add-archive-vfs: archives in a variant of the
    /// accent colour).
    fn with_archive_tint(mut self) -> Self {
        self.archive = archive_tint(
            self.accent,
            self.accent_text,
            self.text,
            self.dark,
            [self.bg, self.panel, self.hover_bg],
        );
        self
    }

    /// Terminal font.
    pub fn term_font(&self) -> FontId {
        FontId::monospace(self.term_font_size)
    }

    /// Base font, sized from tokens.
    pub fn font(&self) -> FontId {
        FontId::monospace(self.font_size)
    }

    /// These tokens for a pane zoomed by `factor`: text, row height and spacing grow or
    /// shrink together; colours, borders and corners stay. The terminal font is left alone
    /// (terminals have their own zoom).
    pub fn scaled(&self, factor: f32) -> Self {
        let mut t = *self;
        t.font_size = (self.font_size * factor).max(4.0);
        t.row_height = (self.row_height * factor).round().max(8.0);
        t.padding = (self.padding * factor).max(1.0);
        t.zoom = self.zoom * factor;
        t
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

fn mix_toward(from: Color32, to: Color32, amount: f64) -> Color32 {
    let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * amount).round() as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

/// CIE L*a*b* (D65) of an sRGB colour, for telling colours apart by eye.
fn lab(color: Color32) -> [f64; 3] {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(color.r()), linear(color.g()), linear(color.b()));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |v: f64| {
        if v > 0.008856 {
            v.cbrt()
        } else {
            7.787 * v + 16.0 / 116.0
        }
    };
    [
        116.0 * f(y) - 16.0,
        500.0 * (f(x) - f(y)),
        200.0 * (f(y) - f(z)),
    ]
}

/// How different two colours look (CIE76 ΔE; about 2 is just noticeable).
fn delta_e(a: Color32, b: Color32) -> f64 {
    let (a, b) = (lab(a), lab(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Hue (0..1) and saturation (0..1) of a colour, HSL style.
fn hue_saturation(color: Color32) -> (f64, f64) {
    let [r, g, b] = [color.r(), color.g(), color.b()].map(|c| f64::from(c) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let lightness = (max + min) / 2.0;
    let delta = max - min;
    if delta <= f64::EPSILON {
        return (0.0, 0.0);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs()).max(f64::EPSILON);
    let hue = if (max - r).abs() <= f64::EPSILON {
        ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() <= f64::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    (hue / 6.0, saturation.min(1.0))
}

fn from_hsl(hue: f64, saturation: f64, lightness: f64) -> Color32 {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let h = hue * 6.0;
    let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let byte = |v: f64| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(byte(r), byte(g), byte(b))
}

/// The archive colour, a variant of the accent that reads clearly apart from both the
/// text and the accent itself, at 4.5:1 on every row surface.
///
/// Dark themes take the readable accent a step lighter. Light themes take the accent's
/// hue at full strength in the lightest shade that is still readable: darkening the
/// accent there drifts towards the near-black text. Each falls back to the other, and a
/// grey accent to whichever readable candidate is furthest from the text.
fn archive_tint(
    accent: Color32,
    accent_text: Color32,
    text: Color32,
    dark: bool,
    surfaces: [Color32; 3],
) -> Color32 {
    const FROM_TEXT: f64 = 30.0;
    const FROM_ACCENT: f64 = 12.0;
    let readable = |c: Color32| surfaces.iter().all(|bg| contrast(c, *bg) >= 4.5);
    let shade = mix_toward(
        accent_text,
        if dark { Color32::WHITE } else { Color32::BLACK },
        0.35,
    );
    // The accent's hue at full strength: the shade furthest from the text that is readable
    // and stays apart from the accent (preferably clearly, else just visibly).
    let (hue, saturation) = hue_saturation(accent);
    let vivid = |apart: f64| {
        (saturation >= 0.15)
            .then(|| {
                (1..100)
                    .map(|l| from_hsl(hue, saturation.max(0.85), f64::from(l) / 100.0))
                    .filter(|c| readable(*c) && delta_e(*c, accent_text) >= apart)
                    .max_by(|a, b| delta_e(*a, text).total_cmp(&delta_e(*b, text)))
            })
            .flatten()
    };
    let candidates = if dark {
        [Some(shade), vivid(FROM_ACCENT), vivid(8.0)]
    } else {
        [vivid(FROM_ACCENT), vivid(8.0), Some(shade)]
    };
    let good = |c: &Color32| {
        readable(*c) && delta_e(*c, text) >= FROM_TEXT && delta_e(*c, accent_text) >= 8.0
    };
    if let Some(c) = candidates.iter().flatten().find(|c| good(c)) {
        return *c;
    }
    candidates
        .iter()
        .flatten()
        .copied()
        .filter(|c| readable(*c))
        .max_by(|a, b| delta_e(*a, text).total_cmp(&delta_e(*b, text)))
        .unwrap_or(accent_text)
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

/// Smallest pointer target, in points (WCAG 2.2 SC 2.5.8 Target Size (Minimum): 24×24 CSS px).
pub const MIN_TARGET: f32 = 24.0;

/// Scale one Ui's text styles and spacing by `factor` (a zoomed pane). Only that Ui and its
/// children change; the context style and the other panes keep theirs.
pub fn scale_style(style: &mut egui::Style, factor: f32) {
    for font in style.text_styles.values_mut() {
        font.size = (font.size * factor).max(4.0);
    }
    let spacing = &mut style.spacing;
    spacing.item_spacing *= factor;
    spacing.button_padding *= factor;
    spacing.interact_size *= factor;
    spacing.icon_width *= factor;
    spacing.icon_width_inner *= factor;
    spacing.icon_spacing *= factor;
    spacing.indent *= factor;
    spacing.combo_width *= factor;
    spacing.text_edit_width *= factor;
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

/// `base` moved `t` (0..=1) of the way towards `toward`, as an opaque colour: tints derived
/// from the palette (e.g. an Activity card's background) without new literals elsewhere.
pub fn blend(base: Color32, toward: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(
        mix(base.r(), toward.r()),
        mix(base.g(), toward.g()),
        mix(base.b(), toward.b()),
    )
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// `preferred` when it has `minimum` contrast on `surface`, else `fallback`.
fn readable(preferred: Color32, fallback: Color32, surface: Color32, minimum: f64) -> Color32 {
    if contrast(preferred, surface) >= minimum {
        preferred
    } else {
        fallback
    }
}

impl Tokens {
    /// Mermaid diagram colours (change `markdown-mermaid`), so dark/light, contrast and
    /// accent settings apply. Text keeps 4.5:1 on every fill; lines and borders keep 3:1 on
    /// what they sit on (WCAG 1.4.3, 1.4.11).
    pub fn mermaid_palette(&self) -> fm_preview::mermaid::Palette {
        let background = self.panel;
        let node_fill = self.hover_bg;
        // Groups are told apart by their border; a fill of their own would leave edge labels
        // (drawn on `label_background`) as loose boxes inside them.
        let cluster_fill = background;
        let line = readable(self.text_muted, self.text, background, 3.0);
        let line = readable(line, self.text, cluster_fill, 3.0);
        let cluster_border = readable(self.border, line, cluster_fill, 3.0);
        let cluster_border = readable(cluster_border, line, background, 3.0);
        fm_preview::mermaid::Palette {
            dark: self.dark,
            background: hex(background),
            node_fill: hex(node_fill),
            node_border: hex(self.accent_text),
            text: hex(self.text),
            line: hex(line),
            cluster_fill: hex(cluster_fill),
            cluster_border: hex(cluster_border),
            label_background: hex(background),
            note_fill: hex(node_fill),
            note_border: hex(self.accent_text),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn blend_moves_part_of_the_way_and_keeps_the_ends() {
        let (a, b) = (Color32::from_rgb(0, 0, 0), Color32::from_rgb(200, 100, 50));
        assert_eq!(blend(a, b, 0.0), a);
        assert_eq!(blend(a, b, 1.0), b);
        assert_eq!(blend(a, b, 0.5), Color32::from_rgb(100, 50, 25));
        assert_eq!(blend(a, b, 2.0), b, "clamped");
    }

    #[test]
    fn every_code_scheme_is_readable() {
        use fm_engine::editor_prefs::EditorScheme;
        for scheme in EditorScheme::ALL {
            for dark in [true, false] {
                let p = CodePalette::of(scheme, dark);
                let need = if scheme == EditorScheme::HighContrast {
                    7.0
                } else {
                    4.5
                };
                for (name, c) in p.foregrounds() {
                    for bg in [p.bg, p.line_bg] {
                        assert!(
                            contrast(c, bg) >= need,
                            "{scheme:?} dark={dark} {name} {:.2}",
                            contrast(c, bg)
                        );
                    }
                }
                assert!(contrast(p.gutter, p.bg) >= 3.0, "{scheme:?} gutter");
                // Rainbow brackets stay readable on every scheme (7:1 in high contrast).
                let rainbow = if scheme == EditorScheme::HighContrast {
                    CodePalette::contrast_rainbow(dark)
                } else if dark {
                    Tokens::dark().rainbow
                } else {
                    Tokens::light().rainbow
                };
                for c in rainbow {
                    for bg in [p.bg, p.line_bg] {
                        assert!(contrast(c, bg) >= need, "{scheme:?} rainbow {c:?}");
                    }
                }
            }
        }
        let t = Tokens::dark();
        for (name, c) in t.code.foregrounds() {
            assert!(
                contrast(c, t.selection_bg) >= 3.0,
                "{name} on the selection"
            );
        }
    }

    #[test]
    fn rainbow_brackets_are_readable_in_both_themes() {
        for t in [Tokens::dark(), Tokens::light()] {
            for c in t.rainbow {
                for bg in [t.bg, t.hover_bg] {
                    assert!(contrast(c, bg) >= 4.5, "{c:?} on {bg:?}");
                }
            }
            assert_eq!(t.bracket(7, false), t.rainbow[1]);
            assert_eq!(t.bracket(0, true), t.error);
        }
    }

    /// The pane header's zoom button: icon and percentage in `text` on the header (`panel`) or
    /// its hover fill, zoomed outline and focus ring in `accent_text` (WCAG 2.2: 1.4.3 text
    /// 4.5:1, 1.4.11 non-text 3:1), for both themes and any accent.
    #[test]
    fn the_header_zoom_button_has_wcag_contrast_for_every_accent() {
        let mut all = vec![Tokens::dark(), Tokens::light()];
        for theme in [ThemePreference::Light, ThemePreference::Dark] {
            for r in [0, 64, 128, 192, 255] {
                for g in [0, 64, 128, 192, 255] {
                    for b in [0, 64, 128, 192, 255] {
                        let prefs = fm_engine::Prefs {
                            theme,
                            accent: Some(format!("#{r:02x}{g:02x}{b:02x}")),
                            ..Default::default()
                        };
                        all.push(Tokens::resolve_with(&prefs, false));
                    }
                }
            }
        }
        for t in all {
            for surface in [t.panel, t.hover_bg] {
                assert!(contrast(t.text, surface) >= 4.5, "text on {surface:?}");
                assert!(
                    contrast(t.accent_text, surface) >= 3.0,
                    "outline on {surface:?} (accent {:?})",
                    t.accent
                );
            }
        }
    }

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
                editing,
                editing_text,
                success,
                git_modified,
                git_added,
                git_removed,
                git_untracked,
                git_ignored,
                git_conflict,
                archive,
                diff_add_bg,
                diff_del_bg,
                diff_add_word,
                diff_del_word,
                sync_conflict_bg,
                subtitle_text,
                subtitle_outline,
                checker,
                pdf_gap,
                pdf_page_border,
                field_fill,
                field_focus,
                sig_valid,
                sig_changed,
                sig_broken,
                sig_ink,
                action_text,
                action_sign,
                action_place,
                action_remember,
                action_neutral,
                action_save,
                action_save_as,
                action_replace,
                action_discard,
                pdf_contrast_fg,
                pdf_contrast_bg,
                row_height,
                padding,
                corner_radius,
                border_width,
                font_size,
                term_font_size,
                zoom,
                term_bg,
                term_fg,
                term_cursor,
                term_selection,
                ansi,
                rainbow,
                code,
            } = tokens;
            assert_eq!(
                code.bg, bg,
                "the default code scheme sits on the app background"
            );
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
                ("editing", editing),
                ("editing_text", editing_text),
                ("success", success),
                ("git_modified", git_modified),
                ("git_added", git_added),
                ("git_removed", git_removed),
                ("git_untracked", git_untracked),
                ("git_ignored", git_ignored),
                ("git_conflict", git_conflict),
                ("archive", archive),
                ("diff_add_bg", diff_add_bg),
                ("diff_del_bg", diff_del_bg),
                ("diff_add_word", diff_add_word),
                ("diff_del_word", diff_del_word),
                ("sync_conflict_bg", sync_conflict_bg),
                ("term_bg", term_bg),
                ("term_fg", term_fg),
                ("term_cursor", term_cursor),
                ("term_selection", term_selection),
                ("subtitle_text", subtitle_text),
                ("subtitle_outline", subtitle_outline),
                ("checker light", checker[0]),
                ("checker dark", checker[1]),
                ("pdf_gap", pdf_gap),
                ("pdf_page_border", pdf_page_border),
                ("field_fill", field_fill),
                ("field_focus", field_focus),
                ("sig_valid", sig_valid),
                ("sig_changed", sig_changed),
                ("sig_broken", sig_broken),
                ("sig_ink", sig_ink),
                ("action_text", action_text),
                ("action_sign", action_sign),
                ("action_place", action_place),
                ("action_remember", action_remember),
                ("action_neutral", action_neutral),
                ("action_save", action_save),
                ("action_save_as", action_save_as),
                ("action_replace", action_replace),
                ("action_discard", action_discard),
                ("pdf_contrast_fg", pdf_contrast_fg),
                ("pdf_contrast_bg", pdf_contrast_bg),
            ] {
                assert_ne!(color.a(), 0, "{name} must be opaque");
            }
            assert!(row_height > 0.0 && padding > 0.0 && font_size > 0.0);
            assert!(term_font_size > 0.0);
            assert_eq!(zoom, 1.0);
            assert!(corner_radius >= 0.0 && border_width > 0.0);
            let _ = dark;
            assert!(ansi.iter().all(|c| c.a() == 255));
            assert!(rainbow.iter().all(|c| c.a() == 255));
            assert_ne!(ansi[1], ansi[2], "ANSI colours must be distinct");
        }
        check(Tokens::dark());
        check(Tokens::light());
    }

    /// The archive colour is a variant of the accent that stays readable and clearly apart
    /// from the text (light theme: it used to drift to near-black) and from the accent.
    #[test]
    fn archive_colour_follows_the_accent_for_every_accent_in_both_themes() {
        let dark = Tokens::dark();
        assert!(
            luminance(dark.archive) > luminance(dark.accent_text),
            "lighter on dark"
        );
        // The reported case: the light-blue default accent in the light theme.
        let prefs = fm_engine::Prefs {
            theme: ThemePreference::Light,
            accent: Some("#5cb2ff".into()),
            ..Default::default()
        };
        let light = Tokens::resolve_with(&prefs, false);
        assert!(
            delta_e(light.archive, light.text) >= 30.0,
            "{:?}",
            light.archive
        );
        for theme in [ThemePreference::Dark, ThemePreference::Light] {
            for r in (0..=255u8).step_by(51) {
                for g in (0..=255u8).step_by(51) {
                    for b in (0..=255u8).step_by(51) {
                        let prefs = fm_engine::Prefs {
                            theme,
                            accent: Some(format!("#{r:02x}{g:02x}{b:02x}")),
                            ..Default::default()
                        };
                        let t = Tokens::resolve_with(&prefs, false);
                        for surface in [t.bg, t.panel, t.hover_bg] {
                            assert!(
                                contrast(t.archive, surface) >= 4.5,
                                "archive {:?} on {surface:?} (accent {:?})",
                                t.archive,
                                t.accent
                            );
                        }
                        let (_, saturation) = hue_saturation(t.accent);
                        if saturation >= 0.15 {
                            assert!(
                                delta_e(t.archive, t.text) >= 25.0,
                                "archive {:?} too close to the text (accent {:?}, {theme:?})",
                                t.archive,
                                t.accent
                            );
                            assert!(
                                delta_e(t.archive, t.accent_text) >= 8.0,
                                "archive {:?} too close to accent {:?}",
                                t.archive,
                                t.accent_text
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn colour_helpers_round_trip() {
        let (h, s) = hue_saturation(Color32::from_rgb(0xff, 0x00, 0x00));
        assert!(h.abs() < 1e-9 && (s - 1.0).abs() < 1e-9);
        assert_eq!(from_hsl(0.0, 1.0, 0.5), Color32::from_rgb(0xff, 0x00, 0x00));
        assert_eq!(
            from_hsl(2.0 / 3.0, 1.0, 0.5),
            Color32::from_rgb(0x00, 0x00, 0xff)
        );
        assert!(delta_e(Color32::BLACK, Color32::WHITE) > 99.0);
        assert!(delta_e(Color32::BLACK, Color32::BLACK) < 1e-9);
    }

    #[test]
    fn archive_tint_is_readable_on_rows() {
        for t in [Tokens::dark(), Tokens::light()] {
            for (name, under) in [("bg", t.bg), ("panel", t.panel), ("hover_bg", t.hover_bg)] {
                let ratio = contrast(t.archive, under);
                assert!(
                    ratio >= 4.5,
                    "archive on {name} ({}): {ratio:.2}:1",
                    if t.dark { "dark" } else { "light" }
                );
            }
        }
    }

    #[test]
    fn fill_and_sign_tiles_are_readable() {
        for t in [Tokens::dark(), Tokens::light()] {
            for (name, fill) in [
                ("action_sign", t.action_sign),
                ("action_place", t.action_place),
                ("action_remember", t.action_remember),
                ("action_neutral", t.action_neutral),
                ("action_save", t.action_save),
                ("action_save_as", t.action_save_as),
                ("action_replace", t.action_replace),
                ("action_discard", t.action_discard),
                ("accent", t.accent),
                // The signature-problem square on the pages.
                ("sig_valid", t.sig_valid),
                ("sig_changed", t.sig_changed),
                ("sig_broken", t.sig_broken),
            ] {
                let ratio = contrast(t.action_text, fill);
                assert!(
                    ratio >= 4.5,
                    "{name} ({}): {ratio:.2}:1",
                    if t.dark { "dark" } else { "light" }
                );
            }
        }
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

    fn parse_hex(hex: &str) -> Color32 {
        let v = |i: usize| {
            hex.get(i..i + 2)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        };
        Color32::from_rgb(v(1).unwrap_or(0), v(3).unwrap_or(0), v(5).unwrap_or(0))
    }

    /// Every theme × accent: diagram text 4.5:1 on fills, lines and borders 3:1 on what they
    /// sit on.
    #[test]
    fn mermaid_colours_are_readable_for_every_accent_in_both_themes() {
        let mut all = vec![Tokens::light(), Tokens::dark()];
        for theme in [ThemePreference::Light, ThemePreference::Dark] {
            for r in [0, 128, 255] {
                for g in [0, 128, 255] {
                    for b in [0, 128, 255] {
                        let prefs = fm_engine::Prefs {
                            theme,
                            accent: Some(format!("#{r:02x}{g:02x}{b:02x}")),
                            ..Default::default()
                        };
                        all.push(Tokens::resolve_with(&prefs, false));
                    }
                }
            }
        }
        for t in all {
            let p = t.mermaid_palette();
            assert!(p.validate().is_ok());
            let c = |s: &String| parse_hex(s);
            for fill in [
                &p.node_fill,
                &p.background,
                &p.cluster_fill,
                &p.label_background,
            ] {
                assert!(contrast(c(&p.text), c(fill)) >= 4.5, "text on {fill}");
            }
            assert!(contrast(c(&p.line), c(&p.background)) >= 3.0, "lines");
            assert!(
                contrast(c(&p.line), c(&p.cluster_fill)) >= 3.0,
                "lines in groups"
            );
            assert!(
                contrast(c(&p.node_border), c(&p.node_fill)) >= 3.0,
                "node borders"
            );
            assert!(
                contrast(c(&p.node_border), c(&p.background)) >= 3.0,
                "node borders"
            );
            assert!(
                contrast(c(&p.cluster_border), c(&p.background)) >= 3.0,
                "group borders"
            );
            assert_eq!(p.dark, t.dark);
        }
        assert_ne!(
            Tokens::dark().mermaid_palette(),
            Tokens::light().mermaid_palette()
        );
    }
}
