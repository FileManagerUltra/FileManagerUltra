//! Painted transport icons and thin bars for the Media pane. The bundled fonts have no
//! media glyphs, so every symbol is drawn with the painter in theme colours.

use egui::{Color32, Pos2, Rect, Response, Role, Sense, Shape, Stroke, Ui, WidgetInfo, pos2, vec2};

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    Prev,
    Play,
    Pause,
    Stop,
    Next,
    FrameBack,
    FrameForward,
    Fullscreen,
    Shuffle,
    Repeat,
    RepeatOne,
    Volume,
    Muted,
    Menu,
    NewFolder,
    Clear,
    SkipBack,
    SkipForward,
    ChapterPrev,
    ChapterNext,
    /// List toolbars (Preferences > Preview > File types).
    Add,
    Remove,
    Edit,
    MoveUp,
    MoveDown,
}

pub(crate) const BUTTON: egui::Vec2 = vec2(28.0, 26.0);

/// A flat icon button. `label` is both the tooltip and the accessible name; `active` shows
/// toggles (shuffle, repeat, mute) in the accent colour.
pub(crate) fn button(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    enabled: bool,
    active: bool,
) -> Response {
    sized_button(ui, tokens, icon, label, enabled, active, BUTTON)
}

/// `button` with a custom hit area, e.g. to fit inside a single text row.
pub(crate) fn sized_button(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    label: &str,
    enabled: bool,
    active: bool,
    size: egui::Vec2,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if enabled && response.hovered() {
            painter.rect_filled(rect, tokens.corner_radius, tokens.hover_bg);
        } else if active {
            painter.rect_filled(rect, tokens.corner_radius, tokens.selection_bg);
        }
        let color = if !enabled {
            tokens.text_muted.gamma_multiply(0.5)
        } else if active {
            tokens.accent_text
        } else {
            tokens.text
        };
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                tokens.corner_radius,
                Stroke::new(1.5, tokens.accent),
                egui::StrokeKind::Inside,
            );
        }
        draw(painter, icon, rect.center(), color);
    }
    response.on_hover_text(label)
}

/// A closed padlock about 10x13 px, centred on `c`; `hole` is the body colour behind the keyhole.
pub(crate) fn paint_lock(painter: &egui::Painter, c: Pos2, color: Color32, hole: Color32) {
    let body = Rect::from_min_max(pos2(c.x - 5.0, c.y - 1.0), pos2(c.x + 5.0, c.y + 6.0));
    let radius = 3.2;
    let top = c.y - 1.0;
    let mut shackle = vec![pos2(c.x - radius, top)];
    let arch = 10;
    for step in 0..=arch {
        let angle = std::f32::consts::PI * (1.0 + step as f32 / arch as f32);
        shackle.push(pos2(
            c.x + radius * angle.cos(),
            top - 1.5 + radius * angle.sin(),
        ));
    }
    shackle.push(pos2(c.x + radius, top));
    painter.add(Shape::line(shackle, Stroke::new(1.6, color)));
    painter.rect_filled(body, 1.5, color);
    painter.circle_filled(pos2(c.x, c.y + 2.0), 1.2, hole);
    painter.rect_filled(
        Rect::from_center_size(pos2(c.x, c.y + 3.6), vec2(1.2, 2.0)),
        0.0,
        hole,
    );
}

fn tri(painter: &egui::Painter, points: [Pos2; 3], color: Color32) {
    painter.add(Shape::convex_polygon(points.to_vec(), color, Stroke::NONE));
}

fn right_tri(painter: &egui::Painter, left: f32, mid_y: f32, w: f32, h: f32, color: Color32) {
    tri(
        painter,
        [
            Pos2::new(left, mid_y - h / 2.0),
            Pos2::new(left, mid_y + h / 2.0),
            Pos2::new(left + w, mid_y),
        ],
        color,
    );
}

fn left_tri(painter: &egui::Painter, right: f32, mid_y: f32, w: f32, h: f32, color: Color32) {
    tri(
        painter,
        [
            Pos2::new(right, mid_y - h / 2.0),
            Pos2::new(right, mid_y + h / 2.0),
            Pos2::new(right - w, mid_y),
        ],
        color,
    );
}

fn bar_rect(painter: &egui::Painter, x: f32, y: f32, w: f32, h: f32, color: Color32) {
    painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), vec2(w, h)), 0.0, color);
}

