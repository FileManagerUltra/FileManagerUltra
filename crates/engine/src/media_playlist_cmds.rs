//! Bounded playlist import/export; filesystem work stays on the local job lane.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::Instant;

use fm_media::{MAX_PLAYLIST_BYTES, MAX_QUEUE_ITEMS, PlaylistFormat, parse_playlist, write_m3u8};
use fm_vfs::{FmError, Vfs, VfsUri};

use crate::file_cmds::object;
use crate::media_cmds::validate_media;
use crate::registry::{CommandSpec, Params};
use crate::{Session, jobs::JobLane};

#[derive(Debug)]
enum Outcome {
    Loaded {
        items: Vec<VfsUri>,
        notices: Vec<String>,
    },
    Saved,
}

#[derive(Debug)]
struct Pending {
    receiver: Receiver<Result<Outcome, String>>,
    original: Option<Vec<VfsUri>>,
}

#[derive(Debug, Default)]
pub(crate) struct PlaylistJob {
    pending: Option<Pending>,
}

impl PlaylistJob {
    pub(crate) fn is_busy(&self) -> bool {
        self.pending.is_some()
    }
}

impl Session {
    pub(crate) fn media_playlist_tick(&mut self) -> bool {
        let Some(pending) = &self.playlist_job.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("Playlist background job stopped.".into()),
        };
        let original = self
            .playlist_job
            .pending
            .take()
            .and_then(|pending| pending.original);
        let result = result.and_then(|outcome| match outcome {
            Outcome::Loaded { items, notices } => {
                if original.as_deref() != Some(self.media.queue.items()) {
                    return Err("Playlist was not added because the queue changed while it was loading. Load it again.".into());
                }
                if items.len().saturating_add(self.media.queue.items().len()) > MAX_QUEUE_ITEMS {
                    return Err("The playlist does not fit in the media queue.".into());
                }
                let count = items.len();
                let first = self.media.queue.items().len();
                for uri in items {
                    self.media.queue.enqueue(uri).map_err(|error| error.to_string())?;
                }
                self.media.note_added(first, count);
                for notice in &notices {
                    self.messages.push(notice);
                }
                if !notices.is_empty() {
                    self.notice = Some(format!(
                        "Added {count} playlist items. {} entries were skipped; see Messages.",
                        notices.len()
                    ));
                }
                self.info = Some((format!("Added {count} playlist items"), Instant::now()));
                Ok(())
            }
            Outcome::Saved => {
                self.info = Some(("Saved playlist".into(), Instant::now()));
                Ok(())
            }
        });
        if let Err(message) = result {
            let message = fm_vfs::redact::redact(&message);
            self.messages.push(&message);
            self.notice = Some(message);
        }
        true
    }
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn target(params: &Params, save: bool) -> Result<VfsUri, FmError> {
    let map = object(params)?.ok_or_else(|| bad("uri: local playlist path is required"))?;
    if map.keys().any(|key| key != "uri") {
        return Err(bad("only uri: local playlist path is accepted"));
    }
    let raw = map
        .get("uri")
        .and_then(Params::as_str)
        .ok_or_else(|| bad("uri: local playlist path is required"))?;
    validate_path(raw, save)
}

/// Pure validation shared with the playlist path dialog.
pub fn validate_path(raw: &str, save: bool) -> Result<VfsUri, FmError> {
    let uri = VfsUri::parse(raw).map_err(|_| bad("invalid playlist path"))?;
    if uri.local_path().is_none() {
        return Err(bad(
            "Choose a local playlist file; remote playlists are not available yet.",
        ));
    }
    let format = format(&uri)?;
    if save && format != PlaylistFormat::M3u8 {
        return Err(bad("Save the playlist with an .m3u8 extension."));
    }
    Ok(uri)
}

fn format(uri: &VfsUri) -> Result<PlaylistFormat, FmError> {
    match uri
        .file_name()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("m3u") => Ok(PlaylistFormat::M3u),
        Some("m3u8") => Ok(PlaylistFormat::M3u8),
        Some("pls") => Ok(PlaylistFormat::Pls),
        _ => Err(bad("Choose an M3U, M3U8 or PLS playlist.")),
    }
}

