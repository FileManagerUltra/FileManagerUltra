//! Preferences commands: the dialog and the settings it edits.

use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

/// `app.preferences` — raise the Preferences dialog flag (the UI renders and clears it).
pub fn preferences() -> CommandSpec {
    CommandSpec {
        id: "app.preferences",
        label: "Preferences…",
        menu: &["File", "Preferences"],
        shortcut: Some("Ctrl+,"),
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.preferences_open = true;
            Ok(())
        },
    }
}

/// `prefs.links.retarget` — whether copying a tree re-targets absolute symlinks that point
/// inside it. Without `value` the setting is toggled.
pub fn retarget_links() -> CommandSpec {
    CommandSpec {
        id: "prefs.links.retarget",
        label: "Re-target Links When Copying",
        menu: &["File", "Preferences", "Re-target links"],
        shortcut: None,
        params: "value?: bool",
        enabled: |_session| true,
        run: |session, params| {
            let value = match object(params)?.and_then(|m| m.get("value")) {
                None => !session.prefs.retarget_links,
                Some(v) => v.as_bool().ok_or_else(|| {
                    FmError::InvalidParams("value must be true or false".to_string())
                })?,
            };
            session.prefs.retarget_links = value;
            Ok(())
        },
    }
}

/// `prefs.terminal.shell` — the shell new terminal panes start: `default` (PowerShell on
/// Windows, the login shell elsewhere), `powershell` or `cmd`. Running terminals keep
/// theirs until restarted.
pub fn terminal_shell() -> CommandSpec {
    CommandSpec {
        id: "prefs.terminal.shell",
        label: "Terminal Shell",
        menu: &["File", "Preferences", "Terminal shell"],
        shortcut: None,
        params: "{value: \"default\"|\"powershell\"|\"cmd\"}",
        enabled: |_session| true,
        run: |session, params| {
            let value = object(params)?
                .and_then(|m| m.get("value"))
                .and_then(Params::as_str)
                .ok_or_else(|| {
                    FmError::InvalidParams(
                        "value: \"default\", \"powershell\" or \"cmd\"".to_string(),
                    )
                })?;
            session.prefs.terminal_shell = match value {
                "default" => crate::prefs::TerminalShell::Default,
                "powershell" => crate::prefs::TerminalShell::PowerShell,
                "cmd" => crate::prefs::TerminalShell::Cmd,
                _ => {
                    return Err(FmError::InvalidParams(
                        "value must be default, powershell or cmd".to_string(),
                    ));
                }
            };
            Ok(())
        },
    }
}

/// `prefs.preview.recursive` — whether a folder in the preview pane counts and thumbnails
/// every file below it. Without `value` the setting is toggled.
pub fn recursive_preview() -> CommandSpec {
    CommandSpec {
        id: "prefs.preview.recursive",
        label: "Recursive Folder Preview",
        menu: &["File", "Preferences", "Recursive folder preview"],
        shortcut: None,
        params: "value?: bool",
        enabled: |_session| true,
        run: |session, params| {
            let value = match object(params)?.and_then(|m| m.get("value")) {
                None => !session.prefs.recursive_preview,
                Some(v) => v.as_bool().ok_or_else(|| {
                    FmError::InvalidParams("value must be true or false".to_string())
                })?,
            };
            if value != session.prefs.recursive_preview {
                session.prefs.recursive_preview = value;
                // The render key includes the scope, so folders re-render; the grids ask
                // for their window again once the new item list lands.
                let panes: Vec<_> = session.previews.slots.keys().copied().collect();
                for pane in panes {
                    if let Some(slot) = session.previews.slots.get_mut(&pane) {
                        slot.thumb_window = None;
                    }
                    session.thumbs.cancel(pane);
                }
            }
            Ok(())
        },
    }
}

fn secs(
    map: &serde_json::Map<String, Params>,
    key: &str,
    range: &std::ops::RangeInclusive<u64>,
) -> Result<Option<u64>, FmError> {
    match map.get(key) {
        None | Some(Params::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| range.contains(n))
            .map(Some)
            .ok_or_else(|| {
                FmError::InvalidParams(format!(
                    "{key} must be {}-{} seconds",
                    range.start(),
                    range.end()
                ))
            }),
    }
}

