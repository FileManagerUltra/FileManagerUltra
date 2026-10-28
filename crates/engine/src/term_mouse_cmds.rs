//! Structured mouse input; legacy reports must reach the pty as raw bytes, not UTF-8.

use fm_layout::PaneKind;
use fm_term::{MouseAction, MouseButton, MouseEvent};
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "term.mouse",
        label: "Terminal Mouse Input",
        menu: &["Terminal", "Mouse Input"],
        shortcut: None,
        params: "{pane?: number, row: number, col: number, action: \"press\"|\"release\"|\"move\"|\"up\"|\"down\", button?: \"left\"|\"middle\"|\"right\", shift?: bool, alt?: bool, ctrl?: bool}",
        enabled: |s| s.focused_kind() == Some(PaneKind::Terminal),
        run: |s, params| {
            let map = object(params)?
                .ok_or_else(|| FmError::InvalidParams("mouse event object required".into()))?;
            let bad = |field: &str| FmError::InvalidParams(format!("invalid mouse {field}"));
            let cell = |field| {
                map.get(field)
                    .and_then(Params::as_u64)
                    .and_then(|v| u16::try_from(v).ok())
                    .ok_or_else(|| bad(field))
            };
            let flag = |field| match map.get(field) {
                None => Ok(false),
                Some(v) => v.as_bool().ok_or_else(|| bad(field)),
            };
            let button = match map.get("button") {
                None => None,
                Some(v) => Some(match v.as_str() {
                    Some("left") => MouseButton::Left,
                    Some("middle") => MouseButton::Middle,
                    Some("right") => MouseButton::Right,
                    _ => return Err(bad("button")),
                }),
            };
            let action = match map.get("action").and_then(Params::as_str) {
                Some("press") => MouseAction::Press(button.ok_or_else(|| bad("button"))?),
                Some("release") => MouseAction::Release(button.ok_or_else(|| bad("button"))?),
                Some("move") => MouseAction::Move(button),
                Some("up") => MouseAction::ScrollUp,
                Some("down") => MouseAction::ScrollDown,
                _ => return Err(bad("action")),
            };
            let event = MouseEvent {
                row: cell("row")?,
                col: cell("col")?,
                action,
                shift: flag("shift")?,
                alt: flag("alt")?,
                ctrl: flag("ctrl")?,
            };
            let pane = match map.get("pane") {
                None => s.layout.focused(),
                Some(v) => v.as_u64().ok_or_else(|| bad("pane"))?,
            };
            if s.pane_kind(pane) != Some(PaneKind::Terminal) {
                return Err(FmError::Unsupported("pane is not a terminal"));
            }
            s.terminals
                .get_mut(&pane)
                .ok_or(FmError::Unsupported("terminal is not running"))?
                .mouse(event)
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use serde_json::json;

    #[test]
    fn malformed_mouse_events_return_errors() {
        let mut s = Session::default();
        let command = specs().remove(0);
        for bad in [
            Params::Null,
            json!([]),
            json!({}),
            json!({"row": 0, "col": 0, "action": "press"}),
            json!({"row": -1, "col": 0, "action": "move"}),
            json!({"row": 0, "col": 65536, "action": "move"}),
            json!({"row": 0.5, "col": 0, "action": "move"}),
            json!({"row": 0, "col": 0, "action": "invalid"}),
            json!({"row": 0, "col": 0, "action": "move", "button": "extra"}),
            json!({"row": 0, "col": 0, "action": "move", "shift": 1}),
            json!({"row": 0, "col": 0, "action": "move", "pane": "1"}),
            json!({"row": 0, "col": 0, "action": "move"}),
        ] {
            assert!((command.run)(&mut s, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn registered_mouse_command_rejects_non_terminal_and_missing_session() {
        let mut s = Session::new().expect("session");
        let params = json!({"row": 0, "col": 0, "action": "press", "button": "left"});
        assert!(s.dispatch("term.mouse", &params).is_err());
        s.layout.state_mut(s.layout.focused()).expect("pane").kind = PaneKind::Terminal;
        assert!(s.dispatch("term.mouse", &params).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn mouse_reports_reach_the_pty_without_utf8_corrupting_legacy_bytes() {
        use std::time::{Duration, Instant};
        for (setup, col, count, expected) in [
            (
                "\\033[?1000h\\033[?1006h",
                7,
                9,
                "1b 5b 3c 30 3b 38 3b 31 4d",
            ),
            ("\\033[?1000h", 100, 6, "1b 5b 4d 20 85 21"),
        ] {
            let mut s = Session::new().expect("session");
            let pane = s.layout.focused();
            s.layout.state_mut(pane).expect("pane").kind = PaneKind::Terminal;
            s.ensure_terminal(pane).expect("shell");
            let term = s.terminals.get_mut(&pane).expect("terminal");
            term.resize(24, 160);
            term.write(format!(
                "stty -echo -icanon; printf '{setup}'; dd bs=1 count={count} 2>/dev/null | od -An -tx1; printf '\\033[?1000l\\033[?1006l'; stty sane\r"
            ).as_bytes()).expect("prepare");
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                s.poll();
                if s.terminal_snapshot(pane).is_some_and(|s| s.mouse.enabled()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(s.terminal_snapshot(pane).expect("snapshot").mouse.enabled());
            assert!(
                s.dispatch(
                    "term.mouse",
                    &json!({"row": 24, "col": 0, "action": "move"})
                )
                .is_err()
            );
            s.dispatch(
                "term.mouse",
                &json!({"row": 0, "col": col, "action": "press", "button": "left"}),
            )
            .expect("mouse");
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut received = false;
            while Instant::now() < deadline {
                s.poll();
                let snap = s.terminal_snapshot(pane).expect("snapshot");
                if snap
                    .text_between((0, 0), (snap.rows - 1, snap.cols - 1))
                    .contains(expected)
                {
                    received = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(received, "pty must receive {expected}");
        }
    }
}