fn load(uri: &VfsUri, vfs: &dyn Vfs) -> Result<Outcome, FmError> {
    let mut bytes = Vec::new();
    vfs.open_read(uri)?
        .take(MAX_PLAYLIST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PLAYLIST_BYTES {
        return Err(bad("The playlist exceeds the 2 MiB size limit."));
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| bad("The playlist must contain UTF-8 text."))?;
    let parent = uri
        .parent()
        .ok_or_else(|| bad("The playlist needs a parent folder."))?;
    let playlist = parse_playlist(text, format(uri)?, &parent)?;
    let mut items = Vec::new();
    let mut notices = vec![
        "Skipped an unsupported URL or credential-bearing playlist entry."
            .to_string();
        playlist.skipped
    ];
    for item in playlist.items {
        let reason = if validate_media(&item).is_err() {
            Some("unsupported media or remote path")
        } else {
            match vfs.stat(&item) {
                Ok(entry) if entry.is_file() => None,
                Ok(_) => Some("not a file"),
                Err(_) => Some("missing or unreadable"),
            }
        };
        if let Some(reason) = reason {
            notices.push(fm_vfs::redact::redact(&format!(
                "Skipped playlist item {}: {reason}.",
                item.file_name().unwrap_or("item")
            )));
        } else {
            items.push(item);
        }
    }
    Ok(Outcome::Loaded { items, notices })
}

fn save(uri: &VfsUri, items: &[VfsUri]) -> Result<Outcome, FmError> {
    let text = write_m3u8(items)?;
    let path = uri
        .local_path()
        .ok_or_else(|| bad("A local playlist path is required."))?;
    let parent = path
        .parent()
        .ok_or_else(|| bad("The playlist needs a parent folder."))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(&path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            bad("The playlist already exists. Choose a new filename to avoid overwriting it.")
        } else {
            FmError::Io(error.error)
        }
    })?;
    Ok(Outcome::Saved)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.playlist.load",
            label: "Load Playlist...",
            menu: &["Media", "Load playlist"],
            shortcut: None,
            params: "{uri: string}: append a local M3U/M3U8/PLS playlist without starting playback",
            enabled: |s| !s.playlist_job.is_busy(),
            run: |s, params| {
                let uri = target(params, false)?;
                if s.playlist_job.is_busy() {
                    return Err(bad("Wait for the current playlist job to finish."));
                }
                let vfs = s.backends.require(&uri)?;
                let (tx, receiver) = sync_channel(1);
                s.jobs.submit(JobLane::Local, move |_| {
                    let result = load(&uri, &*vfs)
                        .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
                    let _ = tx.send(result);
                    Ok(())
                });
                s.playlist_job.pending = Some(Pending {
                    receiver,
                    original: Some(s.media.queue.items().to_vec()),
                });
                Ok(())
            },
        },
        CommandSpec {
            id: "media.playlist.save",
            label: "Save Queue as Playlist...",
            menu: &["Media", "Save playlist"],
            shortcut: None,
            params: "{uri: string}: save the queue as a new local M3U8 file (never overwrite)",
            enabled: |s| !s.playlist_job.is_busy() && !s.media.queue.items().is_empty(),
            run: |s, params| {
                let uri = target(params, true)?;
                if s.playlist_job.is_busy() {
                    return Err(bad("Wait for the current playlist job to finish."));
                }
                if s.media.queue.items().is_empty() {
                    return Err(bad("The media queue is empty."));
                }
                let items = s.media.queue.items().to_vec();
                let (tx, receiver) = sync_channel(1);
                s.jobs.submit(JobLane::Local, move |_| {
                    let result = save(&uri, &items)
                        .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
                    let _ = tx.send(result);
                    Ok(())
                });
                s.playlist_job.pending = Some(Pending {
                    receiver,
                    original: None,
                });
                Ok(())
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
    use std::time::Duration;

    fn wait(s: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while s.playlist_job.is_busy() {
            s.poll();
            assert!(Instant::now() < deadline, "playlist job timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn fixture(text: &[u8]) -> Session {
        let fs = MemFs::new();
        fs.mkdirs_str("/music").expect("folder");
        fs.put_str("/music/one.mp3", b"one").expect("audio");
        fs.put_str("/music/two.flac", b"two").expect("audio");
        fs.put_str("/music/movie.mkv", b"video").expect("video");
        fs.put_str("/music/list.m3u8", text).expect("playlist");
        let mut s = Session::new().expect("session");
        s.backends.register(Arc::new(fs));
        s
    }

    #[test]
    fn load_skips_urls_missing_and_unsupported_without_starting_playback() {
        let mut s = fixture(
            b"#EXTM3U\none.mp3\nhttps://alice:secret@host/song.mp3\nfile:///music/two.flac\nsftp://alice:secret@host/song.mp3\nmissing.mp3\nnested.m3u8\nfolder\npassword=secret.mp3\ntwo.flac\nmovie.mkv\n"
        );
        s.dispatch("media.playlist.load", &json!({"uri": "/music/list.m3u8"}))
            .expect("load");
        assert!(
            s.media.queue.items().is_empty(),
            "results apply only on poll"
        );
        assert!(
            s.dispatch("media.playlist.load", &json!({"uri": "/music/list.m3u8"}))
                .is_err(),
            "one job at a time"
        );
        wait(&mut s);
        assert_eq!(
            s.media
                .queue
                .items()
                .iter()
                .filter_map(VfsUri::file_name)
                .collect::<Vec<_>>(),
            ["one.mp3", "two.flac", "movie.mkv"]
        );
        assert!(!s.media.playing);
        assert_eq!(s.media.queue.current_index(), None);
        let messages: Vec<_> = s
            .messages
            .recent()
            .map(|message| message.text.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("missing.mp3"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("nested.m3u8"))
        );
        assert!(
            messages
                .iter()
                .all(|message| !message.contains("secret") && !message.contains("alice"))
        );
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("skipped"))
        );
    }

    #[test]
    fn load_is_atomic_for_oversized_and_malformed_playlists_and_full_queues() {
        for bytes in [
            vec![b'a'; MAX_PLAYLIST_BYTES + 1],
            b"one.mp3\n"
                .iter()
                .copied()
                .chain(vec![b'a'; fm_media::MAX_PLAYLIST_LINE_BYTES + 1])
                .collect(),
            vec![0xff],
        ] {
            let mut s = fixture(&bytes);
            s.dispatch("media.playlist.load", &json!({"uri": "/music/list.m3u8"}))
                .expect("submit");
            wait(&mut s);
            assert!(s.media.queue.items().is_empty());
            assert!(s.notice.is_some(), "report invalid playlist");
        }
        let mut s = fixture(b"one.mp3\ntwo.flac\n");
        for _ in 0..MAX_QUEUE_ITEMS - 1 {
            s.media
                .queue
                .enqueue(VfsUri::parse("/music/one.mp3").expect("uri"))
                .expect("enqueue");
        }
        s.dispatch("media.playlist.load", &json!({"uri": "/music/list.m3u8"}))
            .expect("load");
        wait(&mut s);
        assert_eq!(s.media.queue.items().len(), MAX_QUEUE_ITEMS - 1);
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("does not fit"))
        );
    }

    #[test]
    fn queue_edits_discard_delayed_import() {
        let mut s = fixture(b"one.mp3\n");
        s.dispatch("media.playlist.load", &json!({"uri": "/music/list.m3u8"}))
            .expect("submit");
        s.media
            .queue
            .enqueue(VfsUri::parse("/music/two.flac").expect("uri"))
            .expect("edit");
        wait(&mut s);
        assert_eq!(s.media.queue.items().len(), 1);
        assert_eq!(s.media.queue.items()[0].file_name(), Some("two.flac"));
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("queue changed"))
        );
    }

    #[test]
    fn save_load_roundtrip_is_ordered_owner_only_and_does_not_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut s = Session::new().expect("session");
        for name in ["two.mp3", "one.flac"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"fixture").expect("file");
            s.media
                .queue
                .enqueue(VfsUri::parse(path.to_str().expect("utf8")).expect("uri"))
                .expect("enqueue");
        }
        let original = s.media.queue.items().to_vec();
        let path = dir.path().join("list.m3u8");
        let params = json!({"uri": path.to_str().expect("utf8")});
        s.dispatch("media.playlist.save", &params).expect("save");
        wait(&mut s);
        assert!(
            s.info
                .as_ref()
                .is_some_and(|(text, _)| text == "Saved playlist")
        );
        let bytes = std::fs::read(&path).expect("saved");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        s.dispatch("media.clear", &Params::Null).expect("clear");
        s.dispatch("media.playlist.load", &params).expect("load");
        wait(&mut s);
        assert_eq!(s.media.queue.items(), original);
        s.dispatch("media.playlist.save", &params)
            .expect("submit existing");
        wait(&mut s);
        assert!(
            s.notice
                .as_deref()
                .is_some_and(|notice| notice.contains("already exists"))
        );
        assert_eq!(std::fs::read(&path).expect("read"), bytes);
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("dir").count(),
            3,
            "no leaked temporary files"
        );
    }

    #[test]
    fn malformed_parameters_never_start_jobs_or_mutate_the_queue() {
        let mut s = fixture(b"one.mp3");
        for id in ["media.playlist.load", "media.playlist.save"] {
            for params in [
                Params::Null,
                json!("path"),
                json!({}),
                json!({"uri": 4}),
                json!({"uri": "https://alice:secret@host/list.m3u8"}),
                json!({"uri": "sftp://alice@host/list.m3u8"}),
                json!({"uri": "/music/list.exe"}),
                json!({"uri": "/music/list.m3u8", "overwrite": true}),
            ] {
                let error = s.dispatch(id, &params).expect_err("bad params");
                assert!(!error.to_string().contains("secret"));
                assert!(!s.playlist_job.is_busy());
                assert!(s.media.queue.items().is_empty());
            }
        }
    }
}
