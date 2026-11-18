//! Folder sync commands (capability `file-sync`). The UI and menus dispatch these by id;
//! nothing here touches a filesystem on the calling thread (see `sync_hub`).

use fm_layout::PaneId;
use fm_sync::{Action, Profile};
use fm_vfs::{FmError, VfsUri};
use serde_json::Value;

use crate::file_cmds::{focused_dir, object};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;
use crate::sync_hub::{Arming, RowFilter, SortKey};

const MENU: &[&str] = &["File", "Synchronize Folders"];

pub(crate) fn bad(msg: &str) -> FmError {
    FmError::InvalidParams(msg.to_string())
}

pub(crate) fn str_param<'a>(params: &'a Params, key: &str) -> Result<Option<&'a str>, FmError> {
    match object(params)?.and_then(|m| m.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= 4096 => Ok(Some(s)),
        Some(_) => Err(bad(&format!("{key} must be a short string"))),
    }
}

fn bool_param(params: &Params, key: &str) -> Result<bool, FmError> {
    match object(params)?.and_then(|m| m.get(key)) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(bad(&format!("{key} must be true or false"))),
    }
}

/// The profile an id-taking command acts on: `id` or the one in view.
pub(crate) fn target(s: &Session, params: &Params) -> Result<String, FmError> {
    let id = match str_param(params, "id")? {
        Some(id) => id.to_string(),
        None => s
            .sync
            .current
            .clone()
            .ok_or_else(|| bad("no sync is open"))?,
    };
    if s.sync.states.contains_key(&id) {
        Ok(id)
    } else {
        Err(FmError::NotFound("sync profile".to_string()))
    }
}

fn parse_action(name: &str) -> Option<Action> {
    Some(match name {
        "none" => Action::None,
        "skip" => Action::Skip,
        "copy_to_left" => Action::CopyToLeft,
        "copy_to_right" => Action::CopyToRight,
        "delete_left" => Action::DeleteLeft,
        "delete_right" => Action::DeleteRight,
        _ => return None,
    })
}

/// The folder the user has focused in the tree or selected in the listing, if any.
fn highlighted_dir(s: &mut Session, pane: PaneId) -> Option<VfsUri> {
    let state = s.layout.state(pane).filter(|p| p.kind.is_files())?;
    if state.tree.active {
        return state.tree.focused.clone();
    }
    let entry = s.cursor_entry(pane)?;
    // The cursor alone is not a choice; only a folder the user selected counts.
    let selected = s.layout.state(pane)?.listing.is_selected(&entry.name);
    (selected && entry.is_dir()).then_some(entry.uri)
}

fn draft(s: &mut Session) -> Profile {
    let mut p = Profile {
        name: "New sync".to_string(),
        id: s.sync.fresh_id(),
        ..Profile::default()
    };
    // Seed from the focused folder and the first other Files pane.
    let focused = s.layout.focused();
    let left = highlighted_dir(s, focused).or_else(|| focused_dir(s));
    let right = s
        .layout
        .panes()
        .into_iter()
        .filter(|id| *id != focused)
        .filter_map(|id| s.layout.state(id))
        .find(|st| st.kind.is_files())
        .map(|st| st.location.clone());
    if let Some(l) = left {
        p.left = l.to_string();
    }
    if let Some(r) = right {
        p.right = r.to_string();
    }
    p
}

fn open() -> CommandSpec {
    CommandSpec {
        id: "sync.open",
        label: "Synchronize Folders…",
        menu: &["File", "Synchronize Folders…"],
        shortcut: None,
        params: r#"{ id?: string } — a saved profile; omitted starts a new one from the highlighted (or current) folder and the other pane"#,
        enabled: |_| true,
        run: |s, params| {
            let saved = match str_param(params, "id")? {
                Some(id) => Some(
                    s.sync
                        .profiles
                        .iter()
                        .find(|p| p.id == id)
                        .cloned()
                        .ok_or_else(|| FmError::NotFound("sync profile".to_string()))?,
                ),
                None => None,
            };
            match saved {
                Some(p) => s.sync.open_profile(p, true),
                None => {
                    let p = draft(s);
                    s.sync.open_profile(p, false);
                }
            }
            Ok(())
        },
    }
}

fn close() -> CommandSpec {
    CommandSpec {
        id: "sync.close",
        label: "Close Folder Sync",
        menu: &["File", "Synchronize Folders", "Close"],
        shortcut: None,
        params: "none",
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            object(params)?;
            if let Some(id) = s.sync.current.clone()
                && let Some(st) = s.sync.states.get_mut(&id)
            {
                st.arming = Arming::Off;
            }
            s.sync.current = None;
            s.sync.minimized = false;
            Ok(())
        },
    }
}

