//! Pane zoom (change `terminal-zoom`): every pane has its own zoom, shown as − 100% + in its
//! header. Terminal and Preview panes keep the zoom they already had (`term.zoom.*`,
//! `preview.scale.*`); the other kinds scale their text, rows and spacing with
//! `PaneState::view_scale`, which is stored with the pane.

use fm_layout::{MAX_TERM_SCALE, MIN_TERM_SCALE, PaneId, PaneKind};
use fm_vfs::FmError;
use serde_json::json;

use crate::{CommandSpec, Params, Session};

fn bad_params() -> FmError {
    FmError::InvalidParams("expected {pane?: number}".into())
}

/// The pane the command is for: `pane`, or the focused pane.
fn target(s: &Session, params: &Params) -> Result<PaneId, FmError> {
    let pane = match params {
        Params::Null => s.layout.focused(),
        Params::Object(map) => {
            if map.keys().any(|key| key != "pane") {
                return Err(bad_params());
            }
            match map.get("pane") {
                None | Some(Params::Null) => s.layout.focused(),
                Some(value) => value.as_u64().ok_or_else(bad_params)?,
            }
        }
        _ => return Err(bad_params()),
    };
    s.layout
        .state(pane)
        .map(|_| pane)
        .ok_or_else(|| FmError::NotFound("no such pane".to_string()))
}

/// The zoom shown in `pane`'s header: the terminal or preview zoom for those kinds, the pane
/// zoom for the rest. `None` for an unknown pane.
pub fn pane_scale(s: &Session, pane: PaneId) -> Option<f32> {
    let state = s.layout.state(pane)?;
    Some(match state.kind {
        PaneKind::Terminal => state.term_scale(),
        PaneKind::Preview => s.previews.slots.get(&pane).map_or(1.0, |slot| slot.scale),
        _ => state.view_scale(),
    })
}

