//! `git.*` commands (capability `git-status`): refresh and marker toggles.

use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};

/// `git.refresh` — rescan every cached repository now.
pub fn refresh() -> CommandSpec {
    CommandSpec {
        id: "git.refresh",
        label: "Refresh Git Status",
        menu: &["View", "Refresh Git Status"],
        shortcut: None,
        params: "none",
        enabled: |s| s.prefs.git_enabled,
        run: |s, _p: &Params| {
            s.git.invalidate_all();
            Ok(())
        },
    }
}

/// `git.markers.toggle {which?: "enabled"|"gutter"|"tint"}` — default `enabled`.
pub fn toggle() -> CommandSpec {
    CommandSpec {
        id: "git.markers.toggle",
        label: "Toggle Git Markers",
        menu: &["View", "Git Markers"],
        shortcut: None,
        params: "{which?: \"enabled\"|\"gutter\"|\"tint\"}",
        enabled: |_s| true,
        run: |s, p: &Params| {
            let which = match object(p)? {
                None => "enabled",
                Some(map) => match map.get("which") {
                    None | Some(Params::Null) => "enabled",
                    Some(v) => v.as_str().ok_or_else(|| {
                        FmError::InvalidParams("which must be a string".to_string())
                    })?,
                },
            };
            match which {
                "enabled" => s.prefs.git_enabled = !s.prefs.git_enabled,
                "gutter" => s.prefs.git_gutter = !s.prefs.git_gutter,
                "tint" => s.prefs.git_tint = !s.prefs.git_tint,
                _ => {
                    return Err(FmError::InvalidParams(
                        "which must be enabled, gutter or tint".to_string(),
                    ));
                }
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![refresh(), toggle()]
}