fn profile_set() -> CommandSpec {
    CommandSpec {
        id: "sync.profile.set",
        label: "Edit Sync Profile",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, patch: object } — fields of the profile to change (not the id)"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let patch = object(params)?
                .and_then(|m| m.get("patch"))
                .and_then(Value::as_object)
                .ok_or_else(|| bad("patch must be an object"))?;
            let state = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            if state.busy() {
                return Err(bad("stop the running sync before editing it"));
            }
            let mut value = serde_json::to_value(&state.profile)
                .map_err(|e| FmError::backend("sync", e.to_string()))?;
            let Some(obj) = value.as_object_mut() else {
                return Err(bad("profile is not an object"));
            };
            for (k, v) in patch {
                if k == "id" {
                    continue;
                }
                if !obj.contains_key(k) {
                    return Err(bad(&format!("unknown profile field {k}")));
                }
                obj.insert(k.clone(), v.clone());
            }
            let mut next: Profile = serde_json::from_value(value)
                .map_err(|e| bad(&format!("invalid profile value: {e}")))?;
            // A typed OS path and its uri are the same folder; keep one form so edits don't reset.
            for folder in [&mut next.left, &mut next.right] {
                if let Ok(uri) = fm_sync::profile::parse_endpoint(folder) {
                    *folder = uri.to_string();
                }
            }
            // Folders and mode changes invalidate the comparison.
            let reset = next.left != state.profile.left
                || next.right != state.profile.right
                || next.mode != state.profile.mode
                || next.source != state.profile.source
                || next.filters != state.profile.filters;
            state.profile = next;
            state.saved = false;
            if reset {
                state.arming = Arming::Off;
            }
            if reset {
                state.plan = None;
                state.rows.clear();
                state.phase = crate::sync_hub::Phase::Idle;
            }
            Ok(())
        },
    }
}

fn profile_save() -> CommandSpec {
    CommandSpec {
        id: "sync.profile.save",
        label: "Save Sync Profile",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string }"#,
        enabled: |s| {
            s.sync
                .current_state()
                .is_some_and(|st| st.profile.validate().is_ok())
        },
        run: |s, params| {
            let id = target(s, params)?;
            let profile = s
                .sync
                .states
                .get(&id)
                .map(|st| st.profile.clone())
                .ok_or_else(|| bad("no sync"))?;
            s.sync.upsert(profile)?;
            s.sync.persist()?;
            if let Some(st) = s.sync.states.get_mut(&id) {
                st.saved = true;
            }
            s.info = Some(("Sync profile saved".to_string(), std::time::Instant::now()));
            Ok(())
        },
    }
}

fn minimize() -> CommandSpec {
    CommandSpec {
        id: "sync.minimize",
        label: "Run Sync in Background",
        menu: &["File", "Synchronize Folders", "Minimize"],
        shortcut: None,
        params: "none",
        enabled: |s| s.sync.current.is_some() && !s.sync.minimized,
        run: |s, params| {
            object(params)?;
            if s.sync.current.is_none() {
                return Err(bad("no sync is open"));
            }
            s.sync.minimized = true;
            Ok(())
        },
    }
}

fn restore() -> CommandSpec {
    CommandSpec {
        id: "sync.restore",
        label: "Show Sync",
        menu: &["File", "Synchronize Folders", "Show"],
        shortcut: None,
        params: r#"{ id?: string } — the sync to show; default is the minimised one"#,
        enabled: |s| !s.sync.states.is_empty(),
        run: |s, params| {
            let id = match str_param(params, "id")? {
                Some(id) => id.to_string(),
                None => match s.sync.current.clone() {
                    Some(id) => id,
                    None => match s.sync.states.keys().min() {
                        Some(id) => id.clone(),
                        None => return Err(bad("no sync is open")),
                    },
                },
            };
            if !s.sync.states.contains_key(&id) {
                return Err(FmError::NotFound("sync profile".to_string()));
            }
            s.sync.current = Some(id);
            s.sync.minimized = false;
            Ok(())
        },
    }
}

fn sort() -> CommandSpec {
    CommandSpec {
        id: "sync.sort",
        label: "Sort Sync Rows",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, key: "path"|"name"|"size"|"modified"|"action", descending?: bool }"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let key = str_param(params, "key")?
                .and_then(SortKey::parse)
                .ok_or_else(|| bad("key is not recognised"))?;
            let descending = match object(params)?.and_then(|m| m.get("descending")) {
                None | Some(Value::Null) => false,
                Some(v) => v
                    .as_bool()
                    .ok_or_else(|| bad("descending must be a boolean"))?,
            };
            let state = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            state.sort = key;
            state.descending = descending;
            state.refresh_rows();
            Ok(())
        },
    }
}