fn change(s: &mut Session, params: &Params, mode: i8) -> Result<(), FmError> {
    let pane = target(s, params)?;
    let kind = s
        .layout
        .state(pane)
        .map(|st| st.kind)
        .ok_or_else(|| FmError::NotFound("no such pane".to_string()))?;
    let named = json!({ "pane": pane });
    match kind {
        PaneKind::Terminal => crate::term_zoom_cmds::change(s, &named, mode),
        PaneKind::Preview => crate::preview_scale_cmds::change(s, &named, mode),
        _ => {
            let state = s
                .layout
                .state_mut(pane)
                .ok_or_else(|| FmError::NotFound("no such pane".to_string()))?;
            let current = state.view_scale();
            state.view_scale = fm_layout::zoom_step(current, mode, MIN_TERM_SCALE, MAX_TERM_SCALE);
            Ok(())
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "pane.scale.in",
            label: "Zoom Pane In",
            menu: &["View", "Pane", "Zoom In"],
            shortcut: None,
            params: "{pane?: number (default focused)}",
            enabled: |_| true,
            run: |s, p| change(s, p, 1),
        },
        CommandSpec {
            id: "pane.scale.out",
            label: "Zoom Pane Out",
            menu: &["View", "Pane", "Zoom Out"],
            shortcut: None,
            params: "{pane?: number (default focused)}",
            enabled: |_| true,
            run: |s, p| change(s, p, -1),
        },
        CommandSpec {
            id: "pane.scale.reset",
            label: "Reset Pane Zoom",
            menu: &["View", "Pane", "Reset Zoom"],
            shortcut: None,
            params: "{pane?: number (default focused)}",
            enabled: |_| true,
            run: |s, p| change(s, p, 0),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files (first) beside a second pane of `kind` (focused).
    fn files_and(kind: PaneKind) -> (Session, PaneId, PaneId) {
        let mut s = Session::new().expect("session");
        let first = s.layout.focused();
        s.dispatch("pane.split.horizontal", &Params::Null)
            .expect("split");
        let second = s.layout.focused();
        s.layout.state_mut(second).expect("pane").kind = kind;
        (s, first, second)
    }

    fn scale(s: &Session, pane: PaneId) -> f32 {
        pane_scale(s, pane).expect("pane")
    }

    #[test]
    fn each_pane_zooms_on_its_own_within_bounds() {
        for kind in [
            PaneKind::Files,
            PaneKind::Trash,
            PaneKind::Network,
            PaneKind::Sftp,
            PaneKind::Media,
        ] {
            let (mut s, first, second) = files_and(kind);
            let global = s.prefs.zoom;
            s.dispatch("pane.scale.in", &Params::Null).expect("focused");
            assert!((scale(&s, second) - 1.2).abs() < 1e-4, "{kind:?}");
            assert_eq!(scale(&s, first), 1.0, "the other pane is unchanged");
            for _ in 0..50 {
                s.dispatch("pane.scale.in", &json!({"pane": first}))
                    .expect("zoom");
            }
            assert_eq!(scale(&s, first), MAX_TERM_SCALE);
            for _ in 0..50 {
                s.dispatch("pane.scale.out", &json!({"pane": first}))
                    .expect("zoom");
            }
            assert_eq!(scale(&s, first), MIN_TERM_SCALE);
            s.dispatch("pane.scale.reset", &json!({"pane": first}))
                .expect("reset");
            assert_eq!(scale(&s, first), 1.0);
            assert_eq!(s.prefs.zoom, global, "interface zoom untouched");
        }
    }

    #[test]
    fn zooming_out_to_the_floor_and_back_in_returns_to_exactly_100() {
        for kind in [PaneKind::Files, PaneKind::Terminal, PaneKind::Preview] {
            let (mut s, _, pane) = files_and(kind);
            let target = json!({ "pane": pane });
            for _ in 0..10 {
                s.dispatch("pane.scale.out", &target).expect("out");
            }
            assert_eq!(scale(&s, pane), MIN_TERM_SCALE, "{kind:?}");
            let mut seen = Vec::new();
            for _ in 0..4 {
                s.dispatch("pane.scale.in", &target).expect("in");
                seen.push((scale(&s, pane) * 100.0).round());
            }
            assert_eq!(seen, [58.0, 69.0, 83.0, 100.0], "{kind:?}");
            assert!((scale(&s, pane) - 1.0).abs() < 1e-4, "{kind:?}");
        }
    }

    #[test]
    fn terminal_and_preview_panes_use_their_own_zoom() {
        let (mut s, _, term) = files_and(PaneKind::Terminal);
        s.dispatch("pane.scale.in", &json!({"pane": term}))
            .expect("zoom");
        let state = s.layout.state(term).expect("pane");
        assert!((state.term_scale() - 1.2).abs() < 1e-4);
        assert_eq!(state.view_scale(), 1.0, "terminal zoom, not the pane zoom");

        let (mut s, _, preview) = files_and(PaneKind::Preview);
        s.dispatch("pane.scale.in", &json!({"pane": preview}))
            .expect("zoom");
        let slot = s.previews.slots.get(&preview).expect("slot");
        assert!((slot.scale - 1.2).abs() < 1e-4);
        assert!((scale(&s, preview) - 1.2).abs() < 1e-4);
        assert_eq!(s.layout.state(preview).expect("pane").view_scale(), 1.0);
        s.dispatch("pane.scale.reset", &json!({"pane": preview}))
            .expect("reset");
        assert_eq!(scale(&s, preview), 1.0);
    }

    #[test]
    fn pane_zoom_rejects_bad_params() {
        let (mut s, first, _) = files_and(PaneKind::Files);
        for id in ["pane.scale.in", "pane.scale.out", "pane.scale.reset"] {
            for bad in [
                json!("x"),
                json!([]),
                json!({"pane": -1}),
                json!({"pane": "1"}),
                json!({"scale": 2}),
                json!({"pane": 999_999}),
            ] {
                assert!(s.dispatch(id, &bad).is_err(), "{id} {bad}");
            }
        }
        assert_eq!(scale(&s, first), 1.0, "nothing changed");
        assert_eq!(pane_scale(&s, 999_999), None);
    }
}
