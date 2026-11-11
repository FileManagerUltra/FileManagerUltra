//! Resolve dropped files and folders (scanned recursively) off-thread before enqueueing.

use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use fm_media::MAX_QUEUE_ITEMS;
use fm_vfs::{FmError, MAX_DIR_ENTRIES, Vfs, VfsUri};

use crate::file_cmds::{focused_dir, focused_selection, object};
use crate::media_cmds::{validate_media, validate_media_source};
use crate::registry::{CommandSpec, Params};
use crate::{Session, jobs::JobLane};

#[derive(Debug)]
struct Loaded {
    items: Vec<VfsUri>,
    notices: Vec<String>,
    /// Folders that were added whole, as ranges of `items`.
    folders: Vec<AddedFolder>,
}

#[derive(Debug)]
struct AddedFolder {
    offset: usize,
    len: usize,
    name: String,
    dir: String,
}

#[derive(Debug)]
struct Pending {
    receiver: Receiver<Result<Loaded, String>>,
    original: Vec<VfsUri>,
    play_now: bool,
    /// Queue folder the items are dropped into.
    group: Option<u32>,
    /// Where the items land instead of the end of the queue.
    slot: Option<Slot>,
}

/// A queue row to drop beside: `after` or before it, `outside` its whole folder.
#[derive(Clone, Copy, Debug)]
struct Slot {
    row: usize,
    after: bool,
    outside: bool,
}

#[derive(Debug, Default)]
pub(crate) struct MediaSourceJob {
    pending: Option<Pending>,
}

impl MediaSourceJob {
    pub(crate) fn is_busy(&self) -> bool {
        self.pending.is_some()
    }
}

fn slot_from(
    s: &Session,
    map: &serde_json::Map<String, serde_json::Value>,
    has_group: bool,
) -> Result<Option<Slot>, FmError> {
    let (has_after, has_before) = (map.contains_key("after"), map.contains_key("before"));
    if !has_after && !has_before {
        return if map.contains_key("outside") {
            Err(bad("outside needs after or before"))
        } else {
            Ok(None)
        };
    }
    if has_after == has_before || has_group {
        return Err(bad(
            "give one of after and before, and not together with group",
        ));
    }
    let key = if has_after { "after" } else { "before" };
    let row = map
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|row| usize::try_from(row).ok())
        .filter(|row| *row < s.media.queue.items().len())
        .ok_or_else(|| bad(&format!("{key} must be a queue row")))?;
    let outside = match map.get("outside") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| bad("outside must be a boolean"))?,
    };
    if !outside && s.media.queue.is_locked(row) {
        return Err(bad("the folder is locked: unlock it to add files"));
    }
    Ok(Some(Slot {
        row,
        after: has_after,
        outside,
    }))
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

