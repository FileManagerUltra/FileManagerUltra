//! Painted icons shared by the guided and the advanced sync windows, so the same thing looks the
//! same everywhere. Shapes only (no font glyphs); colours come from the theme tokens.

use egui::{Color32, Painter, Pos2, Stroke, Ui, Vec2, pos2, vec2};
use fm_sync::{CompareMethod, SyncMode};

use crate::theme::Tokens;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Icon {
    Folder,
    /// Copy new and changed files one way.
    Update,
    /// Make the target an exact copy.
    Mirror,
    TwoWay,
    Trash,
    Permanent,
    Clock,
    Hash,
    Funnel,
    Loop,
    List,
    Save,
    Check,
    Next,
    Back,
    /// An arrow pointing up: the parent folder.
    Up,
    /// A folder with a plus.
    NewFolder,
    /// An open eye: hidden items are shown.
    Eye,
    /// A struck-through eye: hidden items are left out.
    EyeOff,
    Compare,
    Swap,
    /// An open folder: load a saved sync.
    Open,
    /// Sliders: every option by hand.
    Sliders,
    /// Two diagonal arrows pointing outwards: show bigger.
    Expand,
    CopyRight,
    CopyLeft,
    /// A bin with a bar on the side that loses the file.
    DeleteLeft,
    DeleteRight,
    /// A circle with a slash: leave this item out of the run.
    Skip,
    NoAction,
    Equal,
    Conflict,
    Unknown,
}

impl Icon {
    pub(crate) fn mode(m: SyncMode) -> Self {
        match m {
            SyncMode::Update => Self::Update,
            SyncMode::Mirror => Self::Mirror,
            SyncMode::TwoWay => Self::TwoWay,
        }
    }

    /// The icon for a `sync.set_action` name.
    pub(crate) fn for_action(name: &str) -> Self {
        match name {
            "copy_to_right" => Self::CopyRight,
            "copy_to_left" => Self::CopyLeft,
            "delete_left" => Self::DeleteLeft,
            "delete_right" => Self::DeleteRight,
            "skip" => Self::Skip,
            _ => Self::NoAction,
        }
    }

    pub(crate) fn compare(m: CompareMethod) -> Self {
        match m {
            CompareMethod::SizeMtime => Self::Clock,
            _ => Self::Hash,
        }
    }

    /// The meaning colour: additions green, deletions red, both-ways accent, the rest neutral.
    pub(crate) fn color(self, tokens: &Tokens) -> Color32 {
        match self {
            Self::Update | Self::Check | Self::CopyRight | Self::CopyLeft => tokens.git_added,
            Self::Mirror | Self::Trash | Self::Permanent | Self::DeleteLeft | Self::DeleteRight => {
                tokens.git_removed
            }
            Self::Conflict => tokens.git_conflict,
            Self::Unknown => tokens.warning,
            Self::Skip | Self::NoAction | Self::Equal => tokens.text_muted,
            Self::TwoWay | Self::Loop | Self::Next | Self::Compare | Self::Swap => tokens.accent,
            Self::Funnel | Self::Hash => tokens.git_modified,
            Self::Open => tokens.git_modified,
            Self::Sliders | Self::Expand | Self::NewFolder | Self::Eye => tokens.accent,
            Self::Folder
            | Self::Clock
            | Self::List
            | Self::Save
            | Self::Back
            | Self::Up
            | Self::EyeOff => tokens.text_muted,
        }
    }
}

fn arrow(p: &Painter, c: Pos2, k: f32, dy: f32, right: bool, st: Stroke) {
    let d = if right { 1.0 } else { -1.0 };
    let y = c.y + dy * k;
    let (a, b) = (pos2(c.x - 6.0 * k * d, y), pos2(c.x + 6.0 * k * d, y));
    p.line_segment([a, b], st);
    p.line_segment([b, pos2(b.x - 3.5 * k * d, y - 3.5 * k)], st);
    p.line_segment([b, pos2(b.x - 3.5 * k * d, y + 3.5 * k)], st);
}