fn profile_new() -> CommandSpec {
    CommandSpec {
        id: "sync.profile.new",
        label: "New Sync Profile",
        menu: MENU,
        shortcut: None,
        params: "none",
        enabled: |_| true,
        run: |s, params| {
            object(params)?;
            let p = draft(s);
            s.sync.open_profile(p, false);
            Ok(())
        },
    }
}

fn profile_duplicate() -> CommandSpec {
    CommandSpec {
        id: "sync.profile.duplicate",
        label: "Duplicate Sync Profile",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string }"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let mut p = s
                .sync
                .states
                .get(&id)
                .map(|st| st.profile.clone())
                .ok_or_else(|| bad("no sync"))?;
            p.id = s.sync.fresh_id();
            p.name = format!("{} copy", p.name.chars().take(100).collect::<String>());
            p.watch.enabled = false;
            s.sync.open_profile(p, false);
            Ok(())
        },
    }
}

fn profile_rename() -> CommandSpec {
    CommandSpec {
        id: "sync.profile.rename",
        label: "Rename Sync Profile",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, name: string }"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let name = str_param(params, "name")?.ok_or_else(|| bad("name is required"))?;
            let name = name.trim();
            if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
                return Err(bad(
                    "the name must be 1-120 characters without control characters",
                ));
            }
            if let Some(state) = s.sync.states.get_mut(&id) {
                state.profile.name = name.to_string();
            }
            if let Some(saved) = s.sync.profiles.iter_mut().find(|p| p.id == id) {
                saved.name = name.to_string();
                s.sync.persist()?;
            }
            Ok(())
        },
    }
}

fn profile_delete() -> CommandSpec {
    CommandSpec {
        id: "sync.profile.delete",
        label: "Delete Sync Profile",
        menu: MENU,
        shortcut: None,
        params: r#"{ id: string } — removes the profile and its remembered state"#,
        enabled: |s| !s.sync.profiles.is_empty(),
        run: |s, params| {
            let id = str_param(params, "id")?
                .ok_or_else(|| bad("id is required"))?
                .to_string();
            if s.sync.states.get(&id).is_some_and(|st| st.busy()) {
                return Err(bad("stop the running sync first"));
            }
            let before = s.sync.profiles.len();
            s.sync.profiles.retain(|p| p.id != id);
            if s.sync.profiles.len() == before && !s.sync.states.contains_key(&id) {
                return Err(FmError::NotFound("sync profile".to_string()));
            }
            s.sync.states.remove(&id);
            if s.sync.current.as_deref() == Some(id.as_str()) {
                s.sync.current = None;
            }
            if let Some(store) = s.sync.store() {
                // Best effort: a leftover snapshot only costs a full re-compare.
                drop(store.delete(&id));
            }
            s.sync.persist()
        },
    }
}

fn compare() -> CommandSpec {
    CommandSpec {
        id: "sync.compare",
        label: "Compare Folders",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string } — scans both folders and builds the plan; changes nothing"#,
        enabled: |s| {
            s.sync
                .current_state()
                .is_some_and(|st| !st.busy() && st.profile.validate().is_ok())
        },
        run: |s, params| {
            let id = target(s, params)?;
            let result = s.sync_compare(&id, false);
            // Show why in the sync window too, not only in the Activity panel.
            if let Err(e) = &result
                && let Some(st) = s.sync.states.get_mut(&id)
                && !st.busy()
            {
                st.phase = crate::sync_hub::Phase::Failed(e.to_string());
            }
            result
        },
    }
}

fn run() -> CommandSpec {
    CommandSpec {
        id: "sync.run",
        label: "Synchronize",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, confirmed?: bool } — runs the compared plan; deletes that need confirmation are refused unless confirmed"#,
        enabled: |s| {
            s.sync.current_state().is_some_and(|st| {
                !st.busy() && st.plan.as_ref().is_some_and(|p| !p.is_empty_work())
            })
        },
        run: |s, params| {
            let id = target(s, params)?;
            let confirmed = bool_param(params, "confirmed")?;
            s.sync_run(&id, confirmed)
        },
    }
}