impl Session {
    pub(crate) fn media_source_tick(&mut self) -> bool {
        let Some(pending) = &self.media_source.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Media import background job stopped.".into()),
        };
        let Some(pending) = self.media_source.pending.take() else {
            return false;
        };
        let result = result.and_then(|loaded| {
            if self.media.queue.items() != pending.original {
                return Err(
                    "Media was not added because the queue changed during import. Try again."
                        .into(),
                );
            }
            if self
                .media
                .queue
                .items()
                .len()
                .saturating_add(loaded.items.len())
                > MAX_QUEUE_ITEMS
            {
                return Err("The dropped media does not fit in the queue.".into());
            }
            let has_items = !loaded.items.is_empty();
            let count = loaded.items.len();
            let into_group = pending.group.filter(|_| has_items);
            let folders = if into_group.is_some() {
                Vec::new()
            } else {
                loaded.folders
            };
            let resume = folders.iter().find_map(|folder| {
                let members = loaded
                    .items
                    .get(folder.offset..folder.offset + folder.len)?;
                let (offset, position) = self.media.resume_in(&folder.dir, members)?;
                Some((folder.offset + offset, position))
            });
            let mut first = if pending.play_now && has_items && into_group.is_none() {
                self.media_insert_next_with(loaded.items, !folders.is_empty())
                    .map_err(|error| error.to_string())?
            } else {
                let first = self.media.queue.items().len();
                for item in loaded.items {
                    self.media
                        .queue
                        .enqueue(item)
                        .map_err(|error| error.to_string())?;
                }
                first
            };
            if let Some(slot) = pending
                .slot
                .filter(|_| into_group.is_none() && folders.is_empty() && has_items)
            {
                let appended = first;
                let mut previous = None;
                for k in 0..count {
                    let index = appended + k;
                    let queue = &mut self.media.queue;
                    let to = match previous {
                        Some(prev) => queue.place_after(index, prev),
                        None if slot.outside => {
                            queue.place_beside_group(index, slot.row, slot.after)
                        }
                        None if slot.after => queue.place_after(index, slot.row),
                        None => queue.place_before(index, slot.row),
                    }
                    .map_err(|error| error.to_string())?;
                    if previous.is_none() {
                        first = to;
                    }
                    previous = Some(to);
                }
            }
            if let Some(id) = into_group {
                let mut moved = first;
                for at in first..first + count {
                    let now = self
                        .media_group_add(at, id)
                        .map_err(|error| error.to_string())?;
                    if at == first {
                        moved = now;
                    }
                }
                first = moved;
            }
            for folder in &folders {
                self.media
                    .queue
                    .group_range(
                        first + folder.offset,
                        folder.len,
                        &folder.name,
                        Some(folder.dir.clone()),
                    )
                    .map_err(|error| error.to_string())?;
            }
            for notice in &loaded.notices {
                self.messages.push(notice);
            }
            if !loaded.notices.is_empty() {
                self.notice = Some(format!(
                    "{} media items were skipped; see Messages.",
                    loaded.notices.len()
                ));
            }
            if has_items
                && (pending.play_now || (self.prefs.media_autoplay_on_add && !self.media.playing))
            {
                let started = match resume {
                    Some((offset, position)) => {
                        self.media_start_at(first.saturating_add(offset), position)
                    }
                    None => self.media_play_appended(first),
                };
                started.map_err(|error| error.to_string())?;
            }
            Ok(())
        });
        if let Err(message) = result {
            let message = fm_vfs::redact::redact(&message);
            self.messages.push(&message);
            self.notice = Some(message);
        }
        true
    }

    fn submit_media_sources(
        &mut self,
        sources: Vec<VfsUri>,
        notices: Vec<String>,
        folder: bool,
        group: Option<u32>,
        slot: Option<Slot>,
    ) -> Result<(), FmError> {
        if self.media_source.is_busy() {
            return Err(bad("Wait for the current media import to finish."));
        }
        let backend = self
            .backends
            .get(fm_vfs::Scheme::File)
            .ok_or_else(|| bad("The local filesystem is unavailable."))?;
        let (tx, receiver) = sync_channel(1);
        self.jobs.submit(JobLane::Local, move |_| {
            let result = load(&sources, &*backend, notices, folder)
                .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
            let _ = tx.send(result);
            Ok(())
        });
        self.media_source.pending = Some(Pending {
            receiver,
            original: self.media.queue.items().to_vec(),
            play_now: folder,
            group,
            slot,
        });
        Ok(())
    }
}

/// How many directory levels below a dropped folder are scanned for books.
const MAX_SCAN_DEPTH: usize = 8;

/// Walks a dropped folder: every directory holding two or more media files becomes a
/// playlist folder named after its path below the dropped one ("Series / Book 1").
struct Scan {
    items: Vec<VfsUri>,
    folders: Vec<AddedFolder>,
    inspected: usize,
    max_depth: usize,
}

impl Scan {
    fn dir(
        &mut self,
        backend: &dyn Vfs,
        dir: &VfsUri,
        rel: &mut Vec<String>,
        depth: usize,
    ) -> Result<(), FmError> {
        let mut children = backend.read_dir(dir)?;
        self.inspected = self.inspected.saturating_add(children.len());
        if self.inspected > MAX_DIR_ENTRIES {
            return Err(bad(
                "Too many folder entries in one media import; drop fewer folders.",
            ));
        }
        fm_layout::SortSpec::default().sort(&mut children);
        let before = self.items.len();
        for child in &children {
            if child.is_file() && validate_media(&child.uri).is_ok() {
                if self.items.len() == MAX_QUEUE_ITEMS {
                    return Err(bad("Too many media files; the queue limit is 10,000."));
                }
                self.items.push(child.uri.clone());
            }
        }
        let count = self.items.len() - before;
        // A dropped folder is a playlist folder even with one file (an .m4b book).
        if count >= 1 {
            let name: String = rel
                .join(" / ")
                .chars()
                .filter(|c| !c.is_control())
                .take(fm_media::MAX_GROUP_NAME)
                .collect();
            self.folders.push(AddedFolder {
                offset: before,
                len: self.items.len() - before,
                name: if name.trim().is_empty() {
                    "Folder".into()
                } else {
                    name
                },
                dir: dir.path_str().to_string(),
            });
        }
        if depth >= self.max_depth {
            return Ok(());
        }
        let subdirs: Vec<_> = children
            .into_iter()
            .filter(|child| {
                child.is_dir() && child.uri.file_name().is_some_and(|n| !n.starts_with('.'))
            })
            .collect();
        // A lone subfolder of a folder without media is a wrapper around the same book.
        let wrapper = count == 0 && subdirs.len() == 1;
        for child in subdirs {
            if !wrapper {
                rel.push(child.uri.file_name().unwrap_or("").to_string());
            }
            self.dir(backend, &child.uri, rel, depth + 1)?;
            if !wrapper {
                rel.pop();
            }
        }
        Ok(())
    }
}

