//! Pane-local preview magnification, independent of application UI zoom.

use fm_vfs::FmError;

use crate::{
    CommandSpec, Params, Session,
    preview_cmds::{PreviewSlot, has_preview},
};

pub const MIN_SCALE: f32 = 0.5;
pub const MAX_SCALE: f32 = 4.0;

fn change(s: &mut Session, params: &Params, mode: i8) -> Result<(), FmError> {
    if !params.is_null() {
        let object = params
            .as_object()
            .ok_or_else(|| FmError::InvalidParams("expected {pane?: number}".into()))?;
        if object.keys().any(|key| key != "pane") {
            return Err(FmError::InvalidParams("expected {pane?: number}".into()));
        }
        if object.get("pane").is_some_and(|p| p.as_u64().is_none()) {
            return Err(FmError::InvalidParams(
                "pane must be a non-negative integer".into(),
            ));
        }
    }
    let pane = s.preview_pane(params)?;
    let slot = s
        .previews
        .slots
        .entry(pane)
        .or_insert_with(PreviewSlot::new);
    slot.scale = match mode {
        1 => (slot.scale * 1.2).min(MAX_SCALE),
        -1 => (slot.scale / 1.2).max(MIN_SCALE),
        _ => 1.0,
    };
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "preview.scale.in",
            label: "Zoom Preview In",
            menu: &["View", "Preview", "Zoom In"],
            shortcut: Some("Ctrl+="),
            params: "{pane?: number}",
            enabled: has_preview,
            run: |s, p| change(s, p, 1),
        },
        CommandSpec {
            id: "preview.scale.out",
            label: "Zoom Preview Out",
            menu: &["View", "Preview", "Zoom Out"],
            shortcut: Some("Ctrl+-"),
            params: "{pane?: number}",
            enabled: has_preview,
            run: |s, p| change(s, p, -1),
        },
        CommandSpec {
            id: "preview.scale.reset",
            label: "Reset Preview Zoom",
            menu: &["View", "Preview", "Reset Zoom"],
            shortcut: Some("Ctrl+0"),
            params: "{pane?: number}",
            enabled: has_preview,
            run: |s, p| change(s, p, 0),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_layout::PaneKind;
    use serde_json::json;

    #[test]
    fn zoom_is_pane_local_bounded_and_rejects_bad_params() {
        let mut s = Session::new().expect("session");
        let first = s.layout.focused();
        s.layout.state_mut(first).expect("pane").kind = PaneKind::Preview;
        s.dispatch("pane.split.horizontal", &Params::Null)
            .expect("split");
        let second = s.layout.focused();
        s.layout.state_mut(second).expect("pane").kind = PaneKind::Preview;
        for id in [
            "preview.scale.in",
            "preview.scale.out",
            "preview.scale.reset",
        ] {
            for bad in [
                json!("x"),
                json!([]),
                json!({"pane": -1}),
                json!({"scale": 100}),
                json!({"pane": 999999}),
            ] {
                assert!(s.dispatch(id, &bad).is_err());
            }
        }
        let global = s.prefs.zoom;
        for _ in 0..50 {
            s.dispatch("preview.scale.in", &json!({"pane": first}))
                .expect("zoom");
        }
        assert_eq!(s.preview_slot(first).expect("slot").scale, MAX_SCALE);
        assert_eq!(s.preview_slot(second).map_or(1.0, |s| s.scale), 1.0);
        assert_eq!(s.prefs.zoom, global);
        for _ in 0..50 {
            s.dispatch("preview.scale.out", &json!({"pane": first}))
                .expect("zoom");
        }
        assert_eq!(s.preview_slot(first).expect("slot").scale, MIN_SCALE);
        s.dispatch("preview.scale.reset", &json!({"pane": first}))
            .expect("reset");
        assert_eq!(s.preview_slot(first).expect("slot").scale, 1.0);
    }
}
