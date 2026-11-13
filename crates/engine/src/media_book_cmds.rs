//! Audiobook commands: skip, chapters, marking books, bookmarks, finished state.

use fm_media::MAX_CHAPTERS;
use fm_vfs::FmError;
use serde_json::Value;

use crate::Session;
use crate::file_cmds::{Obj, object};
use crate::media_book::{Bookmark, MAX_BOOKMARKS, MAX_MARKED, MAX_NOTE};
use crate::registry::CommandSpec;

const MAX_SKIP_SECS: f64 = 86_400.0;

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn only<'a>(params: &'a Value, keys: &[&str]) -> Result<Option<&'a Obj>, FmError> {
    let map = object(params)?;
    if let Some(map) = map
        && let Some(key) = map.keys().find(|key| !keys.contains(&key.as_str()))
    {
        return Err(bad(&format!("unknown parameter {key}")));
    }
    Ok(map)
}

fn row(map: Option<&Obj>, key: &str) -> Result<Option<usize>, FmError> {
    map.and_then(|m| m.get(key))
        .map(|value| {
            value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| bad(&format!("{key} must be a non-negative number")))
        })
        .transpose()
}

fn flag(map: Option<&Obj>, key: &str) -> Result<Option<bool>, FmError> {
    map.and_then(|m| m.get(key))
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| bad(&format!("{key} must be true or false")))
        })
        .transpose()
}

impl Session {
    /// Seek inside the current file, clamped to it.
    pub(crate) fn media_seek_file(&mut self, to: f64) -> Result<(), FmError> {
        let to = to.max(0.0).min(self.media.duration().unwrap_or(f64::MAX));
        if let Some(player) = &self.media.player {
            player.seek(to)?;
        }
        if let Some(video) = &mut self.media.video {
            video.seek(to);
        }
        self.media.position = to;
        Ok(())
    }

    /// Go to `to` seconds into the book, crossing file boundaries of a folder book.
    pub(crate) fn media_goto_book(&mut self, to: f64) -> Result<(), FmError> {
        let index = self
            .media
            .queue
            .current_index()
            .ok_or_else(|| bad("no current media item"))?;
        let range = self.media.book_ref(index).and_then(|book| book.folder);
        let Some((range, timeline)) = range.and_then(|range| {
            let timeline = self.media.book_timeline(&range)?;
            Some((range, timeline))
        }) else {
            return self.media_seek_file(to);
        };
        let (file, offset) = timeline.locate(to);
        let target = range.start + file;
        if target == index {
            return self.media_seek_file(offset);
        }
        let playing = self.media.playing;
        self.media.queue.select(target)?;
        self.media.reset_item();
        self.media.failed_items = 0;
        self.media.position = offset;
        if playing { self.media_resume() } else { Ok(()) }
    }
}