fn dismiss_confirm() -> CommandSpec {
    CommandSpec {
        id: "sync.confirm.dismiss",
        label: "Cancel Sync Confirmation",
        menu: &["File", "Synchronize Folders", "Cancel Confirmation"],
        shortcut: None,
        params: r#"{ id?: string }"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| st.confirm),
        run: |s, params| {
            let id = target(s, params)?;
            if let Some(st) = s.sync.states.get_mut(&id) {
                st.confirm = false;
            }
            Ok(())
        },
    }
}

fn cancel() -> CommandSpec {
    CommandSpec {
        id: "sync.cancel",
        label: "Cancel Sync",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string }"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| st.busy()),
        run: |s, params| {
            let id = target(s, params)?;
            s.sync_cancel(&id);
            Ok(())
        },
    }
}

fn set_action() -> CommandSpec {
    CommandSpec {
        id: "sync.set_action",
        label: "Override Sync Action",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, rel: string, action: "none"|"skip"|"copy_to_left"|"copy_to_right"|"delete_left"|"delete_right", recursive?: boolean }"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| st.plan.is_some()),
        run: |s, params| {
            let id = target(s, params)?;
            let rel = str_param(params, "rel")?.ok_or_else(|| bad("rel is required"))?;
            let action = str_param(params, "action")?
                .and_then(parse_action)
                .ok_or_else(|| bad("action is not recognised"))?;
            let state = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            if state.busy() {
                return Err(bad("the sync is running"));
            }
            let plan = state.plan.as_mut().ok_or_else(|| bad("compare first"))?;
            if params.get("recursive").and_then(Value::as_bool) == Some(true) {
                plan.set_action_tree(rel, action)?;
            } else {
                plan.set_action(rel, action)?;
            }
            state.refresh_rows();
            Ok(())
        },
    }
}

fn filter() -> CommandSpec {
    CommandSpec {
        id: "sync.filter",
        label: "Filter Sync Rows",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, show: "all"|"changes"|"conflicts"|"equal" }"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let show = str_param(params, "show")?
                .and_then(RowFilter::parse)
                .ok_or_else(|| bad("show is not recognised"))?;
            let state = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            state.filter = show;
            state.refresh_rows();
            Ok(())
        },
    }
}

fn filter_action() -> CommandSpec {
    CommandSpec {
        id: "sync.filter_action",
        label: "Filter Sync Rows by Action",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, action: "all"|"none"|"skip"|"copy_to_left"|"copy_to_right"|"delete_left"|"delete_right" }"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let name = str_param(params, "action")?.ok_or_else(|| bad("action is required"))?;
            let action = if name == "all" {
                None
            } else {
                Some(parse_action(name).ok_or_else(|| bad("action is not recognised"))?)
            };
            let state = s.sync.states.get_mut(&id).ok_or_else(|| bad("no sync"))?;
            state.action_filter = action;
            state.refresh_rows();
            Ok(())
        },
    }
}

fn ignore() -> CommandSpec {
    CommandSpec {
        id: "sync.ignore",
        label: "Add to Sync Ignore List",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string, rel: string, folder?: boolean } — adds the item to the profile's ignore list and compares again"#,
        enabled: |s| s.sync.current.is_some(),
        run: |s, params| {
            let id = target(s, params)?;
            let rel = str_param(params, "rel")?
                .map(|r| r.trim_matches('/'))
                .filter(|r| !r.is_empty())
                .ok_or_else(|| bad("rel is required"))?;
            if rel.contains(['*', '?']) {
                return Err(bad(
                    "this name contains * or ?; add a pattern by hand instead",
                ));
            }
            let pattern = if bool_param(params, "folder")? {
                format!("{rel}/")
            } else {
                rel.to_string()
            };
            fm_sync::glob::Glob::compile(&pattern)?;
            let state = s.sync.states.get(&id).ok_or_else(|| bad("no sync"))?;
            let mut list = state.profile.filters.exclude.clone();
            if list.contains(&pattern) {
                return Ok(());
            }
            list.push(pattern);
            s.dispatch(
                "sync.profile.set",
                &serde_json::json!({ "id": id, "patch": { "filters": { "exclude": list } } }),
            )?;
            s.dispatch("sync.compare", &serde_json::json!({ "id": id }))
        },
    }
}

