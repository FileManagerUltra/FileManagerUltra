//! Audiobooks: which items are books, their chapters and timeline, remembered positions,
//! bookmarks and finished state. Paths and positions only; the file is owner-only.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::UNIX_EPOCH;

use fm_media::{AudioInfo, Chapter, Timeline};
use fm_vfs::VfsUri;
use serde::{Deserialize, Serialize};

use crate::Session;
use crate::jobs::JobLane;
use crate::media_cmds::{MediaSession, is_video};

/// Remembered items (books and videos); the least recently played is dropped first.
pub const MAX_ITEMS: usize = 500;
pub const MAX_BOOKMARKS: usize = 100;
pub const MAX_NOTE: usize = 200;
/// Items and folders the user marked as audiobooks.
pub const MAX_MARKED: usize = 500;
const MAX_KEY: usize = 4096;
const MAX_DURATIONS: usize = 4096;
/// Files measured per folder; larger folders show no total.
const MAX_BOOK_FILES: usize = 300;
const REWIND_SECS: f64 = 3.0;
const SAVE_STEP_SECS: f64 = 5.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub position: f64,
    #[serde(default)]
    pub note: String,
}

impl Bookmark {
    pub(crate) fn is_valid(&self) -> bool {
        self.position.is_finite()
            && self.position >= 0.0
            && self.note.chars().count() <= MAX_NOTE
            && !self.note.chars().any(char::is_control)
    }
}

/// What is remembered about a book or video: a file path or a folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemState {
    pub key: String,
    /// Size and modified time (seconds) of the file when it was remembered; 0 for folders.
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime: i64,
    #[serde(default)]
    pub position: Option<f64>,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub speed: Option<f64>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
}

impl ItemState {
    fn new(key: &str, stamp: Option<(u64, i64)>) -> Self {
        let (size, mtime) = stamp.unwrap_or_default();
        Self {
            key: key.to_string(),
            size,
            mtime,
            position: None,
            finished: false,
            speed: None,
            bookmarks: Vec::new(),
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        !self.key.is_empty()
            && self.key.len() <= MAX_KEY
            && self.position.is_none_or(|p| p.is_finite() && p >= 0.0)
            && self
                .speed
                .is_none_or(|s| s.is_finite() && (0.5..=3.0).contains(&s))
            && self.bookmarks.len() <= MAX_BOOKMARKS
            && self.bookmarks.iter().all(Bookmark::is_valid)
    }
}

/// Size and mtime of a local file, by its `file` uri path (`/C:/x` on Windows).
fn stamp_of(path: &str) -> Option<(u64, i64)> {
    // A local stat of one file the user just played; remote items are never remembered.
    let os_path = fm_vfs::native_path(path, cfg!(windows))?;
    let meta = std::fs::metadata(os_path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    Some((meta.len(), mtime))
}

fn is_m4b(uri: &VfsUri) -> bool {
    uri.file_name()
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".m4b"))
}

/// Letters only, lowercase: "Audio Books" and "audio_books" both become "audiobooks".
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A folder of several files is treated as a book when it sits under a folder named for
/// audiobooks, or when most of its files are named like chapters ("Chapter 03", "Part 2").
fn looks_like_book(dir: &str, members: &[VfsUri]) -> bool {
    const SHELF: [&str; 3] = ["audiobook", "hoerbuch", "hörbuch"];
    if dir
        .split('/')
        .any(|part| SHELF.iter().any(|word| squash(part).contains(word)))
    {
        return true;
    }
    const CHAPTER: [&str; 7] = [
        "chapter",
        "part",
        "disc",
        "disk",
        "hoofdstuk",
        "kapitel",
        "chapitre",
    ];
    let named = members
        .iter()
        .filter(|uri| {
            uri.file_name().is_some_and(|name| {
                let lower = name.to_lowercase();
                CHAPTER.iter().any(|word| lower.contains(word))
            })
        })
        .count();
    members.len() >= 2 && named * 2 >= members.len()
}

fn genre_is_audiobook(info: &AudioInfo) -> bool {
    info.tags
        .genre
        .as_deref()
        .is_some_and(|genre| genre.to_ascii_lowercase().contains("audiobook"))
}

/// A running worker probe; dropping it cancels the work.
#[derive(Debug)]
struct Probe<T> {
    receiver: Receiver<T>,
    cancelled: Arc<AtomicBool>,
}

impl<T> Drop for Probe<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

type Measured = Vec<(String, Option<f64>)>;

