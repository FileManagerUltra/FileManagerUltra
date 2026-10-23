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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::session::Session;

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