/// `prefs.smb.encryption` — require encrypted SMB sessions for new connections.
pub fn smb_encryption() -> CommandSpec {
    CommandSpec {
        id: "prefs.smb.encryption",
        label: "Require Encrypted SMB",
        menu: &["File", "Preferences", "Require encrypted SMB"],
        shortcut: None,
        params: "value?: bool",
        enabled: |_session| true,
        run: |session, params| {
            session.prefs.smb_require_encryption =
                match object(params)?.and_then(|m| m.get("value")) {
                    None => !session.prefs.smb_require_encryption,
                    Some(v) => v.as_bool().ok_or_else(|| {
                        FmError::InvalidParams("value must be true or false".to_string())
                    })?,
                };
            Ok(())
        },
    }
}

/// `prefs.remote.timing` — how fast a dead remote link is noticed.
pub fn sftp_timing() -> CommandSpec {
    CommandSpec {
        id: "prefs.remote.timing",
        label: "Remote Timing",
        menu: &["File", "Preferences", "Remote timing"],
        shortcut: None,
        params: "poll_secs?: 1-60, timeout_secs?: 2-120, keepalive_secs?: 1-60",
        enabled: |_session| true,
        run: |session, params| {
            let empty = serde_json::Map::new();
            let map = object(params)?.unwrap_or(&empty);
            let poll = secs(map, "poll_secs", &crate::prefs::REMOTE_POLL_RANGE)?;
            let timeout = secs(map, "timeout_secs", &crate::prefs::REMOTE_TIMEOUT_RANGE)?;
            let keepalive = secs(map, "keepalive_secs", &crate::prefs::REMOTE_KEEPALIVE_RANGE)?;
            if let Some(v) = poll {
                session.prefs.remote_poll_secs = v;
            }
            if let Some(v) = timeout {
                session.prefs.remote_timeout_secs = v;
            }
            if let Some(v) = keepalive {
                session.prefs.remote_keepalive_secs = v;
            }
            session.prefs.apply_sftp();
            Ok(())
        },
    }
}

/// Shared body of the three zoom commands: `step` is `None` to reset.
fn zoom_by(session: &mut crate::session::Session, step: Option<f32>) {
    let next = match step {
        Some(step) => session.prefs.zoom_factor() + step,
        None => 1.0,
    };
    // Round to one decimal so repeated steps never drift (1.0000001).
    let next = (next * 10.0).round() / 10.0;
    session.prefs.zoom = next.clamp(
        *crate::prefs::ZOOM_RANGE.start(),
        *crate::prefs::ZOOM_RANGE.end(),
    );
}

/// `view.zoom.in` — enlarge the whole UI one step.
pub fn zoom_in() -> CommandSpec {
    CommandSpec {
        id: "view.zoom.in",
        label: "Zoom In",
        menu: &["View", "Zoom In"],
        shortcut: Some("Ctrl+Shift++"),
        params: "none",
        enabled: |s| s.prefs.zoom_factor() < *crate::prefs::ZOOM_RANGE.end(),
        run: |s, _p| {
            zoom_by(s, Some(crate::prefs::ZOOM_STEP));
            Ok(())
        },
    }
}

/// `view.zoom.out` — shrink the whole UI one step.
pub fn zoom_out() -> CommandSpec {
    CommandSpec {
        id: "view.zoom.out",
        label: "Zoom Out",
        menu: &["View", "Zoom Out"],
        shortcut: Some("Ctrl+Shift+-"),
        params: "none",
        enabled: |s| s.prefs.zoom_factor() > *crate::prefs::ZOOM_RANGE.start(),
        run: |s, _p| {
            zoom_by(s, Some(-crate::prefs::ZOOM_STEP));
            Ok(())
        },
    }
}

