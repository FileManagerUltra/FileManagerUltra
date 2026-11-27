//! Painted toolbar icons for the Preview pane and its Edit mode: the bars above previews,
//! the editor's toolbar, find bar and go-to bar. The bundled fonts have no symbol glyphs,
//! so every icon is drawn with the painter on a 16-unit grid, in theme colours, and grows
//! with the pane's zoom like the Media pane's buttons.

use egui::{Color32, Painter, Pos2, Rect, Response, Role, Sense, Shape, Stroke, Ui, WidgetInfo};
use egui::{pos2, vec2};

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    /// Back to the rendered preview.
    Preview,
    /// Open in the external editor.
    External,
    Save,
    Find,
    GoToLine,
    /// Live preview in another pane.
    Live,
    LineNumbers,
    Wrap,
    /// Overflow menu.
    Menu,
    Up,
    Down,
    Close,
    /// Open in Edit mode.
    Edit,
    Reload,
    /// Pretty-print structured data.
    Pretty,
    /// Compare / show changes.
    Diff,
    /// Show the source text of a rendered document.
    Source,
}

/// Hit area of an icon button at zoom 1.
pub(crate) const BUTTON: egui::Vec2 = vec2(30.0, 28.0);

/// A flat icon button: `label` is the tooltip and accessible name; `active` marks a
/// toggle that is on (accent tint), `enabled == false` greys it out.
pub(crate) fn button(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    enabled: bool,
    active: bool,
) -> Response {
    tool_button(ui, tokens, icon, label, label, enabled, active)
}

/// [`button`] with a short accessible `name` ("Wrap") and a longer tooltip `tip`.
pub(crate) fn tool_button(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    name: &str,
    tip: &str,
    enabled: bool,
    active: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(BUTTON * tokens.zoom, sense);
    response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, active, name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = tokens.corner_radius.max(4.0);
        if active {
            painter.rect_filled(rect, radius, tokens.selection_bg);
        }
        if enabled && response.hovered() {
            painter.rect_filled(rect, radius, tokens.hover_bg);
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.5, tokens.accent),
                egui::StrokeKind::Inside,
            );
        }
        let color = if !enabled {
            tokens.text_muted.gamma_multiply(0.5)
        } else if active {
            tokens.accent_text
        } else {
            tokens.text
        };
        paint(painter, icon, rect.center(), 16.0 * tokens.zoom, color);
    }
    if enabled {
        response.on_hover_text(tip)
    } else {
        response.on_disabled_hover_text(tip)
    }
}

/// A thin vertical divider between groups of icon buttons.
pub(crate) fn divider(ui: &mut Ui, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, BUTTON.y * tokens.zoom), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.shrink2(vec2(0.0, 6.0 * tokens.zoom)).y_range(),
        Stroke::new(1.0, tokens.border),
    );
}

/// The "unsaved changes" dot.
pub(crate) fn dirty_dot(ui: &mut Ui, tokens: &Tokens) -> Response {
    let side = 10.0 * tokens.zoom;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), side * 0.32, tokens.warning);
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, "Unsaved changes"));
    response.on_hover_text("Unsaved changes")
}

