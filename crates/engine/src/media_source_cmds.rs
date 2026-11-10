//! Resolve dropped files and non-recursive folders off-thread before enqueueing.

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
}

#[derive(Debug)]
struct Pending {
    receiver: Receiver<Result<Loaded, String>>,
    original: Vec<VfsUri>,
    play_now: bool,
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
            let first = if pending.play_now && has_items {
                self.media_insert_next(loaded.items)
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
            for notice in &loaded.notices {
                self.messages.push(notice);
            }
            if !loaded.notices.is_empty() {
                self.notice = Some(format!(
                    "{} media items were skipped; see Messages.",
                    loaded.notices.len()
                ));
            }
            if has_items && (pending.play_now || !self.media.playing) {
                self.media_play_appended(first)
                    .map_err(|error| error.to_string())?;
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
        });
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
            let mut children = backend.read_dir(source)?;
            inspected = inspected.saturating_add(children.len());
            if inspected > MAX_DIR_ENTRIES {
                return Err(bad(
                    "Too many folder entries in one media import; drop fewer folders.",
                ));
            }
            fm_layout::SortSpec::default().sort(&mut children);
            let before = items.len();
            for child in children {
                if child.is_file() && validate_media(&child.uri).is_ok() {
                    if items.len() == MAX_QUEUE_ITEMS {
                        return Err(bad("Too many media files; the queue limit is 10,000."));
                    }
                    items.push(child.uri);
                }
            }
            if items.len() == before {
                notices.push(fm_vfs::redact::redact(&format!(
                    "No supported local media files in {} (subfolders are not scanned).",
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
    Ok(Loaded { items, notices })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.drop",
            label: "Enqueue Dropped Media",
            menu: &["Media", "Enqueue dropped media"],
            shortcut: None,
            params: "{uris: string[]}: enqueue local files/folders in payload order; auto-play when idle",
            enabled: |s| !s.media_source.is_busy(),
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("uris: array of paths is required"))?;
                if map.keys().any(|key| key != "uris") {
                    return Err(bad("only uris: array of paths is accepted"));
                }
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
                s.submit_media_sources(sources, notices, false)
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
                s.submit_media_sources(vec![uri], Vec::new(), true)
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
    fn folder_expansion_is_nonrecursive_natural_and_preserves_payload_order() {
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
                "last.flac"
            ]
        );
        assert!(loaded.notices.is_empty());
        assert!(load(&[uri("/music/first.mp3")], &fs, Vec::new(), true).is_err());
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
        assert_eq!(s.media.queue.items().len(), 4);
        assert_eq!(s.media.queue.current_index(), Some(0));
        assert!(s.media.playing);
        assert!(fs.stat(&uri("/music/first.mp3")).is_ok());
        assert!(fs.stat(&uri("/music/album/song2.mp3")).is_ok());
        assert!(s.fileops.jobs().is_empty());
    }

    #[test]
    fn idle_drop_selects_the_first_new_item_and_starts_playback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.mp3");
        let item = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
        let mut s = Session::new().expect("session");
        s.media
            .queue
            .enqueue(item.clone())
            .expect("old paused item");
        let (tx, receiver) = sync_channel(1);
        tx.send(Ok(Loaded {
            items: vec![item],
            notices: Vec::new(),
        }))
        .expect("result");
        s.media_source.pending = Some(Pending {
            receiver,
            original: s.media.queue.items().to_vec(),
            play_now: false,
        });
        assert!(s.media_source_tick());
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert!(s.media.playing);
        assert!(s.media_pane().is_some());
        s.media_stop();
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