/// Book position and length of the current item, or of its file when it is no book.
fn here(s: &Session) -> (f64, f64) {
    let file_total = s.media.duration().unwrap_or(f64::MAX);
    match s.media.book_view() {
        Some(view) => (view.position, view.total.unwrap_or(file_total)),
        None => (s.media.position, file_total),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.skip",
            label: "Skip",
            menu: &["Media", "Skip"],
            shortcut: None,
            params: "{by: number} seconds (negative goes back), or {direction: \"back\"|\"forward\"} using the preferred skip lengths; never leaves the book",
            enabled: |s| s.media.queue.current().is_some(),
            run: |s, params| {
                let map = only(params, &["by", "direction"])?;
                let by = map.and_then(|m| m.get("by"));
                let direction = map.and_then(|m| m.get("direction"));
                let by = match (by, direction) {
                    (Some(by), None) => by
                        .as_f64()
                        .filter(|v| v.is_finite() && *v != 0.0 && v.abs() <= MAX_SKIP_SECS)
                        .ok_or_else(|| {
                            bad("by must be a non-zero number of seconds up to a day")
                        })?,
                    (None, Some(direction)) => match direction.as_str() {
                        Some("back") => -(s.prefs.media_skip_back_secs as f64),
                        Some("forward") => s.prefs.media_skip_forward_secs as f64,
                        _ => return Err(bad("direction must be back or forward")),
                    },
                    _ => return Err(bad("supply exactly one of by or direction")),
                };
                if s.media.queue.current().is_none() {
                    return Err(bad("no current media item"));
                }
                let (position, total) = here(s);
                s.media_goto_book(fm_media::skip_target(position, by, total))
            },
        },
        CommandSpec {
            id: "media.chapter",
            label: "Chapter",
            menu: &["Media", "Chapter"],
            shortcut: None,
            params: "{delta: 1|-1} for next or previous chapter, or {index: number} to jump to a chapter of the current book",
            enabled: |s| s.media.book_view().is_some_and(|v| !v.chapters.is_empty()),
            run: |s, params| {
                let map = only(params, &["delta", "index"])?;
                let delta = map.and_then(|m| m.get("delta"));
                let index = row(map, "index")?;
                let view = s
                    .media
                    .book_view()
                    .filter(|view| !view.chapters.is_empty())
                    .ok_or_else(|| bad("the current item has no chapters"))?;
                let target = match (delta, index) {
                    (Some(delta), None) => match delta.as_i64() {
                        Some(1) => fm_media::next_chapter(&view.chapters, view.position)
                            .ok_or_else(|| bad("already in the last chapter"))?,
                        Some(-1) => fm_media::previous_chapter(&view.chapters, view.position)
                            .ok_or_else(|| bad("no previous chapter"))?,
                        _ => return Err(bad("delta must be 1 or -1")),
                    },
                    (None, Some(index)) if index < MAX_CHAPTERS => index,
                    _ => return Err(bad("supply exactly one of delta or index")),
                };
                let chapter = view
                    .chapters
                    .get(target)
                    .ok_or_else(|| bad("that chapter does not exist"))?;
                s.media_goto_book(chapter.start)
            },
        },
        CommandSpec {
            id: "media.book",
            label: "Mark as Audiobook",
            menu: &["Media", "Mark as audiobook"],
            shortcut: None,
            params: "{index?: number, on?: bool}: treat the queue row's folder (or the file, outside a folder) as an audiobook; toggles when `on` is omitted. Defaults to the current item",
            enabled: |s| !s.media.queue.items().is_empty(),
            run: |s, params| {
                let map = only(params, &["index", "on"])?;
                let index = row(map, "index")?
                    .or_else(|| s.media.queue.current_index())
                    .ok_or_else(|| bad("no media item"))?;
                let on = flag(map, "on")?;
                let uri = s
                    .media
                    .queue
                    .items()
                    .get(index)
                    .ok_or_else(|| bad("media queue index out of range"))?;
                let key = match s.media.queue.group_of(index).and_then(|g| g.dir) {
                    Some(dir) => dir.to_string(),
                    None if uri.local_path().is_some() => uri.path_str().to_string(),
                    None => return Err(bad("only local items can be marked as audiobooks")),
                };
                let was = s.media.is_marked(&key);
                let on = on.unwrap_or(!was);
                if on && !was {
                    if s.media.books.marked.len() >= MAX_MARKED {
                        s.media.books.marked.remove(0);
                    }
                    s.media.books.marked.push(key);
                } else if !on {
                    s.media.books.marked.retain(|marked| *marked != key);
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.bookmark",
            label: "Bookmark",
            menu: &["Media", "Bookmark"],
            shortcut: None,
            params: "{action: \"add\", note?: string} saves the current position of an audiobook or video; {action: \"remove\"|\"jump\", index: number} acts on the saved bookmark (at most 100 per item)",
            enabled: |s| {
                s.media
                    .queue
                    .current_index()
                    .is_some_and(|i| s.media.book_ref(i).is_some())
            },
            run: |s, params| {
                let map = only(params, &["action", "index", "note"])?;
                let action = map
                    .and_then(|m| m.get("action"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| bad("action must be add, remove or jump"))?;
                let index = s
                    .media
                    .queue
                    .current_index()
                    .ok_or_else(|| bad("no current media item"))?;
                let book = s
                    .media
                    .book_ref(index)
                    .ok_or_else(|| bad("bookmarks are for audiobooks and videos"))?;
                let view = s
                    .media
                    .book_view()
                    .ok_or_else(|| bad("bookmarks are for audiobooks and videos"))?;
                let slot = row(map, "index")?;
                match action {
                    "add" => {
                        if slot.is_some() {
                            return Err(bad("index is not used when adding"));
                        }
                        if book.folder.is_some() && view.total.is_none() {
                            return Err(bad("the book is still being measured; try again shortly"));
                        }
                        let note = match map.and_then(|m| m.get("note")) {
                            None | Some(Value::Null) => String::new(),
                            Some(Value::String(note)) => note.trim().to_string(),
                            Some(_) => return Err(bad("note must be text")),
                        };
                        let mark = Bookmark {
                            position: view.position,
                            note,
                        };
                        if !mark.is_valid() {
                            return Err(bad(&format!(
                                "a note is at most {MAX_NOTE} characters without control characters"
                            )));
                        }
                        if view.bookmarks.len() >= MAX_BOOKMARKS {
                            return Err(bad("this item already has 100 bookmarks"));
                        }
                        let at = s.media.touch_state(&book.key, book.folder.is_none());
                        if let Some(state) = s.media.books.items.get_mut(at) {
                            state.bookmarks.push(mark);
                            state
                                .bookmarks
                                .sort_by(|a, b| a.position.total_cmp(&b.position));
                        }
                        Ok(())
                    }
                    "remove" | "jump" => {
                        let slot = slot.ok_or_else(|| bad("index is required"))?;
                        let mark = view
                            .bookmarks
                            .get(slot)
                            .ok_or_else(|| bad("that bookmark does not exist"))?
                            .clone();
                        if action == "jump" {
                            return s.media_goto_book(mark.position);
                        }
                        let at = s.media.touch_state(&book.key, book.folder.is_none());
                        if let Some(state) = s.media.books.items.get_mut(at) {
                            state.bookmarks.remove(slot);
                        }
                        Ok(())
                    }
                    _ => Err(bad("action must be add, remove or jump")),
                }
            },
        },
        CommandSpec {
            id: "media.finished",
            label: "Mark Finished",
            menu: &["Media", "Mark finished"],
            shortcut: None,
            params: "{index?: number, on?: bool}: mark the book of a queue row finished (it restarts from the top next time) or unfinished; toggles when `on` is omitted. Defaults to the current item",
            enabled: |s| {
                s.media
                    .queue
                    .current_index()
                    .is_some_and(|i| s.media.book_ref(i).is_some())
            },
            run: |s, params| {
                let map = only(params, &["index", "on"])?;
                let index = row(map, "index")?
                    .or_else(|| s.media.queue.current_index())
                    .ok_or_else(|| bad("no media item"))?;
                let book = s
                    .media
                    .book_ref(index)
                    .ok_or_else(|| bad("only audiobooks and videos have a finished state"))?;
                let was = s.media.state_of(&book.key).is_some_and(|st| st.finished);
                let on = flag(map, "on")?.unwrap_or(!was);
                let current_book = s
                    .media
                    .queue
                    .current_index()
                    .and_then(|i| s.media.book_ref(i))
                    .is_some_and(|b| b.key == book.key);
                if on {
                    if book.folder.is_some() {
                        s.media.finish_folder(&book.key);
                    } else {
                        let at = s.media.touch_state(&book.key, true);
                        if let Some(state) = s.media.books.items.get_mut(at) {
                            state.finished = true;
                            state.position = None;
                        }
                    }
                    if current_book {
                        s.media_stop();
                    }
                } else {
                    let at = s.media.touch_state(&book.key, book.folder.is_none());
                    if let Some(state) = s.media.books.items.get_mut(at) {
                        state.finished = false;
                    }
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.resume_clear",
            label: "Forget Remembered Positions",
            menu: &["Media", "Forget remembered positions"],
            shortcut: None,
            params: "none: forget every saved position, bookmark, finished mark and folder resume point",
            enabled: |s| !s.media.books.items.is_empty() || !s.media.resume.is_empty(),
            run: |s, params| {
                if !params.is_null() && object(params)?.is_some_and(|m| !m.is_empty()) {
                    return Err(bad("this command takes no parameters"));
                }
                s.media.books.items.clear();
                s.media.resume.clear();
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fm_media::{AudioFormat, AudioInfo};
    use fm_vfs::VfsUri;
    use serde_json::json;

    use crate::Session;
    use crate::media_book::MAX_ITEMS;

    fn info(duration: f64) -> Arc<AudioInfo> {
        Arc::new(AudioInfo {
            encoder_delay: 0,
            encoder_padding: 0,
            format: AudioFormat::new(48_000, 2).expect("format"),
            duration: Some(duration),
            tags: Default::default(),
        })
    }

    fn uri(path: &str) -> VfsUri {
        VfsUri::parse(path).expect("uri")
    }

    /// A folder book of three files, 100 s, 50 s and 25 s, with file `current` selected.
    fn folder_book(current: usize) -> Session {
        let mut s = Session::new().expect("session");
        for name in ["01.m4b", "02.m4b", "03.m4b"] {
            s.media
                .queue
                .enqueue(uri(&format!("/book/{name}")))
                .expect("enqueue");
        }
        s.media
            .queue
            .group_range(0, 3, "book", Some("/book".into()))
            .expect("group");
        s.media.queue.select(current).expect("select");
        for (name, secs) in [("01", 100.0), ("02", 50.0), ("03", 25.0)] {
            s.media
                .books
                .set_duration_for_tests(&format!("/book/{name}.m4b"), secs);
        }
        s.media.info = Some(info([100.0, 50.0, 25.0][current]));
        s
    }

    #[test]
    fn seeking_and_skipping_cross_file_boundaries() {
        let mut s = folder_book(0);
        s.media.position = 90.0;
        let view = s.media.book_view().expect("view");
        assert_eq!(view.total, Some(175.0));
        assert_eq!(view.position, 90.0);
        s.dispatch("media.skip", &json!({"by": 30})).expect("skip");
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert_eq!(s.media.position, 20.0);
        assert_eq!(s.media.book_view().expect("view").position, 120.0);
        s.dispatch("media.skip", &json!({"by": -500}))
            .expect("clamped");
        assert_eq!(s.media.queue.current_index(), Some(0));
        assert_eq!(s.media.position, 0.0);
        s.dispatch("media.skip", &json!({"by": 15})).expect("skip");
        s.dispatch("media.skip", &json!({"direction": "back"}))
            .expect("back");
        assert_eq!(s.media.position, 0.0);
    }

    #[test]
    fn chapters_navigate_per_file_or_from_the_embedded_list() {
        let mut s = folder_book(1);
        s.media.position = 10.0;
        let view = s.media.book_view().expect("view");
        assert_eq!(view.chapters.len(), 3);
        assert_eq!(view.chapter, Some(1));
        s.dispatch("media.chapter", &json!({"delta": 1}))
            .expect("next");
        assert_eq!(s.media.queue.current_index(), Some(2));
        s.dispatch("media.chapter", &json!({"index": 0}))
            .expect("first");
        assert_eq!(s.media.queue.current_index(), Some(0));

        let mut s = folder_book(0);
        s.media.books.set_chapters_for_tests(
            s.media.queue.current().cloned(),
            &[("A", 0.0, 40.0), ("B", 40.0, 100.0)],
        );
        s.media.position = 10.0;
        s.dispatch("media.chapter", &json!({"delta": 1}))
            .expect("next");
        assert_eq!(s.media.position, 40.0);
        s.dispatch("media.chapter", &json!({"delta": -1}))
            .expect("previous");
        assert_eq!(s.media.position, 0.0);
        s.media.position = 60.0;
        s.dispatch("media.chapter", &json!({"delta": -1}))
            .expect("restart");
        assert_eq!(s.media.position, 40.0);
    }

    #[test]
    fn bookmarks_add_sort_jump_remove_and_cap() {
        let mut s = folder_book(1);
        s.media.position = 30.0;
        s.dispatch(
            "media.bookmark",
            &json!({"action": "add", "note": " Here "}),
        )
        .expect("add");
        s.media.position = 5.0;
        s.dispatch("media.bookmark", &json!({"action": "add"}))
            .expect("add");
        let view = s.media.book_view().expect("view");
        assert_eq!(view.bookmarks[0].position, 105.0);
        assert_eq!(view.bookmarks[1].position, 130.0);
        assert_eq!(view.bookmarks[1].note, "Here");
        s.dispatch("media.bookmark", &json!({"action": "jump", "index": 0}))
            .expect("jump");
        assert_eq!(s.media.position, 5.0);
        s.dispatch("media.bookmark", &json!({"action": "remove", "index": 0}))
            .expect("remove");
        assert_eq!(s.media.book_view().expect("view").bookmarks.len(), 1);
        for n in 0..120 {
            s.media.position = f64::from(n % 40);
            let _ = s.dispatch("media.bookmark", &json!({"action": "add"}));
        }
        assert_eq!(s.media.book_view().expect("view").bookmarks.len(), 100);
    }

    #[test]
    fn a_single_book_is_remembered_resumed_finished_and_dropped_when_replaced() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("story.m4b");
        std::fs::write(&path, b"abcd").expect("file");
        let mut s = Session::new().expect("session");
        s.media
            .queue
            .enqueue(uri(&path.to_string_lossy()))
            .expect("enqueue");
        s.media.queue.select(0).expect("select");
        s.media.info = Some(info(1000.0));
        s.media.position = 400.0;
        s.media.note_book();
        assert_eq!(s.media.remembered_position(0), Some(397.0));
        s.media.position = 995.0;
        s.media.note_book();
        assert_eq!(s.media.remembered_position(0), None, "finished restarts");
        s.media.position = 100.0;
        s.media.note_book();
        assert_eq!(s.media.remembered_position(0), Some(97.0));
        std::fs::write(&path, b"a longer replacement").expect("replace");
        assert_eq!(s.media.remembered_position(0), None, "replaced file");
    }

    #[test]
    fn marking_finished_restarts_and_music_is_never_remembered() {
        let mut s = folder_book(1);
        s.media.position = 20.0;
        s.media.note_book();
        s.dispatch("media.finished", &json!({"on": true}))
            .expect("finished");
        assert!(s.media.book_view().expect("view").finished);
        assert!(s.media.resume.is_empty());
        s.dispatch("media.finished", &json!({"on": false}))
            .expect("unfinished");
        assert!(!s.media.book_view().expect("view").finished);

        let mut music = Session::new().expect("session");
        music
            .media
            .queue
            .enqueue(uri("/m/song.mp3"))
            .expect("enqueue");
        music.media.queue.select(0).expect("select");
        music.media.info = Some(info(200.0));
        music.media.position = 50.0;
        music.media.note_book();
        assert!(music.media.books.items.is_empty());
        assert!(
            music
                .dispatch("media.bookmark", &json!({"action": "add"}))
                .is_err()
        );
        music
            .dispatch("media.book", &json!({"on": true}))
            .expect("mark");
        assert!(music.media.book_view().is_some());
        music
            .dispatch("media.book", &json!({"on": false}))
            .expect("unmark");
        assert!(music.media.book_view().is_none());
    }

    #[test]
    fn mp3_folders_are_books_under_an_audiobook_shelf_or_with_chapter_names() {
        let mut s = Session::new().expect("session");
        for name in [
            "/Audio Books/Ghost/01 Intro.mp3",
            "/Audio Books/Ghost/02 Next.mp3",
        ] {
            s.media.queue.enqueue(uri(name)).expect("enqueue");
        }
        s.media
            .queue
            .group_range(0, 2, "Ghost", Some("/Audio Books/Ghost".into()))
            .expect("group");
        assert!(s.media.book_ref(0).is_some());
        for name in [
            "/Music/Album/Chapter 1.mp3",
            "/Music/Album/Chapter 2.mp3",
            "/Music/Mix/a.mp3",
            "/Music/Mix/b.mp3",
        ] {
            s.media.queue.enqueue(uri(name)).expect("enqueue");
        }
        s.media
            .queue
            .group_range(2, 2, "Album", Some("/Music/Album".into()))
            .expect("group");
        s.media
            .queue
            .group_range(4, 2, "Mix", Some("/Music/Mix".into()))
            .expect("group");
        assert!(s.media.book_ref(2).is_some(), "chapter names");
        assert!(s.media.book_ref(4).is_none(), "plain music");
    }

    #[test]
    fn the_context_menu_offers_only_what_applies() {
        let mut s = Session::new().expect("session");
        s.media.queue.enqueue(uri("/m/song.mp3")).expect("enqueue");
        let menu = s.media.book_menu(0);
        assert!(!menu.is_book && !menu.marked && !menu.finished);
        s.dispatch("media.book", &json!({"index": 0, "on": true}))
            .expect("mark");
        let menu = s.media.book_menu(0);
        assert!(menu.is_book && menu.marked && !menu.finished);
        s.dispatch("media.finished", &json!({"index": 0, "on": true}))
            .expect("finish");
        assert!(s.media.book_menu(0).finished);
        s.media.queue.enqueue(uri("/m/story.m4b")).expect("enqueue");
        let menu = s.media.book_menu(1);
        assert!(
            menu.is_book && !menu.marked,
            "nothing to unmark on a real audiobook"
        );
    }

    #[test]
    fn rows_show_resume_and_finished_marks() {
        use crate::media_book::RowMark;
        let mut s = folder_book(1);
        assert_eq!(s.media.row_mark(1), None);
        s.media.position = 20.0;
        s.media.note_book();
        assert_eq!(s.media.row_mark(1), Some(RowMark::Resume(20.0)));
        assert_eq!(s.media.row_mark(0), Some(RowMark::Played));
        assert_eq!(s.media.row_mark(2), None);
        s.dispatch("media.finished", &json!({"on": true}))
            .expect("finished");
        assert_eq!(s.media.row_mark(0), Some(RowMark::Finished));
        assert_eq!(s.media.row_mark(2), Some(RowMark::Finished));
    }

    #[test]
    fn videos_in_a_folder_are_remembered_one_by_one_not_as_a_book() {
        use crate::media_book::RowMark;
        let mut s = Session::new().expect("session");
        let dir = "/Videos/The Life Collection";
        for name in ["01 Challenges.mkv", "02 Reptiles.mkv", "03 Insects.mkv"] {
            s.media
                .queue
                .enqueue(uri(&format!("{dir}/{name}")))
                .expect("enqueue");
        }
        s.media
            .queue
            .group_range(0, 3, "The Life", Some(dir.into()))
            .expect("folder");
        s.media.queue.select(1).expect("episode 2");
        s.media
            .books
            .set_duration_for_tests(&format!("{dir}/02 Reptiles.mkv"), 3_000.0);
        assert!(
            s.media
                .book_ref(1)
                .is_some_and(|b| b.video && b.folder.is_none()),
            "a video is never a folder book"
        );
        s.media.position = 1_200.0;
        s.media.note_book();
        assert_eq!(
            s.media.row_mark(0),
            None,
            "episode 1 is not marked as played"
        );
        assert_eq!(s.media.row_mark(1), Some(RowMark::Resume(1_200.0)));
        assert_eq!(s.media.row_mark(2), None);
        assert!(s.media.remembered_position(1).is_some_and(|p| p > 1_100.0));
    }

    #[test]
    fn the_least_recently_played_item_is_dropped_past_the_cap() {
        let mut s = Session::new().expect("session");
        for n in 0..=MAX_ITEMS {
            s.media.touch_state(&format!("/b/{n}.m4b"), false);
        }
        assert_eq!(s.media.books.items.len(), MAX_ITEMS);
        assert_eq!(s.media.books.items[0].key, "/b/1.m4b");
        s.media.touch_state("/b/1.m4b", false);
        assert_eq!(
            s.media.books.items.last().map(|i| i.key.as_str()),
            Some("/b/1.m4b")
        );
    }

    #[test]
    fn bad_params_are_rejected_and_change_nothing() {
        let mut s = folder_book(1);
        s.media.position = 10.0;
        for (id, params) in [
            ("media.skip", json!({})),
            ("media.skip", json!({"by": 0})),
            ("media.skip", json!({"by": 1e9})),
            ("media.skip", json!({"by": "5"})),
            ("media.skip", json!({"by": 5, "direction": "back"})),
            ("media.skip", json!({"direction": "sideways"})),
            ("media.skip", json!({"x": 1})),
            ("media.chapter", json!({})),
            ("media.chapter", json!({"delta": 2})),
            ("media.chapter", json!({"index": 99})),
            ("media.chapter", json!({"delta": 1, "index": 0})),
            ("media.book", json!({"index": 99})),
            ("media.book", json!({"on": "yes"})),
            ("media.bookmark", json!({})),
            ("media.bookmark", json!({"action": "explode"})),
            ("media.bookmark", json!({"action": "add", "note": 5})),
            ("media.bookmark", json!({"action": "add", "note": "a\u{7}"})),
            ("media.bookmark", json!({"action": "add", "index": 1})),
            ("media.bookmark", json!({"action": "jump"})),
            ("media.bookmark", json!({"action": "jump", "index": 7})),
            ("media.finished", json!({"index": 99})),
            ("media.finished", json!({"on": 1})),
            ("media.resume_clear", json!({"x": 1})),
        ] {
            let before = (s.media.position, s.media.queue.current_index());
            assert!(s.dispatch(id, &params).is_err(), "{id} {params}");
            assert_eq!((s.media.position, s.media.queue.current_index()), before);
        }
        assert!(s.media.book_view().expect("view").bookmarks.is_empty());
        s.media.books.marked.clear();
    }
}
