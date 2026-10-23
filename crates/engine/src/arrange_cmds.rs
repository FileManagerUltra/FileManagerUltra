//! Pane arrangement: move, equalize, zoom, rotate, keyboard resize (capability `pane-arrangement`).
//! Pure layout surgery; per-pane state and terminals are keyed by `PaneId` and travel along.

use fm_layout::{Axis, Edge, PaneId};
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Default keyboard resize step, as a fraction of the split.
pub const RESIZE_STEP: f32 = 0.05;
const MAX_STEP: f32 = 0.5;

fn pane_param(map: &crate::file_cmds::Obj, key: &str) -> Result<PaneId, FmError> {
    map.get(key)
        .and_then(Params::as_u64)
        .ok_or_else(|| FmError::InvalidParams(format!("{key}: pane id required")))
}

fn many_panes(s: &Session) -> bool {
    s.layout.panes().len() > 1
}

/// `pane.move {pane, target, edge}` — `edge` is `left|right|top|bottom|center` (swap).
pub fn move_pane() -> CommandSpec {
    CommandSpec {
        id: "pane.move",
        label: "Move Pane",
        menu: &["Panes", "Move Pane"],
        shortcut: None,
        params: "{pane: number, target: number, edge: \"left\"|\"right\"|\"top\"|\"bottom\"|\"center\"}",
        enabled: many_panes,
        run: |s, p: &Params| {
            let map = object(p)?
                .ok_or_else(|| FmError::InvalidParams("pane, target and edge required".into()))?;
            let pane = pane_param(map, "pane")?;
            let target = pane_param(map, "target")?;
            let edge = map
                .get("edge")
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("edge: string required".to_string()))?;
            if edge == "center" {
                s.layout.swap(pane, target)?;
                return s.layout.focus_pane(pane);
            }
            let edge = Edge::parse(edge).ok_or_else(|| {
                FmError::InvalidParams("edge must be left, right, top, bottom or center".into())
            })?;
            s.layout.move_pane(pane, target, edge)
        },
    }
}

/// `pane.equalize`
pub fn equalize() -> CommandSpec {
    CommandSpec {
        id: "pane.equalize",
        label: "Equalize Panes",
        menu: &["Panes", "Equalize"],
        shortcut: Some("Ctrl+Shift+B"),
        params: "none",
        enabled: many_panes,
        run: |s, _p: &Params| {
            s.layout.equalize();
            Ok(())
        },
    }
}

/// `pane.zoom` — toggle showing only the focused pane.
pub fn zoom() -> CommandSpec {
    CommandSpec {
        id: "pane.zoom",
        label: "Zoom Pane",
        menu: &["Panes", "Zoom"],
        shortcut: Some("Ctrl+Shift+X"),
        params: "none",
        enabled: |s| many_panes(s) || s.layout.zoomed.is_some(),
        run: |s, _p: &Params| {
            s.layout.toggle_zoom();
            Ok(())
        },
    }
}

/// `pane.rotate` — flip the focused pane's parent split between side-by-side and stacked.
pub fn rotate() -> CommandSpec {
    CommandSpec {
        id: "pane.rotate",
        label: "Rotate Split",
        menu: &["Panes", "Rotate"],
        shortcut: Some("Ctrl+Shift+R"),
        params: "none",
        enabled: many_panes,
        run: |s, _p: &Params| {
            let focused = s.layout.focused();
            s.layout.rotate(focused);
            Ok(())
        },
    }
}

fn step(p: &Params) -> Result<f32, FmError> {
    let Some(map) = object(p)? else {
        return Ok(RESIZE_STEP);
    };
    match map.get("amount") {
        None | Some(Params::Null) => Ok(RESIZE_STEP),
        Some(v) => v
            .as_f64()
            .filter(|a| a.is_finite() && *a > 0.0 && *a <= f64::from(MAX_STEP))
            .map(|a| a as f32)
            .ok_or_else(|| FmError::InvalidParams(format!("amount must be in (0, {MAX_STEP}]"))),
    }
}

fn resize(s: &mut Session, p: &Params, axis: Axis, sign: f32) -> Result<(), FmError> {
    let amount = step(p)?;
    let focused = s.layout.focused();
    s.layout.resize(focused, axis, sign * amount);
    Ok(())
}

/// Move the nearest divider in this direction (params: `{amount?}`).
pub fn resize_left() -> CommandSpec {
    CommandSpec {
        id: "pane.resize.left",
        label: "Divider Left",
        menu: &["Panes", "Resize", "Divider Left"],
        shortcut: Some("Ctrl+Shift+Left"),
        params: "{amount?: number}",
        enabled: many_panes,
        run: |s, p: &Params| resize(s, p, Axis::Horizontal, -1.0),
    }
}

/// Move the nearest divider in this direction (params: `{amount?}`).
pub fn resize_right() -> CommandSpec {
    CommandSpec {
        id: "pane.resize.right",
        label: "Divider Right",
        menu: &["Panes", "Resize", "Divider Right"],
        shortcut: Some("Ctrl+Shift+Right"),
        params: "{amount?: number}",
        enabled: many_panes,
        run: |s, p: &Params| resize(s, p, Axis::Horizontal, 1.0),
    }
}

/// Move the nearest divider in this direction (params: `{amount?}`).
pub fn resize_up() -> CommandSpec {
    CommandSpec {
        id: "pane.resize.up",
        label: "Divider Up",
        menu: &["Panes", "Resize", "Divider Up"],
        shortcut: Some("Ctrl+Shift+Up"),
        params: "{amount?: number}",
        enabled: many_panes,
        run: |s, p: &Params| resize(s, p, Axis::Vertical, -1.0),
    }
}

/// Move the nearest divider in this direction (params: `{amount?}`).
pub fn resize_down() -> CommandSpec {
    CommandSpec {
        id: "pane.resize.down",
        label: "Divider Down",
        menu: &["Panes", "Resize", "Divider Down"],
        shortcut: Some("Ctrl+Shift+Down"),
        params: "{amount?: number}",
        enabled: many_panes,
        run: |s, p: &Params| resize(s, p, Axis::Vertical, 1.0),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        move_pane(),
        equalize(),
        zoom(),
        rotate(),
        resize_left(),
        resize_right(),
        resize_up(),
        resize_down(),
    ]
}