#[derive(Debug, Default)]
pub struct BookExtras {
    /// Embedded chapters of the current item, in seconds into that file.
    pub chapters: Vec<Chapter>,
    chapters_for: Option<VfsUri>,
    chapters_probed: bool,
    chapter_job: Option<Probe<Vec<Chapter>>>,
    durations: HashMap<String, f64>,
    duration_job: Option<Probe<Measured>>,
    /// Folders and files the user marked as audiobooks.
    pub marked: Vec<String>,
    /// Remembered books and videos, newest last.
    pub items: Vec<ItemState>,
}

/// An item (or folder) that behaves as an audiobook or remembered video.
#[derive(Clone, Debug, PartialEq)]
pub struct BookRef {
    /// Folder path for a folder book, file path otherwise.
    pub key: String,
    /// Queue rows of a folder book.
    pub folder: Option<Range<usize>>,
    pub video: bool,
}

/// Which audiobook entries a row's context menu shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BookMenu {
    pub is_book: bool,
    /// The user marked it as an audiobook (so it can be unmarked).
    pub marked: bool,
    pub finished: bool,
}

/// A playlist row's remembered state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowMark {
    /// Playing the row resumes at this many seconds.
    Resume(f64),
    /// Earlier than the resume point of its folder.
    Played,
    Finished,
}

/// Everything the pane shows about the current book, in book seconds.
#[derive(Clone, Debug)]
pub struct BookView {
    pub folder: bool,
    pub position: f64,
    pub total: Option<f64>,
    pub chapters: Vec<Chapter>,
    pub chapter: Option<usize>,
    pub finished: bool,
    pub bookmarks: Vec<Bookmark>,
}

impl BookExtras {
    #[cfg(test)]
    pub(crate) fn set_duration_for_tests(&mut self, key: &str, duration: f64) {
        self.durations.insert(key.to_string(), duration);
    }

    #[cfg(test)]
    pub(crate) fn set_chapters_for_tests(
        &mut self,
        uri: Option<VfsUri>,
        chapters: &[(&str, f64, f64)],
    ) {
        self.chapters_for = uri;
        self.chapters = chapters
            .iter()
            .filter_map(|(t, s, e)| Chapter::new(t, *s, *e))
            .collect();
    }
}

impl MediaSession {
    pub fn is_marked(&self, key: &str) -> bool {
        self.books.marked.iter().any(|marked| marked == key)
    }

    /// The book (or remembered video) a queue row belongs to.
    pub fn book_ref(&self, index: usize) -> Option<BookRef> {
        let uri = self.queue.items().get(index)?;
        let video = is_video(uri);
        let genre = self.queue.current_index() == Some(index)
            && self
                .info
                .as_ref()
                .is_some_and(|info| genre_is_audiobook(info));
        let group = self
            .queue
            .groups()
            .into_iter()
            .find(|g| index >= g.start && index - g.start < g.len);
        if let Some(group) = group
            && let Some(dir) = group.dir
        {
            let range = group.start..group.start + group.len;
            let members = self.queue.items().get(range.clone())?;
            if !video
                && (genre
                    || self.is_marked(&dir)
                    || members.iter().any(is_m4b)
                    || looks_like_book(&dir, members))
                && range.len() <= MAX_BOOK_FILES
            {
                return Some(BookRef {
                    key: dir,
                    folder: Some(range),
                    video: false,
                });
            }
        }
        uri.local_path()?;
        let key = uri.path_str().to_string();
        (video || genre || is_m4b(uri) || self.is_marked(&key)).then_some(BookRef {
            key,
            folder: None,
            video,
        })
    }

    /// What the right-click menu of a row can offer.
    pub fn book_menu(&self, index: usize) -> BookMenu {
        let book = self.book_ref(index);
        let key = book.as_ref().map(|b| b.key.clone()).or_else(|| {
            let grouped = self
                .queue
                .group_of(index)
                .and_then(|g| g.dir.map(str::to_string));
            grouped.or_else(|| {
                self.queue
                    .items()
                    .get(index)
                    .map(|u| u.path_str().to_string())
            })
        });
        BookMenu {
            is_book: book.is_some(),
            marked: key.as_deref().is_some_and(|k| self.is_marked(k)),
            finished: book
                .as_ref()
                .is_some_and(|b| self.state_of(&b.key).is_some_and(|s| s.finished)),
        }
    }

    pub fn state_of(&self, key: &str) -> Option<&ItemState> {
        self.books.items.iter().find(|state| state.key == key)
    }