/// Paint `icon` centred on `c` inside a square of side `size`.
pub(crate) fn paint(p: &Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let k = size / 16.0;
    let st = Stroke::new(1.6 * k.max(0.75), color);
    let at = |x: f32, y: f32| pos2(c.x + x * k, c.y + y * k);
    match icon {
        Icon::Folder => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(-7.0, -4.0),
                    at(-7.0, 6.0),
                    at(7.0, 6.0),
                    at(7.0, -3.0),
                    at(1.0, -3.0),
                    at(-1.0, -6.0),
                    at(-5.0, -6.0),
                    at(-5.0, -4.0),
                ],
                st,
            ));
        }
        Icon::Update | Icon::Next | Icon::CopyRight => arrow(p, c, k, 0.0, true, st),
        Icon::Back | Icon::CopyLeft => arrow(p, c, k, 0.0, false, st),
        Icon::Up => {
            p.line_segment([at(-6.0, -7.0), at(6.0, -7.0)], st);
            p.line_segment([at(0.0, 7.0), at(0.0, -3.0)], st);
            p.add(egui::Shape::line(
                vec![at(-5.0, 1.0), at(0.0, -4.0), at(5.0, 1.0)],
                st,
            ));
        }
        Icon::Eye | Icon::EyeOff => {
            let lid = |sign: f32| {
                (0..=8)
                    .map(|i| {
                        let x = -8.0 + 2.0 * i as f32;
                        at(x, sign * 5.0 * (1.0 - (x / 8.0).powi(2)))
                    })
                    .collect::<Vec<_>>()
            };
            p.add(egui::Shape::line(lid(-1.0), st));
            p.add(egui::Shape::line(lid(1.0), st));
            p.circle_stroke(c, 2.0 * k, st);
            if icon == Icon::EyeOff {
                p.line_segment([at(-7.0, 7.0), at(7.0, -7.0)], st);
            }
        }
        Icon::NewFolder => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(-8.0, -5.0),
                    at(-8.0, 7.0),
                    at(8.0, 7.0),
                    at(8.0, -3.0),
                    at(1.5, -3.0),
                    at(-0.5, -6.0),
                    at(-6.0, -6.0),
                    at(-6.0, -5.0),
                ],
                st,
            ));
            p.line_segment([at(0.0, 0.0), at(0.0, 5.0)], st);
            p.line_segment([at(-2.5, 2.5), at(2.5, 2.5)], st);
        }
        Icon::DeleteLeft | Icon::DeleteRight => {
            let sx = if icon == Icon::DeleteLeft { -1.0 } else { 1.0 };
            p.line_segment([at(8.0 * sx, -6.0), at(8.0 * sx, 6.0)], st);
            let t = |x: f32, y: f32| at(x * 0.8 - 2.0 * sx, y * 0.85);
            p.line_segment([t(-7.0, -4.0), t(7.0, -4.0)], st);
            p.line_segment([t(-2.0, -4.0), t(-2.0, -7.0)], st);
            p.line_segment([t(2.0, -4.0), t(2.0, -7.0)], st);
            p.line_segment([t(-2.0, -7.0), t(2.0, -7.0)], st);
            p.add(egui::Shape::closed_line(
                vec![t(-5.0, -4.0), t(-4.0, 7.0), t(4.0, 7.0), t(5.0, -4.0)],
                st,
            ));
        }
        Icon::Skip => {
            p.circle_stroke(c, 6.5 * k, st);
            p.line_segment([at(-4.6, 4.6), at(4.6, -4.6)], st);
        }
        Icon::NoAction => {
            p.line_segment([at(-5.0, 0.0), at(5.0, 0.0)], st);
        }
        Icon::Equal => {
            p.line_segment([at(-6.0, -2.5), at(6.0, -2.5)], st);
            p.line_segment([at(-6.0, 2.5), at(6.0, 2.5)], st);
        }
        Icon::Conflict => {
            p.add(egui::Shape::closed_line(
                vec![at(0.0, -7.0), at(7.5, 6.0), at(-7.5, 6.0)],
                st,
            ));
            p.line_segment([at(0.0, -2.0), at(0.0, 2.0)], st);
            p.circle_filled(at(0.0, 4.2), 0.9 * k.max(0.75), color);
        }
        Icon::Unknown => {
            p.circle_stroke(c, 6.5 * k, st);
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(10.0 * k),
                color,
            );
        }
        Icon::Mirror => {
            arrow(p, c, k, 0.0, true, st);
            p.line_segment([at(8.0, -6.0), at(8.0, 6.0)], st);
        }
        Icon::TwoWay => {
            arrow(p, c, k, -3.5, true, st);
            arrow(p, c, k, 3.5, false, st);
        }
        Icon::Trash => {
            p.line_segment([at(-7.0, -4.0), at(7.0, -4.0)], st);
            p.line_segment([at(-2.0, -4.0), at(-2.0, -7.0)], st);
            p.line_segment([at(2.0, -4.0), at(2.0, -7.0)], st);
            p.line_segment([at(-2.0, -7.0), at(2.0, -7.0)], st);
            p.add(egui::Shape::closed_line(
                vec![at(-5.0, -4.0), at(-4.0, 7.0), at(4.0, 7.0), at(5.0, -4.0)],
                st,
            ));
        }
        Icon::Permanent => {
            p.line_segment([at(-5.0, -5.0), at(5.0, 5.0)], st);
            p.line_segment([at(-5.0, 5.0), at(5.0, -5.0)], st);
        }
        Icon::Clock => {
            p.circle_stroke(c, 7.0 * k, st);
            p.line_segment([c, at(0.0, -4.5)], st);
            p.line_segment([c, at(3.5, 1.0)], st);
        }
        Icon::Hash => {
            p.line_segment([at(-2.0, -7.0), at(-4.0, 7.0)], st);
            p.line_segment([at(4.0, -7.0), at(2.0, 7.0)], st);
            p.line_segment([at(-7.0, -2.5), at(7.0, -2.5)], st);
            p.line_segment([at(-7.0, 3.0), at(7.0, 3.0)], st);
        }
        Icon::Funnel => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(-7.0, -6.0),
                    at(7.0, -6.0),
                    at(1.5, 0.0),
                    at(1.5, 7.0),
                    at(-1.5, 5.0),
                    at(-1.5, 0.0),
                ],
                st,
            ));
        }
        Icon::Loop => {
            let pts: Vec<Pos2> = (0..=18)
                .map(|n| {
                    let a = -0.2 + 5.2 * n as f32 / 18.0;
                    at(6.5 * a.cos(), 6.5 * a.sin())
                })
                .collect();
            let end = pts.last().copied().unwrap_or(c);
            p.add(egui::Shape::line(pts, st));
            p.line_segment([end, end + vec2(-1.0, -4.5) * k], st);
            p.line_segment([end, end + vec2(4.0, -1.5) * k], st);
        }
        Icon::List => {
            for y in [-4.5, 0.0, 4.5] {
                p.line_segment([at(-3.0, y), at(7.0, y)], st);
                p.circle_filled(at(-6.0, y), 1.0 * k, color);
            }
        }
        Icon::Save => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(-7.0, -7.0),
                    at(4.0, -7.0),
                    at(7.0, -4.0),
                    at(7.0, 7.0),
                    at(-7.0, 7.0),
                ],
                st,
            ));
            p.rect_stroke(
                egui::Rect::from_min_max(at(-3.5, 1.0), at(3.5, 7.0)),
                0.0,
                st,
                egui::StrokeKind::Inside,
            );
            p.line_segment([at(-3.0, -7.0), at(-3.0, -3.0)], st);
            p.line_segment([at(-3.0, -3.0), at(2.0, -3.0)], st);
        }
        Icon::Check => {
            let st = Stroke::new(2.2 * k.max(0.75), color);
            p.line_segment([at(-6.0, 0.5), at(-2.0, 5.0)], st);
            p.line_segment([at(-2.0, 5.0), at(7.0, -5.0)], st);
        }
        Icon::Compare => {
            p.circle_stroke(at(-1.5, -1.5), 5.0 * k, st);
            p.line_segment([at(2.0, 2.0), at(7.0, 7.0)], st);
        }
        Icon::Expand => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, 1.0)] {
                let corner = at(7.0 * sx, 7.0 * sy);
                p.line_segment([at(1.5 * sx, 1.5 * sy), corner], st);
                p.line_segment([corner, at(7.0 * sx, 2.5 * sy)], st);
                p.line_segment([corner, at(2.5 * sx, 7.0 * sy)], st);
            }
        }
        Icon::Swap => {
            arrow(p, c, k, -3.5, true, st);
            arrow(p, c, k, 3.5, false, st);
        }
        Icon::Open => {
            // Back of the folder, then the lid swung open towards the viewer.
            p.add(egui::Shape::line(
                vec![
                    at(-7.0, 6.0),
                    at(-7.0, -6.0),
                    at(-2.0, -6.0),
                    at(0.0, -3.5),
                    at(5.0, -3.5),
                    at(5.0, -1.0),
                ],
                st,
            ));
            p.add(egui::Shape::closed_line(
                vec![
                    at(-5.0, 6.0),
                    at(-7.0, 6.0),
                    at(-4.0, -1.0),
                    at(8.0, -1.0),
                    at(5.0, 6.0),
                ],
                st,
            ));
        }
        Icon::Sliders => {
            for (y, x) in [(-4.5, -2.5), (0.0, 3.0), (4.5, -3.5)] {
                p.line_segment([at(-7.0, y), at(7.0, y)], st);
                p.circle_filled(at(x, y), 2.2 * k, st.color);
            }
        }
    }
}