fn arrow_head(painter: &egui::Painter, tip: Pos2, dir: f32, color: Color32) {
    tri(
        painter,
        [
            tip,
            tip + vec2(-3.5 * dir, -3.0),
            tip + vec2(-3.5 * dir, 3.0),
        ],
        color,
    );
}

fn draw(painter: &egui::Painter, icon: Icon, c: Pos2, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    match icon {
        Icon::Play => right_tri(painter, c.x - 4.0, c.y, 9.0, 11.0, color),
        Icon::Pause => {
            bar_rect(painter, c.x - 4.5, c.y - 5.0, 3.0, 10.0, color);
            bar_rect(painter, c.x + 1.5, c.y - 5.0, 3.0, 10.0, color);
        }
        Icon::Stop => bar_rect(painter, c.x - 4.5, c.y - 4.5, 9.0, 9.0, color),
        Icon::Prev => {
            bar_rect(painter, c.x - 6.0, c.y - 5.0, 2.0, 10.0, color);
            left_tri(painter, c.x + 5.5, c.y, 9.0, 10.0, color);
        }
        Icon::Next => {
            bar_rect(painter, c.x + 4.0, c.y - 5.0, 2.0, 10.0, color);
            right_tri(painter, c.x - 5.5, c.y, 9.0, 10.0, color);
        }
        Icon::SkipBack => {
            left_tri(painter, c.x, c.y, 7.0, 10.0, color);
            left_tri(painter, c.x + 7.0, c.y, 7.0, 10.0, color);
        }
        Icon::SkipForward => {
            right_tri(painter, c.x - 7.0, c.y, 7.0, 10.0, color);
            right_tri(painter, c.x, c.y, 7.0, 10.0, color);
        }
        Icon::ChapterPrev => {
            bar_rect(painter, c.x - 7.0, c.y - 5.0, 2.0, 10.0, color);
            bar_rect(painter, c.x - 3.5, c.y - 5.0, 2.0, 10.0, color);
            left_tri(painter, c.x + 7.0, c.y, 8.0, 10.0, color);
        }
        Icon::ChapterNext => {
            bar_rect(painter, c.x + 5.0, c.y - 5.0, 2.0, 10.0, color);
            bar_rect(painter, c.x + 1.5, c.y - 5.0, 2.0, 10.0, color);
            right_tri(painter, c.x - 7.0, c.y, 8.0, 10.0, color);
        }
        Icon::FrameBack => {
            left_tri(painter, c.x + 1.0, c.y, 7.0, 9.0, color);
            bar_rect(painter, c.x + 3.0, c.y - 4.5, 2.0, 9.0, color);
            bar_rect(painter, c.x - 7.0, c.y - 4.5, 1.5, 9.0, color);
        }
        Icon::FrameForward => {
            right_tri(painter, c.x - 1.0, c.y, 7.0, 9.0, color);
            bar_rect(painter, c.x - 5.0, c.y - 4.5, 2.0, 9.0, color);
            bar_rect(painter, c.x + 5.5, c.y - 4.5, 1.5, 9.0, color);
        }
        Icon::Fullscreen => {
            let (a, b) = (6.0, 3.5);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = c + vec2(sx * a, sy * a);
                painter.line_segment([corner, corner + vec2(-sx * b, 0.0)], stroke);
                painter.line_segment([corner, corner + vec2(0.0, -sy * b)], stroke);
            }
        }
        Icon::Shuffle => {
            let (l, r) = (c.x - 6.5, c.x + 5.0);
            painter.line_segment(
                [Pos2::new(l, c.y - 4.0), Pos2::new(r - 2.0, c.y + 4.0)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(l, c.y + 4.0), Pos2::new(r - 2.0, c.y - 4.0)],
                stroke,
            );
            arrow_head(painter, Pos2::new(r + 2.0, c.y + 4.0), 1.0, color);
            arrow_head(painter, Pos2::new(r + 2.0, c.y - 4.0), 1.0, color);
        }
        Icon::Repeat | Icon::RepeatOne => {
            let (l, r, t, b) = (c.x - 6.5, c.x + 6.5, c.y - 4.5, c.y + 4.5);
            painter.line_segment([Pos2::new(l, t), Pos2::new(r - 2.0, t)], stroke);
            painter.line_segment([Pos2::new(l, t), Pos2::new(l, b - 1.0)], stroke);
            painter.line_segment([Pos2::new(r, b), Pos2::new(l + 2.0, b)], stroke);
            painter.line_segment([Pos2::new(r, b), Pos2::new(r, t + 1.0)], stroke);
            arrow_head(painter, Pos2::new(r + 1.0, t), 1.0, color);
            arrow_head(painter, Pos2::new(l - 1.0, b), -1.0, color);
            if icon == Icon::RepeatOne {
                painter.line_segment(
                    [Pos2::new(c.x, c.y - 2.5), Pos2::new(c.x, c.y + 2.5)],
                    stroke,
                );
            }
        }
        Icon::Volume | Icon::Muted => {
            bar_rect(painter, c.x - 7.0, c.y - 2.5, 3.0, 5.0, color);
            tri(
                painter,
                [
                    Pos2::new(c.x - 4.0, c.y - 2.5),
                    Pos2::new(c.x + 1.0, c.y - 6.0),
                    Pos2::new(c.x + 1.0, c.y + 6.0),
                ],
                color,
            );
            tri(
                painter,
                [
                    Pos2::new(c.x - 4.0, c.y + 2.5),
                    Pos2::new(c.x - 4.0, c.y - 2.5),
                    Pos2::new(c.x + 1.0, c.y + 6.0),
                ],
                color,
            );
            if icon == Icon::Muted {
                let x = c + vec2(5.5, 0.0);
                painter.line_segment([x + vec2(-2.5, -2.5), x + vec2(2.5, 2.5)], stroke);
                painter.line_segment([x + vec2(-2.5, 2.5), x + vec2(2.5, -2.5)], stroke);
            } else {
                for radius in [3.0_f32, 6.0] {
                    let points: Vec<Pos2> = (-4..=4)
                        .map(|step| {
                            let angle = step as f32 * 0.16;
                            Pos2::new(c.x + 2.5 + radius * angle.cos(), c.y + radius * angle.sin())
                        })
                        .collect();
                    painter.add(Shape::line(points, Stroke::new(1.2, color)));
                }
            }
        }
        Icon::NewFolder => {
            let stroke = Stroke::new(1.5, color);
            let (x0, x1, top, bottom) = (c.x - 8.0, c.x + 8.0, c.y - 6.0, c.y + 6.0);
            painter.add(Shape::closed_line(
                vec![
                    pos2(x0, top),
                    pos2(x0 + 6.0, top),
                    pos2(x0 + 8.0, top + 2.5),
                    pos2(x1, top + 2.5),
                    pos2(x1, bottom),
                    pos2(x0, bottom),
                ],
                stroke,
            ));
            let plus = pos2(c.x, c.y + 1.5);
            painter.line_segment(
                [pos2(plus.x - 2.5, plus.y), pos2(plus.x + 2.5, plus.y)],
                stroke,
            );
            painter.line_segment(
                [pos2(plus.x, plus.y - 2.5), pos2(plus.x, plus.y + 2.5)],
                stroke,
            );
        }
        Icon::Clear => {
            let stroke = Stroke::new(1.5, color);
            for dy in [-4.5, 0.0, 4.5] {
                painter.line_segment(
                    [pos2(c.x - 3.0, c.y + dy), pos2(c.x + 7.0, c.y + dy)],
                    stroke,
                );
            }
            painter.line_segment(
                [pos2(c.x - 8.0, c.y - 3.0), pos2(c.x - 4.0, c.y + 3.0)],
                stroke,
            );
            painter.line_segment(
                [pos2(c.x - 8.0, c.y + 3.0), pos2(c.x - 4.0, c.y - 3.0)],
                stroke,
            );
        }
        Icon::Menu => {
            for dy in [-4.5, 0.0, 4.5] {
                bar_rect(painter, c.x - 6.0, c.y + dy - 1.0, 12.0, 2.0, color);
            }
        }
        Icon::Add => {
            painter.line_segment([pos2(c.x - 6.0, c.y), pos2(c.x + 6.0, c.y)], stroke);
            painter.line_segment([pos2(c.x, c.y - 6.0), pos2(c.x, c.y + 6.0)], stroke);
        }
        Icon::Remove => {
            painter.line_segment([pos2(c.x - 6.0, c.y), pos2(c.x + 6.0, c.y)], stroke);
        }
        Icon::Edit => {
            let stroke = Stroke::new(1.4, color);
            // A pencil from bottom-left to top-right: body outline, then the tip.
            let (tip, a, b) = (
                pos2(c.x - 6.0, c.y + 6.0),
                pos2(c.x - 4.0, c.y + 1.5),
                pos2(c.x - 1.5, c.y + 4.0),
            );
            let (a2, b2) = (pos2(c.x + 3.5, c.y - 6.0), pos2(c.x + 6.0, c.y - 3.5));
            painter.add(Shape::closed_line(vec![a, a2, b2, b], stroke));
            painter.add(Shape::line(vec![a, tip, b], stroke));
            painter.line_segment(
                [pos2(c.x + 1.5, c.y - 4.0), pos2(c.x + 4.0, c.y - 1.5)],
                stroke,
            );
        }
        Icon::MoveUp | Icon::MoveDown => {
            let dir = if icon == Icon::MoveUp { -1.0 } else { 1.0 };
            let head = pos2(c.x, c.y + 6.0 * dir);
            painter.line_segment([pos2(c.x, c.y - 6.0 * dir), head], stroke);
            painter.add(Shape::line(
                vec![
                    pos2(c.x - 4.5, head.y - 4.5 * dir),
                    head,
                    pos2(c.x + 4.5, head.y - 4.5 * dir),
                ],
                stroke,
            ));
        }
    }
}

