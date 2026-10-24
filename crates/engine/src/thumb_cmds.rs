//! `preview.thumbs.*`: thumbnail mode for directories shown in a Preview pane.
//!
//! The grid itself is drawn by `fm-ui`; it tells the engine which window of entries is on
//! screen (`preview.thumbs.visible`) and the engine queues just those (plus a screenful on
//! each side) on the thumbnail workers.

use std::sync::Arc;

use fm_layout::PaneId;
use fm_preview::Preview;
use fm_preview::thumbs::EDGES;
use fm_vfs::{Entry, FmError};

use crate::file_cmds::object;
use crate::preview_cmds::{Body, PreviewSlot, has_preview};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

const MAX_WINDOW: u64 = 10_000;
const SIZE_NAMES: [&str; 3] = ["small", "medium", "large"];

/// The preview's directory items, if the pane currently shows a folder.
fn dir_items(slot: &PreviewSlot) -> Option<(Arc<Preview>, Entry)> {
    let Body::Static(p) = &slot.body else {
        return None;
    };
    let subject = slot.subject.clone()?;
    matches!(&**p, Preview::Dir(_)).then(|| (p.clone(), subject))
}

fn slot_mut(s: &mut Session, pane: PaneId) -> &mut PreviewSlot {
    s.previews
        .slots
        .entry(pane)
        .or_insert_with(PreviewSlot::new)
}

/// Queue the entries around `window` (visible ones first, then a screenful after and before).
fn request(s: &mut Session, pane: PaneId, window: (usize, usize)) -> Result<(), FmError> {
    let Some(slot) = s.previews.slots.get_mut(&pane) else {
        return Ok(());
    };
    if !slot.thumbs {
        return Ok(());
    }
    slot.thumb_window = Some(window);
    let edge = EDGES.get(slot.thumb_size).copied().unwrap_or(EDGES[0]);
    let Some((preview, subject)) = dir_items(slot) else {
        return Ok(());
    };
    let Preview::Dir(doc) = &*preview else {
        return Ok(());
    };
    let (first, count) = window;
    let len = doc.items.len();
    let (vis_end, ahead_end) = (
        first.saturating_add(count).min(len),
        first.saturating_add(count.saturating_mul(2)).min(len),
    );
    let first = first.min(len);
    let behind = first.saturating_sub(count);
    let order: Vec<Entry> = doc.items[first..vis_end]
        .iter()
        .chain(&doc.items[vis_end..ahead_end])
        .chain(doc.items[behind..first].iter().rev())
        .cloned()
        .collect();
    let vfs = s.backends.require(&subject.uri)?;
    s.thumbs.want(pane, &vfs, &order, edge);
    Ok(())
}

/// `preview.thumbs.toggle {on?: bool, pane?: number}` — thumbnail grid for folders.
pub fn toggle() -> CommandSpec {
    CommandSpec {
        id: "preview.thumbs.toggle",
        label: "Preview Thumbnails",
        menu: &["View", "Preview Thumbnails"],
        shortcut: None,
        params: "{on?: bool, pane?: number}",
        enabled: has_preview,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let on = object(p)?
                .and_then(|m| m.get("on"))
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| FmError::InvalidParams("on: bool".into()))
                })
                .transpose()?;
            let slot = slot_mut(s, pane);
            slot.thumbs = on.unwrap_or(!slot.thumbs);
            slot.thumb_window = None;
            if !slot.thumbs {
                s.thumbs.cancel(pane);
            }
            Ok(())
        },
    }
}

/// `preview.thumbs.size {size?: "small"|"medium"|"large", pane?: number}`; no size cycles.
pub fn size() -> CommandSpec {
    CommandSpec {
        id: "preview.thumbs.size",
        label: "Thumbnail Size",
        menu: &["View", "Thumbnail Size"],
        shortcut: None,
        params: "{size?: \"small\"|\"medium\"|\"large\", pane?: number}",
        enabled: has_preview,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let named = match object(p)?.and_then(|m| m.get("size")) {
                None => None,
                Some(v) => {
                    let name = v.as_str().unwrap_or_default();
                    let ix = SIZE_NAMES.iter().position(|n| *n == name).ok_or_else(|| {
                        FmError::InvalidParams("size must be small, medium or large".into())
                    })?;
                    Some(ix)
                }
            };
            let slot = slot_mut(s, pane);
            slot.thumb_size = named.unwrap_or((slot.thumb_size + 1) % EDGES.len());
            let window = slot.thumb_window;
            if let Some(w) = window {
                request(s, pane, w)?;
            }
            Ok(())
        },
    }
}

/// `preview.thumbs.visible {first, count, pane?}` — sent by the grid when its window changes.
pub fn visible() -> CommandSpec {
    CommandSpec {
        id: "preview.thumbs.visible",
        label: "Thumbnail Window",
        menu: &["View", "Thumbnail Window"],
        shortcut: None,
        params: "{first: number, count: number (<= 10000), pane?: number}",
        enabled: has_preview,
        run: |s, p: &Params| {
            let pane = s.preview_pane(p)?;
            let num = |key: &str| {
                object(p)?
                    .and_then(|m| m.get(key))
                    .and_then(Params::as_u64)
                    .filter(|n| *n <= MAX_WINDOW)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| FmError::InvalidParams(format!("{key}: number <= 10000")))
            };
            let (first, count) = (num("first")?, num("count")?);
            request(s, pane, (first, count))
        },
    }
}

/// `preview.thumbs.clear_cache` — forget every thumbnail, in memory and on disk.
pub fn clear_cache() -> CommandSpec {
    CommandSpec {
        id: "preview.thumbs.clear_cache",
        label: "Clear Thumbnail Cache",
        menu: &["View", "Clear Thumbnail Cache"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            s.thumbs.clear_memory();
            s.thumbs.clear_disk();
            let panes: Vec<(PaneId, Option<(usize, usize)>)> = s
                .previews
                .slots
                .iter()
                .map(|(id, sl)| (*id, sl.thumb_window))
                .collect();
            for (pane, window) in panes {
                if let Some(w) = window {
                    request(s, pane, w)?;
                }
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![toggle(), size(), visible(), clear_cache()]
}
