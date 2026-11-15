//! Per-terminal text zoom (change `terminal-zoom`): each terminal pane scales its own text on
//! top of the terminal font size, independent of other terminals and of the interface zoom.
//! The value lives in the pane state, so it travels with the workspace.

use fm_layout::{MAX_TERM_SCALE, MIN_TERM_SCALE, PaneId, PaneKind};
use fm_vfs::FmError;

use crate::{CommandSpec, Params, Session};

const STEP: f32 = 1.2;

fn bad_params() -> FmError {
    FmError::InvalidParams("expected {pane?: number}".into())
}

/// The terminal pane the command is for: `pane`, or the focused pane.
fn terminal_pane(s: &Session, params: &Params) -> Result<PaneId, FmError> {
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
    let is_terminal = s
        .layout
        .state(pane)
        .is_some_and(|st| st.kind == PaneKind::Terminal);
    is_terminal
        .then_some(pane)
        .ok_or_else(|| FmError::NotFound("no terminal pane".to_string()))
}

fn focused_terminal(s: &Session) -> bool {
    terminal_pane(s, &Params::Null).is_ok()
}

pub(crate) fn change(s: &mut Session, params: &Params, mode: i8) -> Result<(), FmError> {
    let pane = terminal_pane(s, params)?;
    let state = s
        .layout
        .state_mut(pane)
        .ok_or_else(|| FmError::NotFound("no terminal pane".to_string()))?;
    let current = state.term_scale();
    state.term_scale = match mode {
        1 => (current * STEP).min(MAX_TERM_SCALE),
        -1 => (current / STEP).max(MIN_TERM_SCALE),
        _ => 1.0,
    };
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "term.zoom.in",
            label: "Zoom Terminal In",
            menu: &["View", "Terminal", "Zoom In"],
            shortcut: Some("Ctrl+Shift+="),
            params: "{pane?: number (a terminal pane; default focused)}",
            enabled: focused_terminal,
            run: |s, p| change(s, p, 1),
        },
        CommandSpec {
            id: "term.zoom.out",
            label: "Zoom Terminal Out",
            menu: &["View", "Terminal", "Zoom Out"],
            shortcut: Some("Ctrl+Shift+-"),
            params: "{pane?: number (a terminal pane; default focused)}",
            enabled: focused_terminal,
            run: |s, p| change(s, p, -1),
        },
        CommandSpec {
            id: "term.zoom.reset",
            label: "Reset Terminal Zoom",
            menu: &["View", "Terminal", "Reset Zoom"],
            shortcut: Some("Ctrl+Shift+0"),
            params: "{pane?: number (a terminal pane; default focused)}",
            enabled: focused_terminal,
            run: |s, p| change(s, p, 0),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn two_terminals() -> (Session, PaneId, PaneId) {
        let mut s = Session::new().expect("session");
        let first = s.layout.focused();
        s.layout.state_mut(first).expect("pane").kind = PaneKind::Terminal;
        s.dispatch("pane.split.horizontal", &Params::Null)
            .expect("split");
        let second = s.layout.focused();
        s.layout.state_mut(second).expect("pane").kind = PaneKind::Terminal;
        (s, first, second)
    }

    fn scale(s: &Session, pane: PaneId) -> f32 {
        s.layout.state(pane).expect("pane").term_scale()
    }

    #[test]
    fn each_terminal_zooms_on_its_own_within_bounds() {
        let (mut s, first, second) = two_terminals();
        let global = (s.prefs.zoom, s.prefs.terminal_font_size);
        s.dispatch("term.zoom.in", &Params::Null).expect("focused");
        assert!((scale(&s, second) - 1.2).abs() < 1e-4);
        assert_eq!(scale(&s, first), 1.0, "the other terminal is unchanged");
        for _ in 0..50 {
            s.dispatch("term.zoom.in", &json!({"pane": first}))
                .expect("zoom");
        }
        assert_eq!(scale(&s, first), MAX_TERM_SCALE);
        for _ in 0..50 {
            s.dispatch("term.zoom.out", &json!({"pane": first}))
                .expect("zoom");
        }
        assert_eq!(scale(&s, first), MIN_TERM_SCALE);
        s.dispatch("term.zoom.reset", &json!({"pane": first}))
            .expect("reset");
        assert_eq!(scale(&s, first), 1.0);
        assert_eq!(
            (s.prefs.zoom, s.prefs.terminal_font_size),
            global,
            "interface zoom and font size untouched"
        );
    }

    #[test]
    fn terminal_zoom_needs_a_terminal_and_rejects_bad_params() {
        let (mut s, first, _) = two_terminals();
        for id in ["term.zoom.in", "term.zoom.out", "term.zoom.reset"] {
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
        s.layout.state_mut(first).expect("pane").kind = PaneKind::Files;
        assert!(s.dispatch("term.zoom.in", &json!({"pane": first})).is_err());
        s.layout.focus_pane(first).expect("focus");
        assert!(
            !s.command_enabled("term.zoom.in"),
            "a Files pane has no terminal zoom"
        );
    }

    #[test]
    fn a_corrupt_stored_zoom_reads_as_bounded() {
        let (mut s, first, _) = two_terminals();
        for (stored, read) in [
            (f32::NAN, 1.0),
            (100.0, MAX_TERM_SCALE),
            (0.0, MIN_TERM_SCALE),
        ] {
            s.layout.state_mut(first).expect("pane").term_scale = stored;
            assert_eq!(scale(&s, first), read);
        }
        s.layout.state_mut(first).expect("pane").term_scale = f32::INFINITY;
        s.dispatch("term.zoom.out", &json!({"pane": first}))
            .expect("zoom");
        assert!(scale(&s, first) < 1.0, "steps from the sanitised value");
    }
}
