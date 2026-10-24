//! Workspace commands (capability `workspaces`): tabs of pane layouts that can be saved,
//! reordered and restored. The state and rules live in [`crate::workspace`].

use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;
use crate::workspace::Flag;

fn tab_param(s: &Session, params: &Params, key: &str) -> Result<usize, FmError> {
    match object(params)?.and_then(|m| m.get(key)) {
        None | Some(Params::Null) => Ok(s.workspaces.active),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| FmError::InvalidParams(format!("{key}: tab index (0-based)"))),
    }
}

fn text_param<'a>(params: &'a Params, key: &str) -> Result<Option<&'a str>, FmError> {
    object(params)?
        .and_then(|m| m.get(key))
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_str()
                .ok_or_else(|| FmError::InvalidParams(format!("{key}: text")))
        })
        .transpose()
}

fn required_text<'a>(params: &'a Params, key: &str) -> Result<&'a str, FmError> {
    text_param(params, key)?.ok_or_else(|| FmError::InvalidParams(format!("{key}: text required")))
}

fn flag_command(flag: Flag, params: &Params, s: &mut Session) -> Result<(), FmError> {
    let named = text_param(params, "name")?;
    let ix = tab_param(s, params, "tab")?;
    let uid = match named {
        Some(name) => s
            .workspaces
            .saved
            .iter()
            .find(|w| w.name.eq_ignore_ascii_case(name))
            .map(|w| w.uid)
            .ok_or_else(|| FmError::NotFound(format!("saved workspace {name:?}")))?,
        None => s
            .workspaces
            .tabs
            .get(ix)
            .map(|t| t.uid)
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {}", ix + 1)))?,
    };
    let current = s.workspaces.saved_for(uid).is_some_and(|w| match flag {
        Flag::AutoSave => w.auto_save,
        Flag::OpenAtStartup => w.open_at_startup,
        Flag::Default => w.default,
    });
    let on = match object(params)?.and_then(|m| m.get("on")) {
        None | Some(Params::Null) => !current,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| FmError::InvalidParams("on: bool".to_string()))?,
    };
    if named.is_some() {
        s.set_saved_flag(uid, flag, on)
    } else {
        s.workspace_set_flag(ix, flag, on)
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "workspace.new",
            label: "New Workspace",
            menu: &["Workspace", "New"],
            shortcut: Some("Ctrl+T"),
            params: "{name?: text}",
            enabled: |_| true,
            run: |s, p| s.workspace_new(text_param(p, "name")?),
        },
        CommandSpec {
            id: "workspace.switch",
            label: "Switch to Workspace",
            menu: &["Workspace", "Switch to"],
            shortcut: None,
            params: "{n: number (1-based tab position)}",
            enabled: |s| s.workspaces.tabs.len() > 1,
            run: |s, p| {
                let n = object(p)?
                    .and_then(|m| m.get("n"))
                    .and_then(Params::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n >= 1)
                    .ok_or_else(|| FmError::InvalidParams("n: tab position from 1".to_string()))?;
                s.workspace_switch(n - 1)
            },
        },
        CommandSpec {
            id: "workspace.close",
            label: "Close Workspace",
            menu: &["Workspace", "Close"],
            shortcut: Some("Ctrl+W"),
            params: "{tab?: number (0-based; default active)}",
            enabled: |_| true,
            run: |s, p| {
                let ix = tab_param(s, p, "tab")?;
                s.workspace_close(ix)
            },
        },
        CommandSpec {
            id: "workspace.save",
            label: "Save Workspace",
            menu: &["Workspace", "Save"],
            shortcut: None,
            params: "{tab?: number, name?: text}",
            enabled: |_| true,
            run: |s, p| {
                let ix = tab_param(s, p, "tab")?;
                s.workspace_save(ix, text_param(p, "name")?)
            },
        },
        CommandSpec {
            id: "workspace.rename",
            label: "Rename Workspace",
            menu: &["Workspace", "Rename"],
            shortcut: None,
            params: "{name: text, tab?: number}",
            enabled: |_| true,
            run: |s, p| {
                let ix = tab_param(s, p, "tab")?;
                s.workspace_rename(ix, required_text(p, "name")?)
            },
        },
        CommandSpec {
            id: "workspace.autosave",
            label: "Auto-save Workspace",
            menu: &["Workspace", "Auto-save"],
            shortcut: None,
            params: "{on?: bool, tab?: number, name?: text (a saved workspace)}",
            enabled: |_| true,
            run: |s, p| flag_command(Flag::AutoSave, p, s),
        },
        CommandSpec {
            id: "workspace.startup",
            label: "Open Workspace at Startup",
            menu: &["Workspace", "Open at startup"],
            shortcut: None,
            params: "{on?: bool, tab?: number, name?: text (a saved workspace)}",
            enabled: |_| true,
            run: |s, p| flag_command(Flag::OpenAtStartup, p, s),
        },
        CommandSpec {
            id: "workspace.default",
            label: "Make Default Workspace",
            menu: &["Workspace", "Default workspace"],
            shortcut: None,
            params: "{on?: bool, tab?: number, name?: text (a saved workspace)}",
            enabled: |_| true,
            run: |s, p| flag_command(Flag::Default, p, s),
        },
        CommandSpec {
            id: "workspace.move",
            label: "Move Workspace Tab",
            menu: &["Workspace", "Move tab"],
            shortcut: None,
            params: "{from: number, to: number} (0-based)",
            enabled: |s| s.workspaces.tabs.len() > 1,
            run: |s, p| {
                let get = |key: &str| {
                    object(p)?
                        .and_then(|m| m.get(key))
                        .and_then(Params::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| FmError::InvalidParams(format!("{key}: tab index")))
                };
                s.workspace_move(get("from")?, get("to")?)
            },
        },
        CommandSpec {
            id: "workspace.open",
            label: "Open Saved Workspace",
            menu: &["Workspace", "Open saved"],
            shortcut: None,
            params: "{name: text}",
            enabled: |s| !s.workspaces.saved.is_empty(),
            run: |s, p| s.workspace_open_saved(required_text(p, "name")?),
        },
        CommandSpec {
            id: "workspace.delete",
            label: "Delete Saved Workspace",
            menu: &["Workspace", "Delete saved"],
            shortcut: None,
            params: "{name: text}",
            enabled: |s| !s.workspaces.saved.is_empty(),
            run: |s, p| s.workspace_delete_saved(required_text(p, "name")?),
        },
        CommandSpec {
            id: "workspace.prompt",
            label: "Answer Unsaved Workspaces Prompt",
            menu: &["Workspace", "Unsaved changes"],
            shortcut: None,
            params: "{choice: save|discard|cancel}",
            enabled: |s| s.unsaved_prompt.is_some(),
            run: |s, p| s.workspace_resolve_prompt(required_text(p, "choice")?),
        },
    ]
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::workspace::MAX_SAVED;
    use serde_json::json;

    fn session() -> Session {
        Session::new().expect("session")
    }

    fn run(s: &mut Session, id: &str, p: Params) -> Result<(), FmError> {
        s.dispatch(id, &p)
    }

    #[test]
    fn tabs_keep_their_own_layout_and_disjoint_pane_ids() {
        let mut s = session();
        run(&mut s, "pane.split.horizontal", Params::Null).expect("split");
        assert_eq!(s.layout.panes().len(), 2);
        let first: Vec<_> = s.layout.panes();
        run(&mut s, "workspace.new", Params::Null).expect("new");
        assert_eq!(s.workspaces.tabs.len(), 2);
        assert_eq!(s.workspaces.active, 1);
        assert_eq!(s.layout.panes().len(), 1, "a new tab starts with one pane");
        let second = s.layout.panes();
        assert!(second.iter().all(|p| !first.contains(p)), "ids never clash");
        run(&mut s, "workspace.switch", json!({"n": 1})).expect("switch");
        assert_eq!(s.layout.panes(), first, "the first tab is intact");
        assert!(run(&mut s, "workspace.switch", json!({"n": 9})).is_err());
        assert!(run(&mut s, "workspace.switch", json!({"n": 0})).is_err());
    }

    #[test]
    fn closing_the_last_tab_leaves_a_fresh_default() {
        let mut s = session();
        run(&mut s, "pane.split.horizontal", Params::Null).expect("split");
        run(&mut s, "workspace.close", Params::Null).expect("close");
        assert_eq!(s.workspaces.tabs.len(), 1);
        assert_eq!(s.layout.panes().len(), 1);
        run(&mut s, "workspace.new", Params::Null).expect("new");
        run(&mut s, "workspace.close", json!({"tab": 0})).expect("close first");
        assert_eq!(s.workspaces.tabs.len(), 1);
        assert_eq!(s.workspaces.active, 0);
        assert!(s.layout.state(s.layout.focused()).is_some());
    }

    #[test]
    fn closing_a_background_tab_keeps_the_active_one() {
        let mut s = session();
        run(&mut s, "workspace.new", Params::Null).expect("new");
        run(&mut s, "workspace.new", Params::Null).expect("new");
        let focused = s.layout.focused();
        run(&mut s, "workspace.close", json!({"tab": 0})).expect("close");
        assert_eq!(s.workspaces.tabs.len(), 2);
        assert_eq!(s.workspaces.active, 1);
        assert_eq!(s.layout.focused(), focused);
    }

    #[test]
    fn save_cap_names_and_flags() {
        let mut s = session();
        for i in 0..MAX_SAVED {
            if i > 0 {
                run(&mut s, "workspace.new", Params::Null).expect("new");
            }
            run(&mut s, "workspace.save", Params::Null).expect("save");
        }
        assert_eq!(s.workspaces.saved.len(), MAX_SAVED);
        run(&mut s, "workspace.new", Params::Null).expect("new");
        assert!(run(&mut s, "workspace.save", Params::Null).is_err(), "11th");
        assert!(run(&mut s, "workspace.autosave", Params::Null).is_err());
        assert_eq!(s.workspaces.saved.len(), MAX_SAVED);

        let other = s
            .workspaces
            .tabs
            .first()
            .map(|t| t.name.clone())
            .expect("tab");
        assert!(run(&mut s, "workspace.rename", json!({"name": other})).is_err());
        assert!(run(&mut s, "workspace.rename", json!({"name": "  "})).is_err());
        assert!(run(&mut s, "workspace.rename", json!({"name": "a\nb"})).is_err());
        run(&mut s, "workspace.rename", json!({"name": "Media"})).expect("rename");

        run(&mut s, "workspace.delete", json!({"name": &other})).expect("delete");
        run(&mut s, "workspace.default", json!({"tab": 1})).expect("default");
        run(&mut s, "workspace.default", json!({"tab": 2})).expect("default");
        assert_eq!(s.workspaces.saved.iter().filter(|w| w.default).count(), 1);
        run(&mut s, "workspace.startup", json!({"on": true})).expect("startup");
        assert!(s.workspaces.saved.iter().any(|w| w.open_at_startup));
    }

    #[test]
    fn dirty_tabs_prompt_before_close_and_quit() {
        let mut s = session();
        run(&mut s, "workspace.new", Params::Null).expect("new");
        run(&mut s, "workspace.save", json!({"name": "Work"})).expect("save");
        assert!(s.workspace_unsaved().is_empty());
        run(&mut s, "pane.split.horizontal", Params::Null).expect("split");
        assert_eq!(s.workspace_unsaved().len(), 1);

        run(&mut s, "app.quit", Params::Null).expect("quit");
        assert!(s.running, "quit waits for the answer");
        assert!(s.unsaved_prompt.is_some());
        run(&mut s, "workspace.prompt", json!({"choice": "cancel"})).expect("cancel");
        assert!(s.running && s.unsaved_prompt.is_none());

        run(&mut s, "workspace.close", Params::Null).expect("close");
        assert!(s.unsaved_prompt.is_some());
        run(&mut s, "workspace.prompt", json!({"choice": "save"})).expect("save");
        assert_eq!(s.workspaces.tabs.len(), 1, "closed after saving");
        let saved = s
            .workspaces
            .saved
            .iter()
            .find(|w| w.name == "Work")
            .expect("saved");
        assert!(
            saved.snapshot.to_string().contains("Split"),
            "the split was saved"
        );

        run(&mut s, "workspace.open", json!({"name": "work"})).expect("reopen");
        assert_eq!(s.layout.panes().len(), 2);
        run(&mut s, "pane.close", Params::Null).expect("close pane");
        run(&mut s, "app.quit", Params::Null).expect("quit");
        run(&mut s, "workspace.prompt", json!({"choice": "discard"})).expect("discard");
        assert!(!s.running);
        assert!(
            run(&mut s, "workspace.prompt", json!({"choice": "x"})).is_err()
                || s.unsaved_prompt.is_none()
        );
    }

    #[test]
    fn autosave_follows_the_layout_and_never_prompts() {
        let mut s = session();
        run(&mut s, "workspace.autosave", Params::Null).expect("autosave");
        assert_eq!(
            s.workspace_tick().map(|t| t.contains("Workspace 1")),
            Some(true)
        );
        run(&mut s, "pane.split.horizontal", Params::Null).expect("split");
        assert!(s.workspace_unsaved().is_empty());
        let text = s.workspace_tick().expect("changed");
        assert!(text.contains("Split"));
        assert!(s.workspace_tick().is_none(), "nothing new to write");
    }

    #[test]
    fn the_saved_set_survives_a_restart() {
        let mut s = session();
        run(&mut s, "workspace.save", json!({"name": "Home"})).expect("save");
        run(&mut s, "workspace.startup", Params::Null).expect("startup");
        run(&mut s, "pane.split.vertical", Params::Null).expect("split");
        run(&mut s, "workspace.save", Params::Null).expect("save");
        run(&mut s, "workspace.new", json!({"name": "Scratch"})).expect("new");
        run(&mut s, "workspace.save", Params::Null).expect("save");
        let text = s.workspace_tick().expect("file");

        let mut t = session();
        assert!(t.workspace_startup(&text).is_none());
        assert_eq!(t.workspaces.saved.len(), 2);
        assert_eq!(
            t.workspaces.tabs.len(),
            1,
            "only the startup workspace opens"
        );
        assert_eq!(
            t.workspaces.tabs.first().map(|w| w.name.as_str()),
            Some("Home")
        );
        assert_eq!(t.layout.panes().len(), 2);
        run(&mut t, "workspace.open", json!({"name": "Scratch"})).expect("open");
        assert_eq!(t.layout.panes().len(), 1);
        let all: Vec<_> = t.workspaces.tabs.iter().map(|w| w.name.clone()).collect();
        assert_eq!(all, ["Home", "Scratch"]);
    }

    #[test]
    fn a_default_workspace_opens_when_none_is_marked_for_startup() {
        let mut s = session();
        run(&mut s, "pane.split.vertical", Params::Null).expect("split");
        run(&mut s, "workspace.default", Params::Null).expect("default");
        let text = s.workspace_tick().expect("file");
        let mut t = session();
        t.workspace_startup(&text);
        assert_eq!(t.layout.panes().len(), 2);
        run(&mut t, "workspace.close", Params::Null).expect("close last");
        assert_eq!(
            t.layout.panes().len(),
            2,
            "replaced by the default workspace"
        );
    }

    #[test]
    fn hostile_files_leave_the_builtin_workspace() {
        for text in [
            "",
            "not json",
            "{}",
            r#"{"workspaces":[1,"x",null,{"name":"a"}]}"#,
            r#"{"workspaces":[{"name":"a","snapshot":{"layout":{}},"open_at_startup":true}]}"#,
            r#"{"workspaces":[{"name":"","snapshot":{}}]}"#,
        ] {
            let mut s = session();
            s.workspace_startup(text);
            assert_eq!(s.workspaces.tabs.len(), 1, "{text}");
            assert_eq!(s.layout.panes().len(), 1);
            assert!(s.layout.state(s.layout.focused()).is_some());
        }
    }

    #[test]
    fn preview_settings_travel_with_the_workspace() {
        let mut s = session();
        run(&mut s, "pane.split.horizontal", Params::Null).expect("split");
        let pane = s.layout.focused();
        if let Some(st) = s.layout.state_mut(pane) {
            st.kind = fm_layout::PaneKind::Preview;
        }
        let mut slot = crate::preview_cmds::PreviewSlot::new();
        slot.wrap = true;
        slot.thumbs = true;
        slot.thumb_size = 2;
        s.previews.slots.insert(pane, slot);
        run(&mut s, "workspace.save", json!({"name": "P"})).expect("save");
        run(&mut s, "workspace.startup", Params::Null).expect("startup");
        let text = s.workspace_tick().expect("file");

        let mut t = session();
        t.workspace_startup(&text);
        let restored = t
            .layout
            .states
            .iter()
            .find(|(_, st)| st.kind == fm_layout::PaneKind::Preview)
            .map(|(id, _)| *id)
            .expect("preview pane");
        let slot = t.previews.slots.get(&restored).expect("slot");
        assert!(slot.wrap && slot.thumbs && slot.thumb_size == 2);
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session();
        for (id, p) in [
            ("workspace.close", json!({"tab": 99})),
            ("workspace.close", json!({"tab": "x"})),
            ("workspace.rename", json!({})),
            ("workspace.move", json!({"from": 0, "to": 5})),
            ("workspace.open", json!({"name": "nope"})),
            ("workspace.delete", json!({"name": "nope"})),
            ("workspace.autosave", json!({"on": 3})),
        ] {
            assert!(run(&mut s, id, p).is_err(), "{id}");
        }
        assert_eq!(s.workspaces.tabs.len(), 1);
    }
}