    /// Find or create the state for `key`, mark it most recent, and return its position in
    /// `items`. A changed file (size or modified time) loses what was remembered.
    pub(crate) fn touch_state(&mut self, key: &str, file: bool) -> usize {
        let stamp = if file { stamp_of(key) } else { None };
        let items = &mut self.books.items;
        let state = match items.iter().position(|state| state.key == key) {
            Some(at) => {
                let mut state = items.remove(at);
                if let Some((size, mtime)) = stamp
                    && (state.size, state.mtime) != (0, 0)
                    && (state.size, state.mtime) != (size, mtime)
                {
                    state = ItemState::new(key, stamp);
                } else if let Some((size, mtime)) = stamp {
                    (state.size, state.mtime) = (size, mtime);
                }
                state
            }
            None => ItemState::new(key, stamp),
        };
        if items.len() >= MAX_ITEMS {
            items.remove(0);
        }
        items.push(state);
        items.len() - 1
    }

    fn known_duration(&self, index: usize) -> Option<f64> {
        let uri = self.queue.items().get(index)?;
        if self.queue.current_index() == Some(index)
            && let Some(duration) = self.duration()
        {
            return Some(duration);
        }
        self.books
            .durations
            .get(uri.path_str())
            .copied()
            .filter(|d| *d >= 0.0)
    }

    /// The whole book as one timeline, once every file has been measured.
    pub(crate) fn book_timeline(&self, range: &Range<usize>) -> Option<Timeline> {
        let durations: Option<Vec<f64>> = range.clone().map(|i| self.known_duration(i)).collect();
        Timeline::new(&durations?)
    }

    pub fn book_view(&self) -> Option<BookView> {
        let index = self.queue.current_index()?;
        let book = self.book_ref(index)?;
        let timeline = book
            .folder
            .as_ref()
            .and_then(|range| self.book_timeline(range));
        let start = book.folder.as_ref().map_or(0, |range| range.start);
        let offset = timeline
            .as_ref()
            .and_then(|t| t.start_of(index - start))
            .unwrap_or(0.0);
        let total = match (&timeline, &book.folder) {
            (Some(t), _) => Some(t.total()),
            (None, None) => self.duration(),
            (None, Some(_)) => None,
        };
        let position = offset + self.position;
        let mut chapters: Vec<Chapter> = if self.books.chapters_for.as_ref() == self.queue.current()
        {
            self.books
                .chapters
                .iter()
                .filter_map(|c| Chapter::new(&c.title, c.start + offset, c.end + offset))
                .collect()
        } else {
            Vec::new()
        };
        if chapters.len() < 2 {
            chapters = timeline
                .as_ref()
                .filter(|t| t.len() >= 2)
                .zip(book.folder.as_ref())
                .map(|(t, range)| {
                    let titles: Vec<String> = self
                        .queue
                        .items()
                        .get(range.clone())
                        .unwrap_or_default()
                        .iter()
                        .map(|uri| {
                            let name = uri.file_name().unwrap_or("");
                            name.rsplit_once('.')
                                .map_or(name, |(stem, _)| stem)
                                .to_string()
                        })
                        .collect();
                    t.file_chapters(&titles)
                })
                .unwrap_or_default();
        }
        let state = self.state_of(&book.key);
        Some(BookView {
            folder: book.folder.is_some(),
            position,
            total,
            chapter: fm_media::chapter_index(&chapters, position),
            chapters,
            finished: state.is_some_and(|s| s.finished),
            bookmarks: state.map(|s| s.bookmarks.clone()).unwrap_or_default(),
        })
    }

    /// Position to continue a book or video queue row from, rewound a little.
    pub fn remembered_position(&self, index: usize) -> Option<f64> {
        match self.row_mark(index)? {
            RowMark::Resume(position) => Some((position - REWIND_SECS).max(0.0)),
            RowMark::Played | RowMark::Finished => None,
        }
    }

