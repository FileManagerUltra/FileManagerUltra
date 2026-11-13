//! Jump from the playlist to the file or folder on disk: `media.reveal`.

use fm_vfs::{FmError, VfsUri};
use serde_json::Value;

use crate::Session;
use crate::file_cmds::object;
use crate::registry::CommandSpec;
use crate::search_cmds::show_in_files;

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

/// What a playlist row or folder points at: the folder to open and the item to select in it.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Item(usize),
    Group(u32),
}

fn target(p: &Value) -> Result<Target, FmError> {
    let map = object(p)?.ok_or_else(|| bad("give index or group"))?;
    if map.keys().any(|key| key != "index" && key != "group") {
        return Err(bad("unknown parameter"));
    }
    let number = |key: &str| {
        map.get(key)
            .map(|v| {
                v.as_u64()
                    .ok_or_else(|| bad(&format!("{key} must be a non-negative number")))
            })
            .transpose()
    };
    match (number("index")?, number("group")?) {
        (Some(index), None) => usize::try_from(index)
            .map(Target::Item)
            .map_err(|_| bad("index is not a queue row")),
        (None, Some(group)) => u32::try_from(group)
            .map(Target::Group)
            .map_err(|_| bad("unknown folder")),
        _ => Err(bad("give exactly one of index and group")),
    }
}

impl Session {
    /// The folder on disk a playlist folder was made from, if any.
    pub fn media_group_dir(&self, id: u32) -> Option<VfsUri> {
        self.media
            .queue
            .groups()
            .into_iter()
            .find(|g| g.id == id)
            .and_then(|g| g.dir)
            .and_then(|dir| VfsUri::parse(&dir).ok())
    }

    fn media_reveal(&mut self, target: Target) -> Result<(), FmError> {
        match target {
            Target::Item(index) => {
                let uri = self
                    .media
                    .queue
                    .items()
                    .get(index)
                    .cloned()
                    .ok_or_else(|| bad("index is not a queue row"))?;
                let folder = uri.parent().ok_or_else(|| bad("a root has no folder"))?;
                let name = uri.file_name().unwrap_or_default().to_string();
                show_in_files(self, folder, Some(name))
            }
            Target::Group(id) => {
                if !self.media.queue.groups().iter().any(|g| g.id == id) {
                    return Err(bad("unknown folder"));
                }
                let dir = self.media_group_dir(id).ok_or_else(|| {
                    bad("this playlist folder was made in the playlist, not from a folder on disk")
                })?;
                show_in_files(self, dir, None)
            }
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "media.reveal",
        label: "Show Playlist Item in Folder",
        menu: &["Media", "Show in folder"],
        shortcut: None,
        params: "{index: number} opens the row's folder in a Files pane and selects the file; {group: number} opens the folder a playlist folder was made from",
        enabled: |s| !s.media.queue.items().is_empty(),
        run: |s, p| {
            let target = target(p)?;
            s.media_reveal(target)
        },
    }]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn uri(path: &std::path::Path) -> VfsUri {
        VfsUri::parse(path.to_str().expect("utf8")).expect("uri")
    }

    fn files_pane(s: &Session) -> fm_layout::PaneId {
        s.layout
            .panes()
            .into_iter()
            .find(|p| s.layout.state(*p).is_some_and(|x| x.kind.is_files()))
            .expect("files pane")
    }

    #[test]
    fn a_row_opens_its_folder_with_the_file_selected() {
        let tmp = tempfile::tempdir().expect("tmp");
        let album = tmp.path().join("Album");
        std::fs::create_dir(&album).expect("dir");
        std::fs::write(album.join("02 Song.mp3"), b"x").expect("w");
        let mut s = Session::new().expect("session");
        s.media
            .queue
            .enqueue(uri(&tmp.path().join("other.mp3")))
            .expect("enqueue");
        s.media
            .queue
            .enqueue(uri(&album.join("02 Song.mp3")))
            .expect("enqueue");
        s.dispatch("media.reveal", &json!({"index": 1}))
            .expect("reveal");
        let pane = files_pane(&s);
        assert_eq!(s.layout.focused(), pane);
        let state = s.layout.state(pane).expect("state");
        assert_eq!(state.location, uri(&album));
        assert_eq!(state.listing.reveal.as_deref(), Some("02 Song.mp3"));
    }

    #[test]
    fn a_folder_made_from_disk_opens_that_folder() {
        let tmp = tempfile::tempdir().expect("tmp");
        let book = tmp.path().join("Book 2");
        std::fs::create_dir(&book).expect("dir");
        let mut s = Session::new().expect("session");
        s.media
            .queue
            .enqueue(uri(&book.join("01.mp3")))
            .expect("enqueue");
        let id = s
            .media
            .queue
            .create_group("Book 2", Some(uri(&book).to_string()))
            .expect("group");
        assert_eq!(s.media_group_dir(id), Some(uri(&book)));
        s.dispatch("media.reveal", &json!({"group": id}))
            .expect("reveal");
        let state = s.layout.state(files_pane(&s)).expect("state");
        assert_eq!(state.location, uri(&book));
        assert_eq!(state.listing.reveal, None);
    }

    #[test]
    fn bad_targets_are_errors_and_change_nothing() {
        let mut s = Session::new().expect("session");
        s.media
            .queue
            .enqueue(uri(std::path::Path::new("/m/a.mp3")))
            .expect("enqueue");
        let made_here = s.media.queue.create_group("Mine", None).expect("group");
        let pane = files_pane(&s);
        let before = s.layout.state(pane).map(|x| x.location.clone());
        for params in [
            json!(null),
            json!({}),
            json!({"index": 5}),
            json!({"index": -1}),
            json!({"index": "0"}),
            json!({"group": 99}),
            json!({"group": made_here}),
            json!({"index": 0, "group": made_here}),
            json!({"index": 0, "x": 1}),
            json!({"group": u64::MAX}),
        ] {
            assert!(s.dispatch("media.reveal", &params).is_err(), "{params}");
        }
        assert_eq!(s.layout.state(pane).map(|x| x.location.clone()), before);
    }
}