/// `view.zoom.reset` — back to 100%.
pub fn zoom_reset() -> CommandSpec {
    CommandSpec {
        id: "view.zoom.reset",
        label: "Reset Zoom",
        menu: &["View", "Reset Zoom"],
        shortcut: Some("Ctrl+Shift+0"),
        params: "none",
        enabled: |s| (s.prefs.zoom_factor() - 1.0).abs() > f32::EPSILON,
        run: |s, _p| {
            zoom_by(s, None);
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::session::Session;

    #[test]
    fn zoom_steps_clamp_and_reset() {
        let mut s = Session::new().expect("session");
        (zoom_in().run)(&mut s, &json!(null)).expect("in");
        assert!((s.prefs.zoom - 1.1).abs() < 1e-6);
        for _ in 0..40 {
            (zoom_in().run)(&mut s, &json!(null)).expect("in");
        }
        assert!((s.prefs.zoom - 3.0).abs() < 1e-6);
        for _ in 0..60 {
            (zoom_out().run)(&mut s, &json!(null)).expect("out");
        }
        assert!((s.prefs.zoom - 0.5).abs() < 1e-6);
        (zoom_reset().run)(&mut s, &json!(null)).expect("reset");
        assert!((s.prefs.zoom - 1.0).abs() < 1e-6);
        s.prefs.zoom = f32::NAN;
        assert!((s.prefs.zoom_factor() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sftp_timing_sets_valid_values_and_rejects_bad_ones() {
        let mut s = Session::default();
        (sftp_timing().run)(&mut s, &json!({"poll_secs": 2, "timeout_secs": 6})).expect("set");
        assert_eq!(
            (s.prefs.remote_poll_secs, s.prefs.remote_timeout_secs),
            (2, 6)
        );
        for bad in [
            json!({"poll_secs": 0}),
            json!({"timeout_secs": 1}),
            json!({"keepalive_secs": 9999}),
            json!({"poll_secs": "5"}),
            json!([1]),
        ] {
            assert!((sftp_timing().run)(&mut s, &bad).is_err(), "{bad}");
        }
        assert_eq!(
            (s.prefs.remote_poll_secs, s.prefs.remote_timeout_secs),
            (2, 6)
        );
        fm_fs_sftp::set_timeouts(10, 5);
    }

    #[test]
    fn recursive_preview_toggles_and_rejects_bad_values() {
        let mut s = Session::default();
        assert!(!s.prefs.recursive_preview);
        (recursive_preview().run)(&mut s, &Params::Null).expect("toggle");
        assert!(s.prefs.recursive_preview);
        (recursive_preview().run)(&mut s, &json!({"value": false})).expect("set");
        assert!(!s.prefs.recursive_preview);
        assert!((recursive_preview().run)(&mut s, &json!({"value": 3})).is_err());
        assert!(!s.prefs.recursive_preview, "unchanged on error");
    }

    #[test]
    fn terminal_shell_sets_and_rejects_bad_values() {
        use crate::prefs::TerminalShell;
        let mut s = Session::default();
        assert_eq!(s.prefs.terminal_shell, TerminalShell::Default);
        (terminal_shell().run)(&mut s, &json!({"value": "powershell"})).expect("set");
        assert_eq!(s.prefs.terminal_shell, TerminalShell::PowerShell);
        for bad in [
            Params::Null,
            json!({}),
            json!({"value": "bash"}),
            json!({"value": 1}),
            json!(["cmd"]),
        ] {
            assert!((terminal_shell().run)(&mut s, &bad).is_err(), "{bad}");
        }
        assert_eq!(
            s.prefs.terminal_shell,
            TerminalShell::PowerShell,
            "unchanged"
        );
        (terminal_shell().run)(&mut s, &json!({"value": "default"})).expect("set");
        assert_eq!(s.prefs.terminal_shell, TerminalShell::Default);
    }

    #[test]
    fn smb_encryption_toggles_and_rejects_bad_values() {
        let mut s = Session::default();
        assert!(!s.prefs.smb_require_encryption);
        (smb_encryption().run)(&mut s, &Params::Null).expect("toggle");
        assert!(s.prefs.smb_require_encryption);
        assert!((smb_encryption().run)(&mut s, &json!({"value": "x"})).is_err());
        assert!(s.prefs.smb_require_encryption, "unchanged on error");
    }

    #[test]
    fn preferences_raises_the_flag() {
        let mut s = Session::default();
        (preferences().run)(&mut s, &Params::Null).expect("run");
        assert!(s.preferences_open);
    }

    #[test]
    fn retarget_defaults_on_toggles_and_sets() {
        let mut s = Session::default();
        assert!(s.prefs.retarget_links);
        (retarget_links().run)(&mut s, &Params::Null).expect("toggle");
        assert!(!s.prefs.retarget_links);
        (retarget_links().run)(&mut s, &json!({"value": true})).expect("set");
        assert!(s.prefs.retarget_links);
    }

    #[test]
    fn retarget_rejects_malformed_params() {
        let mut s = Session::default();
        for bad in [json!({"value": "yes"}), json!({"value": 1}), json!([true])] {
            assert!((retarget_links().run)(&mut s, &bad).is_err(), "{bad}");
        }
        assert!(s.prefs.retarget_links, "unchanged on error");
    }
}