/// Paint `icon` centred on `c` in a square of side `size`.
pub(crate) fn paint(p: &Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let k = size / 16.0;
    let st = Stroke::new(1.5 * k.max(0.75), color);
    let at = |x: f32, y: f32| pos2(c.x + x * k, c.y + y * k);
    let line = |points: &[(f32, f32)]| {
        p.add(Shape::line(
            points.iter().map(|&(x, y)| at(x, y)).collect(),
            st,
        ));
    };
    match icon {
        Icon::Preview => {
            // An eye.
            let lid = |sign: f32| {
                (0..=10)
                    .map(|i| {
                        let x = -7.5 + 1.5 * i as f32;
                        at(x, sign * 4.8 * (1.0 - (x / 7.5).powi(2)))
                    })
                    .collect::<Vec<_>>()
            };
            p.add(Shape::line(lid(-1.0), st));
            p.add(Shape::line(lid(1.0), st));
            p.circle_filled(c, 2.2 * k, color);
        }
        Icon::External => {
            // A box with an arrow leaving it at the top right.
            line(&[
                (1.0, -6.0),
                (-6.0, -6.0),
                (-6.0, 6.0),
                (6.0, 6.0),
                (6.0, -1.0),
            ]);
            line(&[(-1.0, 1.0), (7.0, -7.0)]);
            line(&[(2.0, -7.0), (7.0, -7.0), (7.0, -2.0)]);
        }
        Icon::Save => {
            // A floppy disk.
            p.add(Shape::closed_line(
                vec![
                    at(-6.5, -6.5),
                    at(3.5, -6.5),
                    at(6.5, -3.5),
                    at(6.5, 6.5),
                    at(-6.5, 6.5),
                ],
                st,
            ));
            line(&[(-3.5, -6.5), (-3.5, -3.0), (2.0, -3.0), (2.0, -6.5)]);
            p.rect_stroke(
                Rect::from_min_max(at(-3.5, 1.5), at(3.5, 6.5)),
                0.0,
                st,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Find => {
            p.circle_stroke(at(-1.5, -1.5), 4.8 * k, st);
            p.line_segment([at(2.0, 2.0), at(6.5, 6.5)], Stroke::new(2.0 * k, color));
        }
        Icon::GoToLine => {
            // Lines of text with an arrow pointing at the middle one.
            for y in [-5.0, 5.0] {
                line(&[(0.0, y), (7.0, y)]);
            }
            p.line_segment([at(0.0, 0.0), at(7.0, 0.0)], Stroke::new(2.0 * k, color));
            line(&[(-7.0, 0.0), (-2.5, 0.0)]);
            line(&[(-5.0, -2.5), (-2.5, 0.0), (-5.0, 2.5)]);
        }
        Icon::Live => {
            // Two panes side by side, the right one "on air".
            p.rect_stroke(
                Rect::from_min_max(at(-7.5, -6.0), at(7.5, 6.0)),
                1.5 * k,
                st,
                egui::StrokeKind::Middle,
            );
            line(&[(0.0, -6.0), (0.0, 6.0)]);
            for y in [-2.5, 0.5, 3.5] {
                line(&[(-5.5, y), (-2.0, y)]);
            }
            p.circle_filled(at(3.8, 0.0), 2.0 * k, color);
        }
        Icon::LineNumbers => {
            // A column of numbers, a rule, then lines of text.
            for y in [-5.0, 0.0, 5.0] {
                p.line_segment([at(-6.0, y - 1.8), at(-6.0, y + 1.8)], st);
                line(&[(-1.0, y), (7.0, y)]);
            }
            p.line_segment(
                [at(-3.5, -7.0), at(-3.5, 7.0)],
                Stroke::new(st.width * 0.7, color.gamma_multiply(0.7)),
            );
        }
        Icon::Wrap => {
            // A line, then a line that turns back under itself.
            line(&[(-7.0, -5.0), (7.0, -5.0)]);
            line(&[(-7.0, 0.0), (4.0, 0.0)]);
            let arc: Vec<Pos2> = (0..=8)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 8.0;
                    at(4.0 + 2.5 * a.cos(), 2.5 + 2.5 * a.sin())
                })
                .collect();
            p.add(Shape::line(arc, st));
            line(&[(4.0, 5.0), (-1.0, 5.0)]);
            line(&[(1.5, 2.5), (-1.0, 5.0), (1.5, 7.5)]);
        }
        Icon::Menu => {
            for y in [-4.5, 0.0, 4.5] {
                line(&[(-6.5, y), (6.5, y)]);
            }
        }
        Icon::Up => line(&[(-5.5, 3.0), (0.0, -2.5), (5.5, 3.0)]),
        Icon::Down => line(&[(-5.5, -2.5), (0.0, 3.0), (5.5, -2.5)]),
        Icon::Close => {
            line(&[(-5.0, -5.0), (5.0, 5.0)]);
            line(&[(-5.0, 5.0), (5.0, -5.0)]);
        }
        Icon::Edit => {
            // A pencil, tip at the bottom left.
            p.add(Shape::closed_line(
                vec![
                    at(-6.5, 6.5),
                    at(-6.0, 3.0),
                    at(3.5, -6.5),
                    at(6.5, -3.5),
                    at(-3.0, 6.0),
                ],
                st,
            ));
            line(&[(1.0, -4.0), (4.0, -1.0)]);
        }
        Icon::Reload => {
            // A circular arrow: most of a circle, ending in an arrowhead at the right.
            let arc: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = 0.8 + (std::f32::consts::TAU - 0.8) * i as f32 / 16.0;
                    at(6.0 * a.cos(), -6.0 * a.sin())
                })
                .collect();
            p.add(Shape::line(arc, st));
            line(&[(3.6, 1.6), (6.0, -0.8), (8.0, 1.8)]);
        }
        Icon::Pretty => {
            // Braces around indented lines.
            line(&[
                (-4.0, -7.0),
                (-6.0, -7.0),
                (-6.0, -1.0),
                (-7.5, 0.0),
                (-6.0, 1.0),
                (-6.0, 7.0),
                (-4.0, 7.0),
            ]);
            line(&[
                (4.0, -7.0),
                (6.0, -7.0),
                (6.0, -1.0),
                (7.5, 0.0),
                (6.0, 1.0),
                (6.0, 7.0),
                (4.0, 7.0),
            ]);
            line(&[(-3.0, -3.0), (2.5, -3.0)]);
            line(&[(-1.0, 0.5), (3.0, 0.5)]);
            line(&[(-1.0, 4.0), (2.0, 4.0)]);
        }
        Icon::Diff => {
            // Two columns: a minus on the left, a plus on the right.
            p.rect_stroke(
                Rect::from_min_max(at(-7.5, -6.5), at(7.5, 6.5)),
                1.5 * k,
                st,
                egui::StrokeKind::Middle,
            );
            line(&[(0.0, -6.5), (0.0, 6.5)]);
            line(&[(-5.5, 0.0), (-2.0, 0.0)]);
            line(&[(2.0, 0.0), (5.5, 0.0)]);
            line(&[(3.75, -1.75), (3.75, 1.75)]);
        }
        Icon::Source => {
            // "</>"
            line(&[(-3.0, -4.5), (-7.0, 0.0), (-3.0, 4.5)]);
            line(&[(3.0, -4.5), (7.0, 0.0), (3.0, 4.5)]);
            line(&[(1.5, -6.5), (-1.5, 6.5)]);
        }
    }
}
