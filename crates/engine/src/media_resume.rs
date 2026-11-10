//! Remember where a folder of media (an audiobook) was left off.

use fm_vfs::{FmError, VfsUri};
use serde::{Deserialize, Serialize};

use crate::media_cmds::MediaSession;
use crate::registry::CommandSpec;

/// Folders remembered; the oldest is dropped first.
pub const MAX_RESUME: usize = 500;
const REWIND_SECS: f64 = 3.0;
const MIN_SECS: f64 = 1.0;
const SAVE_STEP_SECS: f64 = 5.0;
const MAX_TEXT: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumePoint {
    pub dir: String,
    pub file: String,
    pub position: f64,
}

impl ResumePoint {
    pub(crate) fn is_valid(&self) -> bool {
        self.position.is_finite()
            && self.position >= 0.0
            && !self.dir.is_empty()
            && !self.file.is_empty()
            && self.dir.len() <= MAX_TEXT
            && self.file.len() <= MAX_TEXT
    }
}

impl MediaSession {
    /// Record the playing item of an automatic folder (an audiobook); other items are not remembered.
    pub(crate) fn note_resume(&mut self) {
        if self.position < MIN_SECS {
            return;
        }
        let Some(index) = self.queue.current_index() else {
            return;
        };
        let (Some(file), Some(group)) = (
            self.queue.items().get(index).and_then(VfsUri::file_name),
            self.queue.group_of(index),
        ) else {
            return;
        };
        let Some(dir) = group.dir else {
            return;
        };
        let point = ResumePoint {
            dir: dir.to_string(),
            file: file.to_string(),
            position: self.position,
        };
        if !point.is_valid() {
            return;
        }
        match self.resume.iter().position(|p| p.dir == point.dir) {
            Some(at) if at + 1 == self.resume.len() => {
                if let Some(last) = self.resume.last_mut() {
                    *last = point;
                }
            }
            Some(at) => {
                self.resume.remove(at);
                self.resume.push(point);
            }
            None => {
                if self.resume.len() >= MAX_RESUME {
                    self.resume.remove(0);
                }
                self.resume.push(point);
            }
        }
    }

    /// An item finished: the last file of a folder (or a lone book or video) restarts next time.
    pub(crate) fn forget_finished_book(&mut self) {
        let Some(index) = self.queue.current_index() else {
            return;
        };
        let book = self.book_ref(index);
        if let Some(group) = self.queue.group_of(index)
            && let Some(dir) = group.dir
        {
            if group.last {
                let dir = dir.to_string();
                if book.is_some_and(|b| b.folder.is_some()) {
                    self.finish_folder(&dir);
                } else {
                    self.resume.retain(|p| p.dir != dir);
                }
            }
        } else if let Some(book) = book {
            let at = self.touch_state(&book.key, true);
            if let Some(state) = self.books.items.get_mut(at) {
                state.finished = true;
                state.position = None;
            }
        }
    }

    /// Item offset within `members` and position to continue the folder `dir`.
    pub fn resume_in(&self, dir: &str, members: &[VfsUri]) -> Option<(usize, f64)> {
        let point = self.resume.iter().find(|p| p.dir == dir)?;
        let offset = members
            .iter()
            .position(|uri| uri.file_name() == Some(point.file.as_str()))?;
        Some((offset, (point.position - REWIND_SECS).max(0.0)))
    }

