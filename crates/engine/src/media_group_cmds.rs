//! Folders in the media queue: create, rename, fill, empty and delete.

use fm_layout::PaneKind;
use fm_vfs::FmError;
use serde_json::Value;

use crate::Session;
use crate::file_cmds::{Obj, object};
use crate::registry::CommandSpec;

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn params<'a>(p: &'a Value, allowed: &[&str]) -> Result<&'a Obj, FmError> {
    let map = object(p)?.ok_or_else(|| bad("parameters are required"))?;
    if map.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(bad("unknown parameter"));
    }
    Ok(map)
}

fn number(map: &Obj, key: &str) -> Result<usize, FmError> {
    map.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| bad(&format!("{key} must be a non-negative number")))
}

enum Place {
    After(usize),
    Before(usize),
}

fn place_target(map: &Obj) -> Result<Place, FmError> {
    match (map.contains_key("after"), map.contains_key("before")) {
        (true, false) => Ok(Place::After(number(map, "after")?)),
        (false, true) => Ok(Place::Before(number(map, "before")?)),
        _ => Err(bad("give exactly one of after and before")),
    }
}

fn group_id(map: &Obj, key: &str) -> Result<u32, FmError> {
    u32::try_from(number(map, key)?).map_err(|_| bad(&format!("{key} is not a folder")))
}

fn text<'a>(map: &'a Obj, key: &str) -> Result<&'a str, FmError> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(&format!("{key} must be a string")))
}

impl Session {
    /// A queue row moved from `from` to `to` (as in `Queue::move_item`): keep pane selections on their rows.
    fn media_remap_selected(&mut self, from: usize, to: usize) {
        for state in self
            .layout
            .states
            .values_mut()
            .filter(|state| state.kind == PaneKind::Media)
        {
            state.media.selected = state.media.selected.map(|selected| {
                if selected == from {
                    to
                } else if from < selected && selected <= to {
                    selected - 1
                } else if to <= selected && selected < from {
                    selected + 1
                } else {
                    selected
                }
            });
        }
    }

    pub(crate) fn media_group_add(&mut self, index: usize, id: u32) -> Result<usize, FmError> {
        let to = self.media.queue.add_to_group(index, id)?;
        self.media_remap_selected(index, to);
        Ok(to)
    }

