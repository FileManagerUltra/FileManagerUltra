//! Drag & drop UI: the payload, drop-zone registration and the hover feedback. The UI only
//! *reports* `(sources, destination, modifiers)`; the engine's `resolve_drop` decides what
//! happens (architecture §8) and the drop is dispatched as `dnd.drop`.

use egui::{Context, CursorIcon, DragAndDrop, Id, Key, LayerId, Order, Pos2, Rect};
use fm_engine::{DropAction, DropModifiers, DropOrigin, DropResolution, Session};
use fm_layout::PaneId;
use fm_vfs::VfsUri;
use serde_json::json;

use crate::theme::Tokens;

/// What a drag carries: the rows being dragged and the pane they came from.
#[derive(Clone, Debug, PartialEq)]
pub struct DragPayload {
    pub sources: Vec<VfsUri>,
    pub origin: PaneId,
}

/// Priority of a zone: a specific row beats the pane's background.
pub const ZONE_BACKGROUND: u8 = 1;
pub const ZONE_ROW: u8 = 2;

#[derive(Clone, Debug)]
pub(crate) struct Zone {
    pub rect: Rect,
    pub dest: VfsUri,
    pub priority: u8,
}

#[derive(Clone, Default)]
struct Zones(Vec<Zone>);

fn zones_id() -> Id {
    Id::NULL.with("fm-drop-zones")
}

/// Declare `rect` as a drop target for `dest` for this frame.
pub fn register_zone(ctx: &Context, rect: Rect, dest: VfsUri, priority: u8) {
    if !DragAndDrop::has_payload_of_type::<DragPayload>(ctx) {
        return;
    }
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Zones>(zones_id()).0.push(Zone {
            rect,
            dest,
            priority,
        })
    });
}

#[derive(Clone, Default)]
struct PaneRects(Vec<(PaneId, Rect)>);

/// Remember where a pane was drawn this frame, so OS file drops can find the pane under the
/// pointer.
pub fn note_pane(ctx: &Context, pane: PaneId, rect: Rect) {
    ctx.data_mut(|d| {
        let rects = &mut d
            .get_temp_mut_or_default::<PaneRects>(Id::NULL.with("fm-pane-rects"))
            .0;
        rects.retain(|(p, _)| *p != pane);
        rects.push((pane, rect));
    });
}

/// The pane drawn under `pos` last frame.
pub fn pane_at(ctx: &Context, pos: Pos2) -> Option<PaneId> {
    ctx.data(|d| {
        d.get_temp::<PaneRects>(Id::NULL.with("fm-pane-rects"))
            .and_then(|r| {
                r.0.iter()
                    .find(|(_, rect)| rect.contains(pos))
                    .map(|(p, _)| *p)
            })
    })
}

fn take_zones(ctx: &Context) -> Vec<Zone> {
    ctx.data_mut(|d| std::mem::take(&mut d.get_temp_mut_or_default::<Zones>(zones_id()).0))
}

/// The most specific zone under `pos` (later registrations win ties).
pub(crate) fn pick_zone(zones: &[Zone], pos: Pos2) -> Option<&Zone> {
    zones
        .iter()
        .rev()
        .filter(|z| z.rect.contains(pos))
        .max_by_key(|z| z.priority)
}

/// The payload for dragging the row `name`: the whole selection when the row is part of
/// it, otherwise just that row.
pub fn payload_for(s: &Session, pane: PaneId, name: &str, uri: &VfsUri) -> DragPayload {
    let sources = s
        .layout
        .state(pane)
        .filter(|st| st.listing.is_selected(name))
        .map(|st| {
            st.listing
                .selection()
                .filter_map(|n| st.location.join(n).ok())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![uri.clone()]);
    DragPayload {
        sources,
        origin: pane,
    }
}

/// `Copy 12 items` / `Move 1 item`.
pub fn badge_text(action: DropAction, count: usize) -> String {
    let verb = match action {
        DropAction::Copy => "Copy",
        DropAction::Move => "Move",
    };
    if count == 1 {
        format!("{verb} 1 item")
    } else {
        format!("{verb} {count} items")
    }
}

fn paint_feedback(ctx: &Context, rect: Rect, pos: Pos2, text: &str, tokens: &Tokens) {
    let layer = LayerId::new(Order::Foreground, Id::NULL.with("fm-drop-outline"));
    ctx.layer_painter(layer).rect_stroke(
        rect.shrink(1.0),
        tokens.corner_radius,
        egui::Stroke::new(tokens.border_width.max(2.0), tokens.accent),
        egui::StrokeKind::Inside,
    );
    egui::Area::new(Id::NULL.with("fm-drop-badge"))
        .order(Order::Tooltip)
        .fixed_pos(pos + egui::vec2(14.0, 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(tokens.panel)
                .stroke(egui::Stroke::new(tokens.border_width, tokens.accent))
                .corner_radius(tokens.corner_radius)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .color(tokens.text)
                            .font(tokens.font()),
                    );
                });
        });
}