/// A thin horizontal bar: the seek and volume control. Returns the new fraction while the
/// user drags or clicks it, and whether the interaction ended on this frame.
pub(crate) struct BarChange {
    pub fraction: f32,
    pub done: bool,
}

pub(crate) fn bar(
    ui: &mut Ui,
    tokens: &Tokens,
    width: f32,
    fraction: f32,
    enabled: bool,
    label: &str,
    value_text: &str,
) -> Option<BarChange> {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width.max(24.0), 16.0), sense);
    let fraction = fraction.clamp(0.0, 1.0);
    response.widget_info(|| {
        WidgetInfo::slider(
            enabled,
            f64::from(fraction),
            format!("{label} {value_text}"),
        )
    });
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, tokens.border);
    let filled = Rect::from_min_max(
        track.min,
        Pos2::new(track.min.x + track.width() * fraction, track.max.y),
    );
    let (fill, knob) = if enabled {
        (tokens.accent, tokens.accent_text)
    } else {
        (
            tokens.text_muted.gamma_multiply(0.4),
            tokens.text_muted.gamma_multiply(0.4),
        )
    };
    painter.rect_filled(filled, 2.0, fill);
    if enabled {
        let x = track.min.x + track.width() * fraction;
        let big = response.hovered() || response.dragged();
        painter.circle_filled(
            Pos2::new(x, rect.center().y),
            if big { 6.0 } else { 4.5 },
            knob,
        );
    }
    let change = if enabled && (response.dragged() || response.clicked() || response.drag_stopped())
    {
        response.interact_pointer_pos().map(|pos| BarChange {
            fraction: ((pos.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0),
            done: !response.dragged(),
        })
    } else {
        None
    };
    response.on_hover_text(format!("{label} {value_text}"));
    change
}

/// Gutter marker of a row with a remembered state: a half-filled dot where playback would
/// resume, a check mark once the book is finished.
pub(crate) fn paint_row_mark(
    painter: &egui::Painter,
    c: Pos2,
    mark: fm_engine::media_book::RowMark,
    color: Color32,
) {
    use fm_engine::media_book::RowMark;
    match mark {
        RowMark::Resume(_) => {
            painter.circle_stroke(c, 4.0, Stroke::new(1.3, color));
            let half: Vec<Pos2> = (0..=10)
                .map(|step| {
                    let angle =
                        std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * step as f32 / 10.0;
                    c + vec2(angle.cos(), angle.sin()) * 4.0
                })
                .collect();
            painter.add(Shape::convex_polygon(half, color, Stroke::NONE));
        }
        RowMark::Played | RowMark::Finished => {
            let stroke = Stroke::new(1.6, color);
            let knee = c + vec2(-1.0, 3.0);
            painter.line_segment([c + vec2(-4.0, 0.0), knee], stroke);
            painter.line_segment([knee, c + vec2(4.0, -3.5)], stroke);
        }
    }
}