fn watch_start() -> CommandSpec {
    CommandSpec {
        id: "sync.watch.start",
        label: "Start Continuous Sync",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string } — compares first; `sync.watch.confirm` then starts keeping the folders in sync while the app is open"#,
        enabled: |s| {
            s.sync.current_state().is_some_and(|st| {
                !st.watching() && st.arming == Arming::Off && st.profile.validate().is_ok()
            })
        },
        run: |s, params| {
            let id = target(s, params)?;
            if let Some(st) = s.sync.states.get_mut(&id) {
                st.profile.validate()?;
                st.arming = Arming::Comparing;
            }
            let started = s.sync_compare(&id, false);
            if started.is_err()
                && let Some(st) = s.sync.states.get_mut(&id)
            {
                st.arming = Arming::Off;
            }
            started
        },
    }
}

fn watch_confirm() -> CommandSpec {
    CommandSpec {
        id: "sync.watch.confirm",
        label: "Start Syncing",
        menu: &["File", "Synchronize Folders", "Start Syncing"],
        shortcut: None,
        params: r#"{ id?: string } — after the first compare: apply it and keep the folders in sync"#,
        enabled: |s| {
            s.sync
                .current_state()
                .is_some_and(|st| st.arming == Arming::Confirm && !st.busy())
        },
        run: |s, params| {
            let id = target(s, params)?;
            let waiting = s
                .sync
                .states
                .get(&id)
                .is_some_and(|st| st.arming == Arming::Confirm && !st.busy());
            if !waiting {
                return Err(bad("compare first, then confirm"));
            }
            s.sync_watch(&id, true)?;
            // Confirmed: no longer waiting, or the status line and Activity card keep
            // asking to "Confirm to start syncing" with a stale Cancel.
            if let Some(st) = s.sync.states.get_mut(&id) {
                st.arming = Arming::Off;
            }
            s.sync_apply_confirmed(&id);
            Ok(())
        },
    }
}

fn watch_cancel() -> CommandSpec {
    CommandSpec {
        id: "sync.watch.cancel",
        label: "Cancel Start Syncing",
        menu: &["File", "Synchronize Folders", "Cancel Start Syncing"],
        shortcut: None,
        params: r#"{ id?: string }"#,
        enabled: |s| {
            s.sync
                .current_state()
                .is_some_and(|st| st.arming == Arming::Confirm)
        },
        run: |s, params| {
            let id = target(s, params)?;
            if let Some(st) = s.sync.states.get_mut(&id) {
                st.arming = Arming::Off;
            }
            Ok(())
        },
    }
}