    /// What the playlist shows for a row: where it would resume, or that its book is finished.
    pub fn row_mark(&self, index: usize) -> Option<RowMark> {
        let uri = self.queue.items().get(index)?;
        let book = self.book_ref(index)?;
        // Only a folder audiobook plays as one timeline; videos in a folder (episodes, DVD
        // parts) are remembered one by one, so earlier ones are never marked as played.
        if book.folder.is_some() {
            let dir = book.key;
            if self.state_of(&dir).is_some_and(|s| s.finished) {
                return Some(RowMark::Finished);
            }
            let name = uri.file_name()?;
            let point = self.resume.iter().find(|p| p.dir == dir)?;
            if point.file == name {
                return Some(RowMark::Resume(point.position));
            }
            // Files before the resume point have been listened to.
            let group = self
                .queue
                .groups()
                .into_iter()
                .find(|g| index >= g.start && index - g.start < g.len)?;
            let members = self
                .queue
                .items()
                .get(group.start..group.start + group.len)?;
            let resume_at = members
                .iter()
                .position(|m| m.file_name() == Some(point.file.as_str()))?;
            return (index - group.start < resume_at).then_some(RowMark::Played);
        }
        let state = self.state_of(uri.path_str())?;
        if let Some((size, mtime)) = stamp_of(uri.path_str())
            && (state.size, state.mtime) != (0, 0)
            && (state.size, state.mtime) != (size, mtime)
        {
            return None;
        }
        if state.finished {
            return Some(RowMark::Finished);
        }
        state.position.map(RowMark::Resume)
    }

    /// Remember the playing book or video, and notice when it has been played to its end.
    pub(crate) fn note_book(&mut self) {
        self.note_resume();
        let Some(index) = self.queue.current_index() else {
            return;
        };
        let Some(book) = self.book_ref(index) else {
            return;
        };
        let Some(duration) = self.duration() else {
            return;
        };
        let position = self.position;
        let done = fm_media::is_finished(position, duration);
        let grouped = self.queue.group_of(index).is_some_and(|g| g.dir.is_some());
        match &book.folder {
            Some(range) => {
                let last = index + 1 == range.end;
                if last && done {
                    self.finish_folder(&book.key);
                } else if self.state_of(&book.key).is_some_and(|s| s.finished) {
                    let at = self.touch_state(&book.key, false);
                    if let Some(state) = self.books.items.get_mut(at) {
                        state.finished = false;
                    }
                }
            }
            // A file in a folder book is tracked by the folder's resume point; a video in a
            // folder is still remembered on its own.
            None if grouped && !book.video => {}
            None => {
                let unchanged = self.books.items.last().is_some_and(|s| {
                    s.key == book.key
                        && s.finished == done
                        && s.position == (!done).then_some(position)
                });
                if unchanged {
                    return;
                }
                let at = self.touch_state(&book.key, true);
                if let Some(state) = self.books.items.get_mut(at) {
                    state.finished = done;
                    state.position = (!done).then_some(position);
                }
            }
        }
    }

    /// The folder was played to its end (or marked finished): next play starts over.
    pub(crate) fn finish_folder(&mut self, dir: &str) {
        self.resume.retain(|p| p.dir != dir);
        let at = self.touch_state(dir, false);
        if let Some(state) = self.books.items.get_mut(at) {
            state.finished = true;
        }
    }

    /// Remembered state with positions rounded so saving does not follow every tick.
    pub(crate) fn items_snapshot(&self) -> Vec<ItemState> {
        self.books
            .items
            .iter()
            .map(|state| ItemState {
                position: state
                    .position
                    .map(|p| (p / SAVE_STEP_SECS).floor() * SAVE_STEP_SECS),
                ..state.clone()
            })
            .collect()
    }

    /// Position of the current item for the saved queue, in 5 s steps like the item states.
    pub(crate) fn saved_position(&self) -> Option<f64> {
        self.queue.current_index()?;
        Some((self.position / SAVE_STEP_SECS).floor() * SAVE_STEP_SECS).filter(|p| *p > 0.0)
    }

    pub(crate) fn saved_length(&self) -> Option<f64> {
        self.duration()
    }

    /// Show the saved spot of the restored current item; playing starts there.
    pub(crate) fn restore_position(&mut self, position: Option<f64>, length: Option<f64>) {
        let Some(uri) = self.queue.current().cloned() else {
            return;
        };
        if let Some(length) = length.filter(|l| *l > 0.0) {
            self.books
                .durations
                .insert(uri.path_str().to_string(), length);
        }
        self.position = position.unwrap_or(0.0);
    }

    /// Length remembered from an earlier run, for display before the item is opened.
    pub(crate) fn cached_length(&self) -> Option<f64> {
        let uri = self.queue.current()?;
        self.books
            .durations
            .get(uri.path_str())
            .copied()
            .filter(|d| *d > 0.0)
    }

    pub(crate) fn restore_books(&mut self, items: Vec<ItemState>, marked: Vec<String>) {
        self.books.items = items;
        self.books.marked = marked;
    }
}