/// Resolve and draw the hovered drop target of one pane, and dispatch `dnd.drop` on
/// release. Call once at the end of each pane's frame, after its rows registered zones.
pub fn finish(ctx: &Context, s: &mut Session, pane_rect: Rect, tokens: &Tokens) {
    let zones = take_zones(ctx);
    let Some(payload) = DragAndDrop::payload::<DragPayload>(ctx) else {
        return;
    };
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        DragAndDrop::clear_payload(ctx);
        return;
    }
    let Some(pos) = ctx.pointer_interact_pos() else {
        return;
    };
    let Some(zone) = pick_zone(&zones, pos) else {
        if pane_rect.contains(pos) {
            ctx.set_cursor_icon(CursorIcon::NoDrop);
        }
        return;
    };
    let (mods, released) = ctx.input(|i| {
        (
            DropModifiers {
                ctrl: i.modifiers.ctrl,
                shift: i.modifiers.shift,
            },
            i.pointer.any_released(),
        )
    });
    let dest = zone.dest.clone();
    let rect = zone.rect;
    match s.preview_drop(&payload.sources, &dest, mods, DropOrigin::Internal) {
        DropResolution::Do(action) => {
            paint_feedback(
                ctx,
                rect,
                pos,
                &badge_text(action, payload.sources.len()),
                tokens,
            );
            if released && DragAndDrop::take_payload::<DragPayload>(ctx).is_some() {
                let sources: Vec<String> = payload.sources.iter().map(|u| u.to_string()).collect();
                s.dispatch_or_notice(
                    "dnd.drop",
                    &json!({
                        "sources": sources,
                        "dest": dest.to_string(),
                        "ctrl": mods.ctrl,
                        "shift": mods.shift,
                    }),
                );
            }
        }
        DropResolution::NoDrop(_) => ctx.set_cursor_icon(CursorIcon::NoDrop),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> VfsUri {
        VfsUri::parse(s).expect("uri")
    }

    fn zone(x0: f32, x1: f32, dest: &str, priority: u8) -> Zone {
        Zone {
            rect: Rect::from_min_max(Pos2::new(x0, 0.0), Pos2::new(x1, 10.0)),
            dest: u(dest),
            priority,
        }
    }

    #[test]
    fn badge_text_pluralises() {
        assert_eq!(badge_text(DropAction::Copy, 12), "Copy 12 items");
        assert_eq!(badge_text(DropAction::Move, 3), "Move 3 items");
        assert_eq!(badge_text(DropAction::Move, 1), "Move 1 item");
    }

    #[test]
    fn rows_beat_the_background() {
        let zones = [
            zone(0.0, 100.0, "/bg", ZONE_BACKGROUND),
            zone(10.0, 50.0, "/row", ZONE_ROW),
        ];
        assert_eq!(
            pick_zone(&zones, Pos2::new(20.0, 5.0)).map(|z| z.dest.clone()),
            Some(u("/row"))
        );
        assert_eq!(
            pick_zone(&zones, Pos2::new(80.0, 5.0)).map(|z| z.dest.clone()),
            Some(u("/bg"))
        );
        assert!(pick_zone(&zones, Pos2::new(200.0, 5.0)).is_none());
    }

    #[test]
    fn dragging_a_selected_row_carries_the_selection() {
        let mut s = Session::default();
        let loc = u("/d");
        s.layout.state_mut(1).expect("pane").location = loc.clone();
        {
            let listing = &mut s.layout.state_mut(1).expect("pane").listing;
            listing.select_only(Some("a"), 0);
            listing.toggle("b", 1);
            listing.toggle("c", 2);
        }
        let p = payload_for(&s, 1, "b", &u("/d/b"));
        assert_eq!(p.sources.len(), 3);
        // An unselected row is carried alone.
        let p = payload_for(&s, 1, "z", &u("/d/z"));
        assert_eq!(p.sources, vec![u("/d/z")]);
        assert_eq!(p.origin, 1);
    }
}