fn load(
    sources: &[VfsUri],
    backend: &dyn Vfs,
    mut notices: Vec<String>,
    folder_only: bool,
) -> Result<Loaded, FmError> {
    let mut items = Vec::new();
    let mut folders = Vec::new();
    let mut inspected = 0usize;
    for source in sources {
        let entry = match backend.stat(source) {
            Ok(entry) => entry,
            Err(error) => {
                if folder_only {
                    return Err(error);
                }
                notices.push(fm_vfs::redact::redact(&format!(
                    "Skipped media item {}: {error}.",
                    source.file_name().unwrap_or("item")
                )));
                continue;
            }
        };
        if folder_only && !entry.is_dir() {
            return Err(bad("Choose a folder for Play Folder."));
        }
        if entry.is_dir() {
            let before = items.len();
            let mut scan = Scan {
                items,
                folders,
                inspected,
                max_depth: if folder_only { 0 } else { MAX_SCAN_DEPTH },
            };
            let root = source.file_name().unwrap_or("Folder").to_string();
            scan.dir(backend, source, &mut vec![root], 0)?;
            (items, folders, inspected) = (scan.items, scan.folders, scan.inspected);
            if items.len() == before {
                notices.push(fm_vfs::redact::redact(&format!(
                    "No supported local media files in {}.",
                    source.file_name().unwrap_or("folder")
                )));
            }
        } else if !folder_only && entry.is_file() && validate_media_source(source).is_ok() {
            if items.len() == MAX_QUEUE_ITEMS {
                return Err(bad("Too many media files; the queue limit is 10,000."));
            }
            items.push(source.clone());
        } else {
            notices.push(fm_vfs::redact::redact(&format!(
                "Skipped unsupported media item {}.",
                source.file_name().unwrap_or("item")
            )));
        }
    }
    Ok(Loaded {
        items,
        notices,
        folders,
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.drop",
            label: "Enqueue Dropped Media",
            menu: &["Media", "Enqueue dropped media"],
            shortcut: None,
            params: "{uris: string[], group?: number, after?: number | before?: number, outside?: bool}: enqueue local files/folders in payload order; auto-play when idle; `group` adds them to that queue folder; `after`/`before` put them beside that queue row (like media.place) instead of at the end",
            enabled: |s| !s.media_source.is_busy(),
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("uris: array of paths is required"))?;
                if map.keys().any(|key| {
                    !["uris", "group", "after", "before", "outside"].contains(&key.as_str())
                }) {
                    return Err(bad(
                        "only uris, group, after, before and outside are accepted",
                    ));
                }
                let group = match map.get("group") {
                    None => None,
                    Some(value) => Some(
                        value
                            .as_u64()
                            .and_then(|id| u32::try_from(id).ok())
                            .filter(|id| s.media.queue.groups().iter().any(|g| g.id == *id))
                            .ok_or_else(|| bad("group must be an existing queue folder"))?,
                    ),
                };
                if group.is_some_and(|id| {
                    s.media
                        .queue
                        .groups()
                        .iter()
                        .any(|g| g.id == id && g.locked)
                }) {
                    return Err(bad("the folder is locked: unlock it to add files"));
                }
                let slot = slot_from(s, map, group.is_some())?;
                let values = map
                    .get("uris")
                    .and_then(Params::as_array)
                    .filter(|values| !values.is_empty() && values.len() <= MAX_QUEUE_ITEMS)
                    .ok_or_else(|| bad("uris must contain 1–10,000 paths"))?;
                let mut sources = Vec::new();
                let mut notices = Vec::new();
                for value in values {
                    let raw = value
                        .as_str()
                        .ok_or_else(|| bad("uris must contain strings"))?;
                    match VfsUri::parse(raw) {
                        Ok(uri) if uri.local_path().is_some() => sources.push(uri),
                        _ => notices.push("Skipped an invalid or remote media path; copy remote files locally first.".into()),
                    }
                }
                s.submit_media_sources(sources, notices, false, group, slot)
            },
        },
        CommandSpec {
            id: "media.play_folder",
            label: "Play Folder",
            menu: &["Media", "Play folder"],
            shortcut: None,
            params: "{uri?: string, pane?: number}: insert a folder's media after current in natural order and play; Preview uses its file's parent",
            enabled: |s| {
                !s.media_source.is_busy()
                    && (focused_dir(s).is_some() || crate::media_entry_cmds::has_sources(s))
            },
            run: |s, params| {
                let map = object(params)?;
                if map.is_some_and(|map| {
                    map.keys()
                        .any(|key| !matches!(key.as_str(), "uri" | "pane"))
                }) {
                    return Err(bad("only uri or pane is accepted"));
                }
                if map.is_some_and(|map| map.contains_key("uri") && map.contains_key("pane")) {
                    return Err(bad("choose either uri or a Preview pane"));
                }
                let uri = if let Some(value) = map.and_then(|map| map.get("uri")) {
                    let raw = value
                        .as_str()
                        .ok_or_else(|| bad("uri must be a local folder path"))?;
                    VfsUri::parse(raw).map_err(|_| bad("invalid media folder"))?
                } else if map.is_some_and(|map| map.contains_key("pane"))
                    || s.pane_kind(s.layout.focused()) == Some(fm_layout::PaneKind::Preview)
                {
                    let input = crate::media_entry_cmds::sources(s, map)?;
                    input
                        .items
                        .first()
                        .and_then(VfsUri::parent)
                        .ok_or_else(|| bad("The previewed file needs a parent folder."))?
                } else {
                    match focused_selection(s).as_slice() {
                        [one]
                            if one
                                .parent()
                                .and_then(|parent| s.dirs.entries(&parent))
                                .is_some_and(|entries| {
                                    entries
                                        .iter()
                                        .any(|entry| entry.uri == *one && entry.is_dir())
                                })
                                || s.layout
                                    .state(s.layout.focused())
                                    .is_some_and(|state| state.tree.active) =>
                        {
                            one.clone()
                        }
                        _ => focused_dir(s).ok_or_else(|| bad("Choose a local folder."))?,
                    }
                };
                if uri.local_path().is_none() {
                    return Err(bad(
                        "Choose a local folder; remote media is not available yet.",
                    ));
                }
                s.submit_media_sources(vec![uri], Vec::new(), true, None, None)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_testkit::MemFs;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn uri(path: &str) -> VfsUri {
        VfsUri::parse(path).expect("uri")
    }

    fn fixture() -> (Session, MemFs) {
        let fs = MemFs::new();
        fs.mkdirs_str("/music/album/nested").expect("folder");
        for path in [
            "/music/first.mp3",
            "/music/last.flac",
            "/music/album/song10.mp3",
            "/music/album/song2.mp3",
            "/music/album/nested/hidden.mp3",
            "/music/album/readme.txt",
        ] {
            fs.put_str(path, b"fixture").expect("file");
        }
        let mut s = Session::new().expect("session");
        s.backends.register(Arc::new(fs.clone()));
        (s, fs)
    }

    fn wait(s: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.media_source.is_busy() {
            s.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn folder_expansion_is_natural_and_preserves_payload_order() {
        let (_, fs) = fixture();
        fs.put_str("/music/album/song3.mkv", b"video")
            .expect("video");
        let loaded = load(
            &[
                uri("/music/first.mp3"),
                uri("/music/album"),
                uri("/music/last.flac"),
            ],
            &fs,
            Vec::new(),
            false,
        )
        .expect("load");
        assert_eq!(
            loaded
                .items
                .iter()
                .filter_map(VfsUri::file_name)
                .collect::<Vec<_>>(),
            [
                "first.mp3",
                "song2.mp3",
                "song3.mkv",
                "song10.mp3",
                "hidden.mp3",
                "last.flac"
            ]
        );
        assert!(loaded.notices.is_empty());
        assert!(load(&[uri("/music/first.mp3")], &fs, Vec::new(), true).is_err());
    }

    #[test]
    fn a_series_folder_becomes_one_playlist_folder_per_book() {
        let fs = MemFs::new();
        for path in [
            "/s/Series/Book 1/01.mp3",
            "/s/Series/Book 1/02.mp3",
            "/s/Series/Book 2/01.mp3",
            "/s/Series/Book 2/02.mp3",
            "/s/Series/Book 2/.part/03.mp3",
            "/s/Series/Book 2/.part/04.mp3",
            "/s/Single/Wrapper/only.m4b",
            "/s/Zeta/Book 3/Inner/book.m4b",
        ] {
            fs.put_str(path, b"x").expect("file");
        }
        let loaded = load(&[uri("/s")], &fs, Vec::new(), false).expect("load");
        let folders: Vec<_> = loaded
            .folders
            .iter()
            .map(|f| (f.name.as_str(), f.offset, f.len))
            .collect();
        assert_eq!(
            folders,
            [
                ("s / Series / Book 1", 0, 2),
                ("s / Series / Book 2", 2, 2),
                ("s / Single", 4, 1),
                ("s / Zeta", 5, 1)
            ]
        );
        assert_eq!(loaded.items.len(), 6, "hidden folders are skipped");
        assert_eq!(loaded.folders[1].dir, "/s/Series/Book 2");
    }

    #[test]
    fn drops_append_without_interrupting_playback_and_never_move_files() {
        let (mut s, fs) = fixture();
        s.media
            .queue
            .enqueue(uri("/music/last.flac"))
            .expect("queue");
        s.media.queue.select(0).expect("select");
        s.media.playing = true;
        s.dispatch(
            "media.drop",
            &json!({"uris": ["/music/album", "/music/first.mp3"]}),
        )
        .expect("drop");
        assert_eq!(s.media.queue.items().len(), 1);
        wait(&mut s);
        assert_eq!(s.media.queue.items().len(), 5);
        assert_eq!(s.media.queue.current_index(), Some(0));
        assert!(s.media.playing);
        assert!(fs.stat(&uri("/music/first.mp3")).is_ok());
        assert!(fs.stat(&uri("/music/album/song2.mp3")).is_ok());
        assert!(s.fileops.jobs().is_empty());
    }

    fn drop_one_item(autoplay: bool) -> Session {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.mp3");
        let item = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
        let mut s = Session::new().expect("session");
        s.prefs.media_autoplay_on_add = autoplay;
        s.media
            .queue
            .enqueue(item.clone())
            .expect("old paused item");
        let (tx, receiver) = sync_channel(1);
        tx.send(Ok(Loaded {
            items: vec![item],
            notices: Vec::new(),
            folders: Vec::new(),
        }))
        .expect("result");
        s.media_source.pending = Some(Pending {
            receiver,
            original: s.media.queue.items().to_vec(),
            play_now: false,
            group: None,
            slot: None,
        });
        assert!(s.media_source_tick());
        s
    }

    #[test]
    fn idle_drop_starts_playback_only_when_autoplay_on_add_is_enabled() {
        let mut s = drop_one_item(true);
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert!(s.media.playing);
        assert!(s.media_pane().is_some());
        s.media_stop();
    }

    #[test]
    fn adding_items_does_not_autoplay_by_default() {
        let s = drop_one_item(false);
        assert_eq!(s.media.queue.items().len(), 2);
        assert_eq!(s.media.queue.current_index(), None);
        assert!(!s.media.playing);
    }

    #[test]
    fn adding_a_known_folder_continues_where_it_was_left() {
        let (mut s, _) = fixture();
        s.media.resume.push(crate::media_resume::ResumePoint {
            dir: "/books/ghost".into(),
            file: "02.mp3".into(),
            position: 100.0,
        });
        let items = vec![
            uri("/books/ghost/01.mp3"),
            uri("/books/ghost/02.mp3"),
            uri("/books/ghost/03.mp3"),
        ];
        let (tx, receiver) = sync_channel(1);
        tx.send(Ok(Loaded {
            items,
            notices: Vec::new(),
            folders: vec![AddedFolder {
                offset: 0,
                len: 3,
                name: "ghost".into(),
                dir: "/books/ghost".into(),
            }],
        }))
        .expect("result");
        s.media_source.pending = Some(Pending {
            receiver,
            original: s.media.queue.items().to_vec(),
            play_now: true,
            group: None,
            slot: None,
        });
        assert!(s.media_source_tick());
        let folder = s.media.queue.groups();
        assert_eq!(folder.len(), 1);
        assert_eq!((folder[0].name.as_str(), folder[0].len), ("ghost", 3));
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert!((s.media.position - 97.0).abs() < 1.0);
        s.media_stop();
    }

    #[test]
    fn dropping_files_with_a_group_adds_them_to_that_folder() {
        let (mut s, _) = fixture();
        s.media
            .queue
            .enqueue(uri("/music/first.mp3"))
            .expect("edit");
        s.media
            .queue
            .enqueue(uri("/music/first.mp3"))
            .expect("edit");
        s.media.queue.group_range(0, 1, "f", None).expect("group");
        let id = s.media.queue.groups()[0].id;
        s.dispatch(
            "media.drop",
            &json!({"uris": ["/music/first.mp3"], "group": id}),
        )
        .expect("drop");
        wait(&mut s);
        let groups = s.media.queue.groups();
        assert_eq!((groups.len(), groups[0].len), (1, 2));
        assert_eq!(s.media.queue.items().len(), 3);
    }

    #[test]
    fn dropping_files_beside_a_row_places_them_there_in_order() {
        let (mut s, _) = fixture();
        for name in ["a", "b", "c"] {
            s.media
                .queue
                .enqueue(uri(&format!("/music/{name}.mp3")))
                .expect("edit");
        }
        s.dispatch(
            "media.drop",
            &json!({"uris": ["/music/first.mp3", "/music/last.flac"], "after": 0}),
        )
        .expect("drop");
        wait(&mut s);
        let names: Vec<_> = s
            .media
            .queue
            .items()
            .iter()
            .filter_map(|u| u.file_name().map(str::to_owned))
            .collect();
        assert_eq!(names, ["a.mp3", "first.mp3", "last.flac", "b.mp3", "c.mp3"]);
        s.media_stop();
        for bad in [
            json!({"uris": ["/music/first.mp3"], "after": 99}),
            json!({"uris": ["/music/first.mp3"], "after": 0, "before": 1}),
            json!({"uris": ["/music/first.mp3"], "outside": true}),
        ] {
            assert!(s.dispatch("media.drop", &bad).is_err());
        }
    }

    #[test]
    fn stale_imports_and_over_capacity_folders_leave_the_queue_unchanged() {
        let (mut s, _) = fixture();
        s.dispatch("media.drop", &json!({"uris": ["/music/album"]}))
            .expect("drop");
        s.media
            .queue
            .enqueue(uri("/music/first.mp3"))
            .expect("edit");
        wait(&mut s);
        assert_eq!(s.media.queue.items().len(), 1);
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("queue changed"))
        );
        for _ in 1..MAX_QUEUE_ITEMS {
            s.media
                .queue
                .enqueue(uri("/music/first.mp3"))
                .expect("fill queue");
        }
        s.dispatch("media.play_folder", &json!({"uri": "/music/album"}))
            .expect("folder");
        wait(&mut s);
        assert_eq!(s.media.queue.items().len(), MAX_QUEUE_ITEMS);
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("does not fit"))
        );
    }

    #[test]
    fn invalid_paths_are_reported_without_credentials_and_bad_params_start_no_job() {
        let (mut s, _) = fixture();
        s.dispatch(
            "media.drop",
            &json!({"uris": ["https://alice:secret@host/song.mp3", "/music/absent.mp3"]}),
        )
        .expect("submit");
        wait(&mut s);
        assert!(s.media.queue.items().is_empty());
        assert!(!s.messages.is_empty());
        assert!(
            s.messages
                .recent()
                .all(|message| !message.text.contains("secret"))
        );
        for (command, params) in [
            ("media.drop", Params::Null),
            ("media.drop", json!({"uris": []})),
            ("media.drop", json!({"uris": [4]})),
            (
                "media.drop",
                json!({"uris": ["/music/first.mp3"], "move": true}),
            ),
            ("media.play_folder", json!({"uri": 4})),
            (
                "media.play_folder",
                json!({"uri": "sftp://alice@host/music"}),
            ),
            (
                "media.play_folder",
                json!({"uri": "/music/album", "recursive": true}),
            ),
        ] {
            assert!(s.dispatch(command, &params).is_err(), "{command}: {params}");
            assert!(!s.media_source.is_busy());
        }
    }
}