    /// Positions are rounded down so saving does not follow every playback tick.
    pub(crate) fn resume_snapshot(&self) -> Vec<ResumePoint> {
        self.resume
            .iter()
            .map(|p| ResumePoint {
                position: (p.position / SAVE_STEP_SECS).floor() * SAVE_STEP_SECS,
                ..p.clone()
            })
            .collect()
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "media.continue",
        label: "Resume Play",
        menu: &["Media", "Resume play"],
        shortcut: None,
        params: "{index: number}: play the saved file and position of the queue folder group containing queue row `index`",
        enabled: |s| !s.media.queue.items().is_empty(),
        run: |s, params| {
            let bad = |m: &str| FmError::InvalidParams(m.into());
            let map = crate::file_cmds::object(params)?.ok_or_else(|| bad("index is required"))?;
            if map.keys().any(|key| key != "index") {
                return Err(bad("only index is accepted"));
            }
            let index = map
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| bad("index must be a queue row"))?;
            let group = s
                .media
                .queue
                .groups()
                .into_iter()
                .find(|g| index >= g.start && index - g.start < g.len)
                .ok_or_else(|| bad("That row is not in a folder"))?;
            let dir = group
                .dir
                .as_deref()
                .ok_or_else(|| bad("Only folders added from disk can be continued"))?;
            let members = s
                .media
                .queue
                .items()
                .get(group.start..group.start + group.len)
                .ok_or_else(|| bad("That row is not in a folder"))?;
            let (offset, position) = s
                .media
                .resume_in(dir, members)
                .ok_or_else(|| bad("Nothing to continue for this folder yet"))?;
            s.media_start_at(group.start + offset, position)
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uris(paths: &[&str]) -> Vec<VfsUri> {
        paths
            .iter()
            .map(|p| VfsUri::parse(p).expect("uri"))
            .collect()
    }

    fn session_with(paths: &[&str], current: usize) -> MediaSession {
        let mut media = MediaSession::default();
        for uri in uris(paths) {
            media.queue.enqueue(uri).expect("enqueue");
        }
        media
            .queue
            .group_range(0, media.queue.items().len(), "book", Some("/book".into()))
            .expect("group");
        media.queue.select(current).expect("select");
        media
    }

    #[test]
    fn a_folder_remembers_file_and_position_and_resumes_with_a_rewind() {
        let mut media = session_with(&["/book/01.mp3", "/book/02.mp3", "/book/03.mp3"], 1);
        media.position = 100.0;
        media.note_resume();
        media.position = 120.0;
        media.note_resume();
        assert_eq!(media.resume.len(), 1);
        let added = uris(&["/book/01.mp3", "/book/02.mp3", "/book/03.mp3"]);
        assert_eq!(media.resume_in("/book", &added), Some((1, 117.0)));
        assert_eq!(media.resume_in("/other", &added), None);
        assert_eq!(media.resume_snapshot()[0].position, 120.0);
    }

    #[test]
    fn ungrouped_and_user_folders_are_not_remembered_and_a_finished_book_restarts() {
        let mut media = MediaSession::default();
        for uri in uris(&["/a/one.mp3", "/b/two.mp3"]) {
            media.queue.enqueue(uri).expect("enqueue");
        }
        media.queue.group_range(1, 1, "mine", None).expect("group");
        for at in 0..2 {
            media.queue.select(at).expect("select");
            media.position = 50.0;
            media.note_resume();
        }
        assert!(media.resume.is_empty());

        let mut book = session_with(&["/book/01.mp3", "/book/02.mp3"], 1);
        book.position = 50.0;
        book.note_resume();
        assert_eq!(book.resume.len(), 1);
        book.forget_finished_book();
        assert!(book.resume.is_empty());
    }

    #[test]
    fn continue_command_resumes_the_folder_and_rejects_bad_params() {
        let mut s = crate::Session::new().expect("session");
        for uri in uris(&["/x/a.mp3", "/book/01.mp3", "/book/02.mp3"]) {
            s.media.queue.enqueue(uri).expect("enqueue");
        }
        s.media
            .queue
            .group_range(1, 2, "book", Some("/book".into()))
            .expect("group");
        s.media.resume.push(ResumePoint {
            dir: "/book".into(),
            file: "02.mp3".into(),
            position: 40.0,
        });
        for bad in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"index": "1"}),
            serde_json::json!({"index": 0}),
            serde_json::json!({"index": 99}),
            serde_json::json!({"index": 1, "x": 1}),
        ] {
            assert!(s.dispatch("media.continue", &bad).is_err(), "{bad}");
        }
        s.dispatch("media.continue", &serde_json::json!({"index": 1}))
            .expect("continue");
        assert_eq!(s.media.queue.current_index(), Some(2));
        assert!((s.media.position - 37.0).abs() < 1.0);
        s.media_stop();
    }

    #[test]
    fn the_oldest_folders_are_dropped_past_the_cap() {
        let mut media = session_with(&["/book/01.mp3", "/book/02.mp3"], 0);
        media.resume = (0..MAX_RESUME)
            .map(|n| ResumePoint {
                dir: format!("/old{n}"),
                file: "x.mp3".into(),
                position: 1.0,
            })
            .collect();
        media.position = 10.0;
        media.note_resume();
        assert_eq!(media.resume.len(), MAX_RESUME);
        assert_eq!(media.resume.last().map(|p| p.dir.as_str()), Some("/book"));
        assert_eq!(media.resume.first().map(|p| p.dir.as_str()), Some("/old1"));
    }
}