    pub(crate) fn media_remove_item(&mut self, index: usize) -> Result<(), FmError> {
        if self.media.queue.is_locked(index) {
            return Err(bad("the folder is locked: unlock it to remove items"));
        }
        let current = self.media.queue.current_index() == Some(index);
        let playing = self.media.playing;
        self.media.queue.remove(index)?;
        for state in self
            .layout
            .states
            .values_mut()
            .filter(|state| state.kind == PaneKind::Media)
        {
            state.media.selected = state
                .media
                .selected
                .filter(|selected| *selected != index)
                .map(|selected| {
                    if selected > index {
                        selected - 1
                    } else {
                        selected
                    }
                });
        }
        if current {
            self.media.reset_item();
            if playing && self.media.queue.current().is_some() {
                self.media_resume()?;
            } else {
                self.media_stop();
            }
        }
        Ok(())
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.group_create",
            label: "New Playlist Folder",
            menu: &["Media", "New folder"],
            shortcut: None,
            params: "{name?: string, index?: number}: add an empty folder at the end of the queue (default name \"New folder\"); with `index` that row moves into it",
            enabled: |_| true,
            run: |s, p| {
                let empty = Obj::new();
                let map = if p.is_null() {
                    &empty
                } else {
                    params(p, &["name", "index"])?
                };
                let name = match map.get("name") {
                    Some(_) => text(map, "name")?,
                    None => "New folder",
                };
                let index = map.get("index").map(|_| number(map, "index")).transpose()?;
                if let Some(index) = index
                    && index >= s.media.queue.items().len()
                {
                    return Err(bad("index is not a queue row"));
                }
                let id = s.media.queue.create_group(name, None)?;
                if let Some(index) = index {
                    s.media_group_add(index, id)?;
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.group_rename",
            label: "Rename Playlist Folder",
            menu: &["Media", "Rename folder"],
            shortcut: None,
            params: "{id: number, name: string}",
            enabled: |s| !s.media.queue.groups().is_empty(),
            run: |s, p| {
                let map = params(p, &["id", "name"])?;
                let id = group_id(map, "id")?;
                s.media.queue.rename_group(id, text(map, "name")?)
            },
        },
        CommandSpec {
            id: "media.group_lock",
            label: "Lock Playlist Folder",
            menu: &["Media", "Lock folder"],
            shortcut: None,
            params: "{id: number, locked?: bool}: lock or unlock a folder (toggle when `locked` is absent); a locked folder keeps its items",
            enabled: |s| !s.media.queue.groups().is_empty(),
            run: |s, p| {
                let map = params(p, &["id", "locked"])?;
                let id = group_id(map, "id")?;
                let locked = match map.get("locked") {
                    Some(value) => value
                        .as_bool()
                        .ok_or_else(|| bad("locked must be a boolean"))?,
                    None => !s
                        .media
                        .queue
                        .groups()
                        .iter()
                        .any(|g| g.id == id && g.locked),
                };
                s.media.queue.set_group_locked(id, locked)
            },
        },
        CommandSpec {
            id: "media.group_add",
            label: "Move to Playlist Folder",
            menu: &["Media", "Move to folder"],
            shortcut: None,
            params: "{index: number, group: number}: move queue row `index` to the end of folder `group`",
            enabled: |s| !s.media.queue.groups().is_empty(),
            run: |s, p| {
                let map = params(p, &["index", "group"])?;
                let (index, id) = (number(map, "index")?, group_id(map, "group")?);
                s.media_group_add(index, id).map(drop)
            },
        },
        CommandSpec {
            id: "media.ungroup",
            label: "Remove from Playlist Folder",
            menu: &["Media", "Remove from folder"],
            shortcut: None,
            params: "{index: number}: take queue row `index` out of its folder, placing it just after",
            enabled: |s| s.media.queue.groups().iter().any(|g| g.len > 0),
            run: |s, p| {
                let map = params(p, &["index"])?;
                let index = number(map, "index")?;
                let to = s.media.queue.remove_from_group(index)?;
                s.media_remap_selected(index, to);
                Ok(())
            },
        },
        CommandSpec {
            id: "media.place",
            label: "Place After Queue Row",
            menu: &["Media", "Place after"],
            shortcut: None,
            params: "{index: number, after: number | before: number, outside?: bool}: move queue row `index` to just after row `after` (or just before row `before`), joining that row's folder; `outside` puts it before/after that row's whole folder instead",
            enabled: |s| s.media.queue.items().len() > 1,
            run: |s, p| {
                let map = params(p, &["index", "after", "before", "outside"])?;
                let index = number(map, "index")?;
                let outside = match map.get("outside") {
                    None => false,
                    Some(value) => value
                        .as_bool()
                        .ok_or_else(|| bad("outside must be a boolean"))?,
                };
                let to = match (place_target(map)?, outside) {
                    (Place::After(after), false) => s.media.queue.place_after(index, after)?,
                    (Place::Before(before), false) => s.media.queue.place_before(index, before)?,
                    (Place::After(row), true) => {
                        s.media.queue.place_beside_group(index, row, true)?
                    }
                    (Place::Before(row), true) => {
                        s.media.queue.place_beside_group(index, row, false)?
                    }
                };
                s.media_remap_selected(index, to);
                Ok(())
            },
        },
        CommandSpec {
            id: "media.group_place",
            label: "Move Playlist Folder",
            menu: &["Media", "Move folder"],
            shortcut: None,
            params: "{id: number, after: number | before: number}: move the folder to just after queue row `after` or just before row `before` (outside that row's folder when it is in one)",
            enabled: |s| s.media.queue.groups().iter().any(|g| g.len > 0),
            run: |s, p| {
                let map = params(p, &["id", "after", "before"])?;
                let id = group_id(map, "id")?;
                match place_target(map)? {
                    Place::After(after) => s.media.queue.place_group_after(id, after)?,
                    Place::Before(before) => s.media.queue.place_group_before(id, before)?,
                };
                // Rows moved in bulk: a stale single selection is worse than none.
                for state in s.layout.states.values_mut() {
                    if state.kind == PaneKind::Media {
                        state.media.selected = None;
                    }
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.group_delete",
            label: "Delete Playlist Folder",
            menu: &["Media", "Delete folder"],
            shortcut: None,
            params: "{id: number, items?: bool}: delete the folder; its items stay unless `items` is true",
            enabled: |s| !s.media.queue.groups().is_empty(),
            run: |s, p| {
                let map = params(p, &["id", "items"])?;
                let id = group_id(map, "id")?;
                let with_items = match map.get("items") {
                    None => false,
                    Some(value) => value.as_bool().ok_or_else(|| bad("items must be a bool"))?,
                };
                let group = s
                    .media
                    .queue
                    .groups()
                    .into_iter()
                    .find(|g| g.id == id)
                    .ok_or_else(|| bad("unknown folder"))?;
                if with_items {
                    for index in (group.start..group.start + group.len).rev() {
                        s.media_remove_item(index)?;
                    }
                }
                s.media.queue.delete_group(id)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use fm_vfs::VfsUri;
    use serde_json::json;

    use super::*;

    fn session(n: usize) -> Session {
        let mut s = Session::new().expect("session");
        for i in 0..n {
            let uri = VfsUri::parse(&format!("/m/{i}.mp3")).expect("uri");
            s.media.queue.enqueue(uri).expect("enqueue");
        }
        s
    }

    #[test]
    fn folders_can_be_created_filled_emptied_and_deleted() {
        let mut s = session(4);
        s.dispatch("media.group_create", &json!({"name": "Mix", "index": 1}))
            .expect("create");
        let g = s.media.queue.groups();
        assert_eq!((g[0].name.as_str(), g[0].len), ("Mix", 1));
        let id = g[0].id;
        s.dispatch("media.group_add", &json!({"index": 0, "group": id}))
            .expect("add");
        assert_eq!(s.media.queue.groups()[0].len, 2);
        s.dispatch("media.group_rename", &json!({"id": id, "name": "Road"}))
            .expect("rename");
        s.dispatch("media.ungroup", &json!({"index": 3}))
            .expect("ungroup");
        s.dispatch("media.group_delete", &json!({"id": id}))
            .expect("delete");
        assert!(s.media.queue.groups().is_empty());
        assert_eq!(s.media.queue.items().len(), 4);
        s.dispatch("media.group_create", &json!(null))
            .expect("bare");
        s.dispatch("media.group_create", &json!({"index": 0}))
            .expect("with row");
        let id = s.media.queue.groups()[0].id;
        s.dispatch("media.group_delete", &json!({"id": id, "items": true}))
            .expect("delete items");
        assert_eq!(s.media.queue.items().len(), 3);
    }

    #[test]
    fn a_locked_folder_refuses_removals_and_drops_until_unlocked() {
        let mut s = session(4);
        let id = s.media.queue.create_group("Book", None).expect("group");
        s.dispatch("media.group_add", &json!({"index": 1, "group": id}))
            .expect("add");
        s.dispatch("media.group_lock", &json!({"id": id}))
            .expect("lock");
        assert!(s.media.queue.groups()[0].locked);
        assert!(s.dispatch("media.remove", &json!({"index": 3})).is_err());
        assert!(
            s.dispatch("media.group_add", &json!({"index": 0, "group": id}))
                .is_err()
        );
        assert!(
            s.dispatch("media.drop", &json!({"uris": ["/x.mp3"], "group": id}))
                .is_err()
        );
        s.dispatch("media.group_lock", &json!({"id": id, "locked": false}))
            .expect("unlock");
        s.dispatch("media.remove", &json!({"index": 3}))
            .expect("remove");
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session(2);
        let id = s.media.queue.create_group("A", None).expect("group");
        for (cmd, bad) in [
            ("media.group_create", json!({"name": 3})),
            ("media.group_create", json!({"name": "  "})),
            ("media.group_create", json!({"index": 9})),
            ("media.group_create", json!({"x": 1})),
            ("media.group_rename", json!({"id": id})),
            ("media.group_rename", json!({"id": 99, "name": "x"})),
            ("media.group_rename", json!({"id": id, "name": "a\nb"})),
            ("media.group_add", json!({"index": 0})),
            ("media.group_add", json!({"index": 9, "group": id})),
            ("media.group_add", json!({"index": 0, "group": 99})),
            ("media.group_add", json!({"index": -1, "group": id})),
            ("media.ungroup", json!({})),
            ("media.ungroup", json!({"index": 9})),
            ("media.place", json!({"index": 0})),
            ("media.place", json!({"index": 0, "after": 9})),
            ("media.place", json!({"index": 0, "before": 9})),
            (
                "media.place",
                json!({"index": 0, "before": 1, "outside": 3}),
            ),
            ("media.place", json!({"index": 0, "after": 1, "before": 1})),
            ("media.group_place", json!({"id": id, "before": 9})),
            ("media.group_place", json!({"id": id, "after": 0})),
            ("media.group_place", json!({"id": 99, "after": 0})),
            ("media.group_delete", json!({"id": 99})),
            ("media.group_lock", json!({"id": 99})),
            ("media.group_lock", json!({"id": id, "locked": "yes"})),
            ("media.group_delete", json!({"id": id, "items": "yes"})),
            ("media.group_delete", json!([])),
            ("media.drop", json!({"uris": ["/x"], "group": 99})),
        ] {
            assert!(s.dispatch(cmd, &bad).is_err(), "{cmd} {bad}");
        }
    }
}