impl Session {
    /// Start chapter and duration probes for the current book; collect finished ones.
    pub(crate) fn media_book_tick(&mut self) -> bool {
        let mut changed = false;
        let current = self.media.queue.current().cloned();
        if self.media.books.chapters_for != current {
            self.media.books.chapters.clear();
            self.media.books.chapter_job = None;
            self.media.books.chapters_probed = false;
            self.media.books.chapters_for = current.clone();
            changed = true;
        }
        if let (Some(uri), Some(index)) = (current, self.media.queue.current_index()) {
            if let Some(info) = self.media.info.clone()
                && let Some(duration) = info.duration
            {
                self.media_note_duration(uri.path_str(), duration);
            }
            if !self.media.books.chapters_probed
                && self.media.books.chapter_job.is_none()
                && uri.local_path().is_some()
                && self.media.book_ref(index).is_some()
            {
                self.media.books.chapters_probed = true;
                self.media.books.chapter_job = Some(self.spawn_probe(move |cancelled| {
                    fm_preview::video::chapters(&uri, cancelled).unwrap_or_default()
                }));
            }
        }
        if let Some(probe) = &self.media.books.chapter_job {
            match probe.receiver.try_recv() {
                Ok(chapters) => {
                    self.media.books.chapters = chapters;
                    self.media.books.chapter_job = None;
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => self.media.books.chapter_job = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        changed |= self.media_measure_books();
        changed
    }

    fn spawn_probe<T: Send + 'static>(
        &self,
        work: impl FnOnce(&AtomicBool) -> T + Send + 'static,
    ) -> Probe<T> {
        let (tx, receiver) = sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        self.jobs.submit(JobLane::Local, move |ctx| {
            if ctx.is_cancelled() || flag.load(Ordering::Acquire) {
                return Ok(());
            }
            let _ = tx.send(work(&flag));
            Ok(())
        });
        Probe {
            receiver,
            cancelled,
        }
    }

    /// Remember a measured length; a negative one marks a file that could not be measured.
    fn media_note_duration(&mut self, key: &str, duration: f64) {
        let durations = &mut self.media.books.durations;
        if durations.len() >= MAX_DURATIONS && !durations.contains_key(key) {
            durations.clear();
        }
        if duration.is_finite() {
            durations.insert(key.to_string(), duration);
        }
    }

    /// Measure the files of the current book, and of books with a saved position, one
    /// folder at a time, so totals and progress can be shown.
    fn media_measure_books(&mut self) -> bool {
        let mut changed = false;
        if let Some(probe) = &self.media.books.duration_job {
            match probe.receiver.try_recv() {
                Ok(found) => {
                    self.media.books.duration_job = None;
                    for (key, duration) in found {
                        self.media_note_duration(&key, duration.unwrap_or(-1.0));
                    }
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => self.media.books.duration_job = None,
                Err(TryRecvError::Empty) => return changed,
            }
        }
        let wanted = self.media_unmeasured();
        if wanted.is_empty() {
            return changed;
        }
        self.media.books.duration_job = Some(self.spawn_probe(move |cancelled| {
            let mut found = Vec::new();
            for (key, path) in wanted {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let duration = fm_media::AudioDecoder::open(&path)
                    .ok()
                    .and_then(|decoder| decoder.info().duration);
                found.push((key, duration));
            }
            found
        }));
        changed
    }

    /// Files of the first book that needs measuring: the current one, then saved ones.
    fn media_unmeasured(&self) -> Vec<(String, std::path::PathBuf)> {
        let media = &self.media;
        let mut order: Vec<Range<usize>> = Vec::new();
        if let Some(book) = media.queue.current_index().and_then(|i| media.book_ref(i))
            && let Some(range) = book.folder
        {
            order.push(range);
        }
        for group in media.queue.groups() {
            let remembered = group
                .dir
                .as_deref()
                .is_some_and(|dir| media.resume.iter().any(|p| p.dir == dir));
            if remembered && group.len <= MAX_BOOK_FILES {
                let range = group.start..group.start + group.len;
                if media
                    .book_ref(range.start)
                    .is_some_and(|b| b.folder.is_some())
                {
                    order.push(range);
                }
            }
        }
        for range in order {
            let missing: Vec<_> = range
                .filter(|i| media.queue.current_index() != Some(*i))
                .filter_map(|i| media.queue.items().get(i))
                .filter(|uri| !media.books.durations.contains_key(uri.path_str()))
                .filter_map(|uri| Some((uri.path_str().to_string(), uri.local_path()?)))
                .collect();
            if !missing.is_empty() {
                return missing;
            }
        }
        Vec::new()
    }
}