/// An icon on its own line of text, in its meaning colour.
pub(crate) fn icon(ui: &mut Ui, tokens: &Tokens, icon: Icon, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint(ui.painter(), icon, rect.center(), size, icon.color(tokens));
}

/// A large floppy-disk illustration, used to make a card stand out. The badge is a green
/// check only once `saved`; before that it is a neutral "write" arrow.
pub(crate) fn save_illustration(ui: &mut Ui, tokens: &Tokens, size: f32, saved: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(
        rect,
        tokens.corner_radius * 2.0,
        tokens.accent.gamma_multiply(0.16),
    );
    let u = size / 100.0;
    let c = rect.center();
    let at = |x: f32, y: f32| pos2(c.x + x * u, c.y + y * u);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| egui::Rect::from_min_max(at(x0, y0), at(x1, y1));
    p.add(egui::Shape::convex_polygon(
        vec![
            at(-34.0, -36.0),
            at(18.0, -36.0),
            at(34.0, -20.0),
            at(34.0, 34.0),
            at(-34.0, 34.0),
        ],
        tokens.accent,
        Stroke::NONE,
    ));
    p.rect_filled(r(-20.0, -36.0, 14.0, -14.0), 2.0 * u, tokens.bg);
    p.rect_filled(r(4.0, -32.0, 10.0, -18.0), 1.0 * u, tokens.accent);
    p.rect_filled(r(-24.0, 2.0, 24.0, 34.0), 3.0 * u, tokens.bg);
    for y in [11.0, 19.0, 27.0] {
        p.line_segment(
            [at(-17.0, y), at(17.0, y)],
            Stroke::new(2.0 * u, tokens.accent.gamma_multiply(0.55)),
        );
    }
    let badge = at(30.0, 30.0);
    p.circle_filled(badge, 19.0 * u, tokens.hover_bg);
    let st = Stroke::new(3.4 * u, tokens.bg);
    if saved {
        p.circle_filled(badge, 15.0 * u, tokens.success);
        p.line_segment([at(22.0, 30.0), at(28.0, 36.0)], st);
        p.line_segment([at(28.0, 36.0), at(39.0, 24.0)], st);
    } else {
        p.circle_filled(badge, 15.0 * u, tokens.accent);
        p.line_segment([at(30.0, 22.0), at(30.0, 38.0)], st);
        p.line_segment([at(30.0, 38.0), at(23.0, 31.0)], st);
        p.line_segment([at(30.0, 38.0), at(37.0, 31.0)], st);
    }
}