fn watch_stop() -> CommandSpec {
    CommandSpec {
        id: "sync.watch.stop",
        label: "Stop Continuous Sync",
        menu: MENU,
        shortcut: None,
        params: r#"{ id?: string }"#,
        enabled: |s| s.sync.current_state().is_some_and(|st| st.watching()),
        run: |s, params| {
            let id = target(s, params)?;
            s.sync_watch(&id, false)
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        open(),
        close(),
        minimize(),
        restore(),
        sort(),
        profile_new(),
        profile_set(),
        profile_save(),
        profile_duplicate(),
        profile_rename(),
        profile_delete(),
        compare(),
        run(),
        dismiss_confirm(),
        cancel(),
        set_action(),
        filter(),
        filter_action(),
        ignore(),
        watch_start(),
        watch_confirm(),
        watch_cancel(),
        watch_stop(),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn pump_until_idle(s: &mut Session) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            s.poll();
            if s.sync.current_state().is_some_and(|st| !st.busy()) || Instant::now() > end {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn session_with(
        left: &std::path::Path,
        right: &std::path::Path,
        cfg: &std::path::Path,
    ) -> Session {
        let mut s = Session::new().expect("session");
        s.sync.set_config_dir(cfg.to_path_buf());
        s.dispatch("sync.open", &json!({})).expect("open");
        let patch = json!({
            "name": "t",
            "left": VfsUriExt::uri(left),
            "right": VfsUriExt::uri(right),
            "mode": "Mirror",
            "delete": "Permanent",
        });
        s.dispatch("sync.profile.set", &json!({ "patch": patch }))
            .expect("set");
        s
    }

    trait VfsUriExt {
        fn uri(&self) -> String;
    }
    impl VfsUriExt for std::path::Path {
        fn uri(&self) -> String {
            fm_vfs::VfsUri::parse(&self.to_string_lossy())
                .expect("uri")
                .to_string()
        }
    }

    #[test]
    fn compare_run_and_persist() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(l.join("a.txt"), "hello").expect("w");
        std::fs::write(r.join("stale.txt"), "x").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        let st = s.sync.current_state().expect("state");
        assert_eq!(st.rows.len(), 2);
        // The permanent delete needs confirmation: refused, flagged.
        s.dispatch("sync.run", &json!({})).expect("run");
        assert!(s.sync.current_state().is_some_and(|st| st.confirm));
        assert!(r.join("stale.txt").exists());
        s.dispatch("sync.confirm.dismiss", &json!({}))
            .expect("dismiss");
        assert!(s.sync.current_state().is_some_and(|st| !st.confirm));
        s.dispatch("sync.run", &json!({"confirmed": true}))
            .expect("run");
        pump_until_idle(&mut s);
        assert_eq!(
            std::fs::read_to_string(r.join("a.txt")).expect("r"),
            "hello"
        );
        assert!(!r.join("stale.txt").exists());

        s.dispatch("sync.profile.save", &json!({})).expect("save");
        let mut again = Session::new().expect("session");
        assert!(again.sync.set_config_dir(c.clone()).is_none());
        assert_eq!(again.sync.profiles.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(c.join("sync-profiles.json"))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }

    #[test]
    fn rows_sort_by_size_and_the_window_minimizes_and_restores() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(l.join("a.txt"), "x").expect("w");
        std::fs::write(l.join("b.txt"), "xxxxxx").expect("w");
        std::fs::write(l.join("c.txt"), "xxx").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        let order = |s: &Session| {
            let st = s.sync.current_state().expect("state");
            let plan = st.plan.as_ref().expect("plan");
            st.rows
                .iter()
                .map(|i| plan.items[*i as usize].rel.clone())
                .collect::<Vec<_>>()
        };
        s.dispatch("sync.sort", &json!({"key": "size"}))
            .expect("sort");
        assert_eq!(order(&s), ["a.txt", "c.txt", "b.txt"]);
        s.dispatch("sync.sort", &json!({"key": "size", "descending": true}))
            .expect("sort");
        assert_eq!(order(&s), ["b.txt", "c.txt", "a.txt"]);
        assert!(s.dispatch("sync.sort", &json!({"key": "colour"})).is_err());

        assert!(s.dispatch("sync.restore", &json!({"id": "nope"})).is_err());
        s.dispatch("sync.minimize", &json!({})).expect("minimize");
        assert!(s.sync.minimized && s.sync.current.is_some());
        assert!(!s.command_enabled("sync.minimize"));
        s.dispatch("sync.restore", &json!({})).expect("restore");
        assert!(!s.sync.minimized);
    }

    #[test]
    fn a_typed_os_path_is_stored_as_the_same_folder_as_its_uri() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        let typed = tmp.path().to_string_lossy().to_string();
        s.dispatch("sync.profile.set", &json!({"patch": {"left": typed}}))
            .expect("set");
        let uri = VfsUri::parse(&typed).expect("uri");
        let st = s.sync.current_state().expect("state");
        assert_eq!(st.profile.left, uri.to_string());
    }

    #[test]
    fn a_new_sync_starts_from_the_highlighted_folder() {
        let tmp = tempfile::tempdir().expect("tmp");
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).expect("mkdir");
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        let uri = VfsUri::parse(&sub.to_string_lossy()).expect("uri");
        let state = s.layout.state_mut(pane).expect("pane");
        state.tree.active = true;
        state.tree.focused = Some(uri.clone());
        s.dispatch("sync.open", &json!({})).expect("open");
        let st = s.sync.current_state().expect("state");
        assert_eq!(st.profile.left, uri.to_string());
    }

    #[test]
    fn a_missing_target_folder_is_created_for_the_initial_sync() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("new").join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(l.join("a.txt"), "hello").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        assert_eq!(s.sync.current_state().expect("state").rows.len(), 1);
        s.dispatch("sync.run", &json!({})).expect("run");
        pump_until_idle(&mut s);
        assert_eq!(
            std::fs::read_to_string(r.join("a.txt")).expect("r"),
            "hello"
        );
    }

    #[test]
    fn compare_with_an_unconnected_server_asks_to_sign_in_first() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, c) = (tmp.path().join("l"), tmp.path().join("cfg"));
        for d in [&l, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        let mut s = session_with(&l, &l, &c);
        s.dispatch(
            "sync.profile.set",
            &json!({"patch": {"mode": "Update", "right": "sftp://demo@127.0.0.1:2222/backups/Photos"}}),
        )
        .expect("set");
        s.dispatch("sync.compare", &json!({}))
            .expect("compare waits for the sign-in instead of failing");
        let prompt = s
            .remote
            .sftp_prompt
            .as_ref()
            .expect("sign-in dialog opened");
        assert_eq!(prompt.host, "127.0.0.1");
        let st = s.sync.current_state().expect("state");
        assert!(st.awaiting_login && !st.busy() && st.plan.is_none());
        // Still waiting while the dialog is open.
        s.poll();
        assert!(s.sync.current_state().is_some_and(|st| st.awaiting_login));
        // Cancelling the sign-in ends the wait with a message in the sync window.
        s.remote.sftp_prompt = None;
        s.poll();
        let st = s.sync.current_state().expect("state");
        assert!(!st.awaiting_login);
        assert!(
            matches!(&st.phase, crate::sync_hub::Phase::Failed(m) if m.contains("127.0.0.1")),
            "{:?}",
            st.phase
        );
    }

    #[test]
    fn a_watched_edit_is_synced_without_reading_on_the_event_thread() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        let mut s = session_with(&l, &r, &c);
        s.dispatch(
            "sync.profile.set",
            &json!({"patch": {"mode": "Update", "watch": {"enabled": true}}}),
        )
        .expect("set");
        std::fs::write(l.join("first.txt"), "one").expect("w");
        s.dispatch("sync.watch.start", &json!({})).expect("watch");
        pump_until_idle(&mut s);
        // Nothing is applied and nothing watched until the user confirms the first compare.
        let st = s.sync.current_state().expect("state");
        assert!(!st.watching() && st.arming == Arming::Confirm && st.plan.is_some());
        assert!(!r.join("first.txt").exists());
        s.dispatch("sync.watch.confirm", &json!({}))
            .expect("confirm");
        assert!(
            s.sync
                .current_state()
                .is_some_and(|st| st.watching() && st.arming == Arming::Off),
            "confirming ends the confirm step"
        );
        pump_until_idle(&mut s);
        assert_eq!(
            std::fs::read_to_string(r.join("first.txt")).expect("first"),
            "one"
        );
        std::fs::write(l.join("new.txt"), "fresh").expect("w");
        let end = Instant::now() + Duration::from_secs(20);
        while !r.join("new.txt").exists() && Instant::now() < end {
            s.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(r.join("new.txt")).expect("synced"),
            "fresh"
        );
        s.dispatch("sync.watch.stop", &json!({})).expect("stop");
        assert!(s.sync.current_state().is_some_and(|st| !st.watching()));
    }

    #[test]
    fn stopping_a_run_yourself_is_not_reported_as_an_error() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        for n in 0..50 {
            std::fs::write(l.join(format!("f{n}.txt")), "x").expect("w");
        }
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        s.dispatch("sync.run", &json!({})).expect("run");
        s.dispatch("sync.cancel", &json!({})).expect("cancel");
        pump_until_idle(&mut s);
        let cancelled = s
            .sync
            .current_state()
            .and_then(|st| st.report.as_ref().map(|rep| rep.cancelled))
            .unwrap_or(false);
        // The run may finish before the cancel lands; a stopped one is an info, never a notice.
        if cancelled {
            assert!(s.notice.is_none() && s.info.is_some());
        }
    }

    #[test]
    fn cancelling_the_compare_that_keep_in_sync_started_undoes_the_choice() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        let mut s = session_with(&l, &r, &c);
        s.dispatch(
            "sync.profile.set",
            &json!({"patch": {"watch": {"enabled": true}}}),
        )
        .expect("set");
        s.dispatch("sync.watch.start", &json!({})).expect("start");
        s.dispatch("sync.cancel", &json!({})).expect("cancel");
        pump_until_idle(&mut s);
        let st = s.sync.current_state().expect("state");
        // A tiny compare can finish before the cancel lands; only a cancelled one must reset.
        if st.arming == Arming::Off {
            assert!(!st.profile.watch.enabled && !st.watching());
        }
    }

    #[test]
    fn a_filtered_tree_keeps_the_folders_of_the_rows_it_shows() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        for side in [&l, &r] {
            std::fs::create_dir_all(side.join("Family/2024")).expect("mkdir");
            std::fs::create_dir_all(side.join("Work")).expect("mkdir");
            std::fs::write(side.join("same.txt"), "s").expect("w");
            std::fs::write(side.join("Work/same.txt"), "s").expect("w");
        }
        std::fs::write(l.join("Family/2024/IMG_0803.jpg"), "new").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.profile.set", &json!({"patch": {"mode": "Update"}}))
            .expect("mode");
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        s.dispatch("sync.filter", &json!({"show": "changes"}))
            .expect("filter");
        let st = s.sync.current_state().expect("state");
        let plan = st.plan.as_ref().expect("plan");
        let shown: Vec<&str> = st
            .rows
            .iter()
            .filter_map(|&n| plan.items.get(n as usize))
            .map(|i| i.rel.as_str())
            .collect();
        assert_eq!(
            shown,
            ["Family", "Family/2024", "Family/2024/IMG_0803.jpg"],
            "the changed file keeps its folders; unchanged folders and files stay hidden"
        );
    }

    #[test]
    fn the_action_filter_shows_only_rows_with_that_action() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l, &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(l.join("a.txt"), "a").expect("w");
        std::fs::write(l.join("b.txt"), "b").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        s.dispatch(
            "sync.set_action",
            &json!({"rel": "a.txt", "action": "skip"}),
        )
        .expect("skip");
        let rows = |s: &Session| s.sync.current_state().map_or(0, |st| st.rows.len());
        let all = rows(&s);
        s.dispatch("sync.filter_action", &json!({"action": "skip"}))
            .expect("filter");
        assert_eq!(rows(&s), 1);
        s.dispatch("sync.compare", &json!({}))
            .expect("compare again");
        pump_until_idle(&mut s);
        assert_eq!(rows(&s), all, "a new compare clears the action filter");
        s.dispatch("sync.filter_action", &json!({"action": "skip"}))
            .expect("filter again");
        s.dispatch("sync.filter_action", &json!({"action": "all"}))
            .expect("clear");
        assert_eq!(rows(&s), all);
    }

    #[test]
    fn ignoring_an_item_adds_it_to_the_filters_and_compares_again() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l.join("docs"), &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(l.join("a.txt"), "a").expect("w");
        std::fs::write(l.join("docs/b.txt"), "b").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        s.dispatch("sync.ignore", &json!({"rel": "a.txt"}))
            .expect("ignore file");
        pump_until_idle(&mut s);
        s.dispatch("sync.ignore", &json!({"rel": "docs", "folder": true}))
            .expect("ignore folder");
        pump_until_idle(&mut s);
        let st = s.sync.current_state().expect("state");
        assert_eq!(st.profile.filters.exclude, ["a.txt", "docs/"]);
        let items = st.plan.as_ref().map_or(0, |p| p.items.len());
        assert_eq!(items, 0);
    }

    #[test]
    fn a_folder_action_applies_to_everything_inside_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (l, r, c) = (
            tmp.path().join("l"),
            tmp.path().join("r"),
            tmp.path().join("cfg"),
        );
        for d in [&l.join("docs"), &l.join("docs-x"), &r, &c] {
            std::fs::create_dir_all(d).expect("mkdir");
        }
        std::fs::write(l.join("docs/a.txt"), "a").expect("w");
        std::fs::write(l.join("docs/b.txt"), "b").expect("w");
        std::fs::write(l.join("docs-x/c.txt"), "c").expect("w");
        let mut s = session_with(&l, &r, &c);
        s.dispatch("sync.compare", &json!({})).expect("compare");
        pump_until_idle(&mut s);
        s.dispatch(
            "sync.set_action",
            &json!({"rel": "docs", "action": "skip", "recursive": true}),
        )
        .expect("skip folder");
        let plan = s.sync.current_state().and_then(|st| st.plan.as_ref());
        let action = |rel: &str| {
            plan.and_then(|p| p.items.iter().find(|i| i.rel == rel))
                .map(|i| i.action)
        };
        assert_eq!(action("docs/a.txt"), Some(Action::Skip));
        assert_eq!(action("docs/b.txt"), Some(Action::Skip));
        assert_eq!(action("docs"), Some(Action::Skip));
        assert_ne!(action("docs-x/c.txt"), Some(Action::Skip));
    }

    #[test]
    fn bad_input_is_an_error() {
        let mut s = Session::new().expect("session");
        assert!(s.dispatch("sync.compare", &json!({})).is_err());
        s.dispatch("sync.open", &json!({})).expect("open");
        for (id, p) in [
            ("sync.profile.set", json!({"patch": {"nope": 1}})),
            ("sync.profile.set", json!({"patch": {"mode": "Sideways"}})),
            ("sync.set_action", json!({"rel": "a", "action": "boom"})),
            ("sync.filter", json!({"show": 3})),
            ("sync.filter_action", json!({"action": "boom"})),
            ("sync.filter_action", json!({})),
            ("sync.ignore", json!({})),
            ("sync.ignore", json!({"rel": "a*"})),
            ("sync.profile.rename", json!({"name": "\u{7}"})),
            ("sync.profile.delete", json!({"id": "missing"})),
            ("sync.watch.confirm", json!({})),
            ("sync.open", json!({"id": "missing"})),
            ("sync.compare", json!({})), // empty folders
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
    }
}
