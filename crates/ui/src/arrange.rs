//! Moving panes by dragging their header (capability `pane-arrangement`): the payload, the
//! five drop targets (four edges and centre) and the overlay. The UI only reports
//! `(pane, target, edge)`; `pane.move` does the tree surgery.

use egui::{Context, CursorIcon, DragAndDrop, Id, Key, LayerId, Order, Pos2, Rect, Response};
use fm_engine::Session;
use fm_layout::{Edge, PaneId};
use serde_json::json;

use crate::theme::Tokens;

/// Fraction of a pane's size, from each side, that counts as that edge.
const EDGE_BAND: f32 = 0.25;

/// What a pane header drag carries (distinct from the file `DragPayload`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneDrag(pub PaneId);

/// Where a dragged pane would land on a target pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Edge(Edge),
    Center,
}

impl Slot {
    fn edge_name(self) -> &'static str {
        match self {
            Self::Edge(e) => e.as_str(),
            Self::Center => "center",
        }
    }
}

/// The drop slot for a pointer position over `rect`: the nearest edge when inside that
/// edge's band, otherwise the centre.
pub fn slot_for(rect: Rect, pos: Pos2) -> Slot {
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    let fx = ((pos.x - rect.left()) / w).clamp(0.0, 1.0);
    let fy = ((pos.y - rect.top()) / h).clamp(0.0, 1.0);
    let candidates = [
        (fx, Edge::Left),
        (1.0 - fx, Edge::Right),
        (fy, Edge::Top),
        (1.0 - fy, Edge::Bottom),
    ];
    let (dist, edge) =
        candidates.into_iter().fold(
            (f32::MAX, Edge::Left),
            |best, c| if c.0 < best.0 { c } else { best },
        );
    if dist <= EDGE_BAND {
        Slot::Edge(edge)
    } else {
        Slot::Center
    }
}

/// The area the dragged pane would occupy.
pub fn preview_rect(rect: Rect, slot: Slot) -> Rect {
    let c = rect.center();
    match slot {
        Slot::Center => rect.shrink(rect.width().min(rect.height()) * 0.08),
        Slot::Edge(Edge::Left) => Rect::from_min_max(rect.min, Pos2::new(c.x, rect.bottom())),
        Slot::Edge(Edge::Right) => Rect::from_min_max(Pos2::new(c.x, rect.top()), rect.max),
        Slot::Edge(Edge::Top) => Rect::from_min_max(rect.min, Pos2::new(rect.right(), c.y)),
        Slot::Edge(Edge::Bottom) => Rect::from_min_max(Pos2::new(rect.left(), c.y), rect.max),
    }
}

/// Make `grip` (the pane header) a drag source for `pane`.
pub fn source(ctx: &Context, grip: &Response, pane: PaneId) {
    grip.dnd_set_drag_payload(PaneDrag(pane));
    if grip.dragged() {
        ctx.set_cursor_icon(CursorIcon::Grabbing);
    } else if grip.hovered() {
        ctx.set_cursor_icon(CursorIcon::Grab);
    }
}

/// Draw the drop overlay over `rect` while a pane is dragged across it, and dispatch
/// `pane.move` on release. Call once per pane per frame (all pane kinds).
pub fn finish(ctx: &Context, s: &mut Session, pane: PaneId, rect: Rect, tokens: &Tokens) {
    let Some(payload) = DragAndDrop::payload::<PaneDrag>(ctx) else {
        return;
    };
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        DragAndDrop::clear_payload(ctx);
        return;
    }
    let source = payload.0;
    let Some(pos) = ctx.pointer_interact_pos().filter(|p| rect.contains(*p)) else {
        return;
    };
    if source == pane {
        ctx.set_cursor_icon(CursorIcon::NoDrop);
        return;
    }
    let slot = slot_for(rect, pos);
    let layer = LayerId::new(Order::Foreground, Id::NULL.with(("fm-pane-drop", pane)));
    let painter = ctx.layer_painter(layer);
    painter.rect_filled(
        preview_rect(rect, slot).shrink(2.0),
        tokens.corner_radius,
        tokens.accent.gamma_multiply(0.28),
    );
    painter.rect_stroke(
        preview_rect(rect, slot).shrink(2.0),
        tokens.corner_radius,
        egui::Stroke::new(tokens.border_width.max(2.0), tokens.accent),
        egui::StrokeKind::Inside,
    );
    if ctx.input(|i| i.pointer.any_released())
        && DragAndDrop::take_payload::<PaneDrag>(ctx).is_some()
    {
        s.dispatch_or_notice(
            "pane.move",
            &json!({"pane": source, "target": pane, "edge": slot.edge_name()}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_max(Pos2::new(100.0, 50.0), Pos2::new(500.0, 450.0))
    }

    #[test]
    fn pointer_bands_pick_edges_and_the_middle_is_centre() {
        let r = rect();
        assert_eq!(slot_for(r, Pos2::new(110.0, 250.0)), Slot::Edge(Edge::Left));
        assert_eq!(
            slot_for(r, Pos2::new(490.0, 250.0)),
            Slot::Edge(Edge::Right)
        );
        assert_eq!(slot_for(r, Pos2::new(300.0, 60.0)), Slot::Edge(Edge::Top));
        assert_eq!(
            slot_for(r, Pos2::new(300.0, 440.0)),
            Slot::Edge(Edge::Bottom)
        );
        assert_eq!(slot_for(r, Pos2::new(300.0, 250.0)), Slot::Center);
    }

    #[test]
    fn corners_choose_the_nearest_edge() {
        let r = rect();
        assert_eq!(slot_for(r, Pos2::new(105.0, 120.0)), Slot::Edge(Edge::Left));
        assert_eq!(slot_for(r, Pos2::new(150.0, 55.0)), Slot::Edge(Edge::Top));
    }

    #[test]
    fn degenerate_rects_do_not_panic() {
        let r = Rect::from_min_max(Pos2::new(5.0, 5.0), Pos2::new(5.0, 5.0));
        let _ = slot_for(r, Pos2::new(5.0, 5.0));
        let _ = slot_for(r, Pos2::new(f32::NAN, 1.0));
    }

    #[test]
    fn previews_cover_the_half_the_pane_would_take() {
        let r = rect();
        let left = preview_rect(r, Slot::Edge(Edge::Left));
        assert_eq!(left.width(), r.width() / 2.0);
        assert_eq!(left.height(), r.height());
        let bottom = preview_rect(r, Slot::Edge(Edge::Bottom));
        assert_eq!(bottom.height(), r.height() / 2.0);
        assert!(r.contains_rect(preview_rect(r, Slot::Center)));
    }
}