fn button_font(tokens: &Tokens, primary: bool) -> egui::FontId {
    egui::FontId::proportional(tokens.font_size + if primary { 2.0 } else { 1.0 })
}

fn button_pad(primary: bool) -> f32 {
    if primary { 11.0 } else { 8.0 }
}

/// The height of a [`button`], for rows that must be exactly that tall.
pub(crate) fn button_height(ui: &Ui, tokens: &Tokens, primary: bool) -> f32 {
    let row = ui.fonts_mut(|f| f.row_height(&button_font(tokens, primary)));
    row + 2.0 * button_pad(primary) + 2.0
}

/// A button with an icon: filled with the accent colour when `primary`, plain otherwise.
pub(crate) fn button(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    primary: bool,
) -> egui::Response {
    let font = button_font(tokens, primary);
    let enabled = ui.is_enabled();
    let fade = if enabled { 1.0 } else { 0.45 };
    let text_color = if primary { tokens.bg } else { tokens.text }.gamma_multiply(fade);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, text_color);
    let pad = button_pad(primary);
    let size = vec2(galley.size().x + 52.0, galley.size().y + 2.0 * pad + 2.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let (fill, stroke) = if primary {
        let f = if response.hovered() {
            tokens.accent.gamma_multiply(0.85)
        } else {
            tokens.accent
        };
        (f, Stroke::NONE)
    } else {
        let v = ui.style().interact(&response);
        (v.weak_bg_fill, v.bg_stroke)
    };
    ui.painter().rect(
        rect,
        tokens.corner_radius,
        fill.gamma_multiply(fade),
        stroke,
        egui::StrokeKind::Inside,
    );
    let icon_color = if primary {
        tokens.bg
    } else {
        icon.color(tokens)
    };
    paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 22.0, rect.center().y),
        18.0,
        icon_color.gamma_multiply(fade),
    );
    ui.painter().galley(
        pos2(rect.left() + 38.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A menu row: the icon, then its label; `selected` marks the current choice.
pub(crate) fn menu_item(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    selected: bool,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(tokens.font_size),
        tokens.text,
    );
    let width = ui.available_width().max(galley.size().x + 52.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if selected {
        ui.painter()
            .rect_filled(rect, tokens.corner_radius, tokens.selection_bg);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, tokens.corner_radius, tokens.hover_bg);
    }
    paint(
        ui.painter(),
        icon,
        pos2(rect.left() + 16.0, rect.center().y),
        18.0,
        icon.color(tokens),
    );
    ui.painter().galley(
        pos2(rect.left() + 34.0, rect.center().y - galley.size().y / 2.0),
        galley,
        tokens.text,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A square button with only an icon; `label` is its accessible name and hover text.
pub(crate) fn icon_button(ui: &mut Ui, tokens: &Tokens, icon: Icon, label: &str) -> egui::Response {
    icon_button_sized(ui, tokens, icon, label, 44.0)
}

/// An icon-only button `side` points wide and high; the label is its tooltip and accessible name.
pub(crate) fn icon_button_sized(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    side: f32,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let fade = if enabled { 1.0 } else { 0.45 };
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let v = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        tokens.corner_radius,
        v.weak_bg_fill.gamma_multiply(fade),
        v.bg_stroke,
        egui::StrokeKind::Inside,
    );
    paint(
        ui.painter(),
        icon,
        rect.center(),
        if side > 40.0 { 22.0 } else { side * 0.58 },
        icon.color(tokens).gamma_multiply(fade),
    );
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A big square-ish button: the icon above a short label. `selected` gives it the accent frame.
pub(crate) fn tile(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    selected: bool,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let fade = if enabled { 1.0 } else { 0.45 };
    let font = egui::FontId::proportional(tokens.font_size);
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), font, tokens.text.gamma_multiply(fade));
    let size = vec2(
        galley.size().x.max(64.0) + 28.0,
        galley.size().y + 22.0 + 26.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    let visuals = ui.style().interact(&response);
    let (fill, stroke) = if selected {
        (
            tokens.accent.gamma_multiply(0.22),
            Stroke::new(2.0, tokens.accent),
        )
    } else {
        (visuals.weak_bg_fill, visuals.bg_stroke)
    };
    ui.painter().rect(
        rect,
        tokens.corner_radius,
        fill.gamma_multiply(fade),
        stroke,
        egui::StrokeKind::Inside,
    );
    paint(
        ui.painter(),
        icon,
        pos2(rect.center().x, rect.top() + 20.0),
        22.0,
        icon.color(tokens).gamma_multiply(fade),
    );
    ui.painter().galley(
        pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.bottom() - galley.size().y - 8.0,
        ),
        galley,
        tokens.text,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
