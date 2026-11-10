//! Audiobook arithmetic: chapters, a multi-file timeline, skips and the finished rule.

/// Chapters kept per item; longer lists are cut.
pub const MAX_CHAPTERS: usize = 5000;
/// Longest chapter title kept, in characters.
pub const MAX_CHAPTER_TITLE: usize = 200;
/// Within this many seconds of the end an item counts as finished.
pub const FINISHED_TAIL_SECS: f64 = 30.0;
/// A fraction of the duration after which an item counts as finished.
pub const FINISHED_FRACTION: f64 = 0.98;
/// Seconds into a chapter after which "previous" restarts it instead of leaving it.
pub const RESTART_GRACE_SECS: f64 = 3.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start: f64,
    pub end: f64,
}

impl Chapter {
    /// Build a chapter from untrusted values; `None` when the times are unusable.
    pub fn new(title: &str, start: f64, end: f64) -> Option<Self> {
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
            return None;
        }
        let title: String = title
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_CHAPTER_TITLE)
            .collect();
        Some(Self {
            title: title.trim().to_string(),
            start,
            end,
        })
    }
}

/// Index of the chapter playing at `position`: the last one that has started.
pub fn chapter_index(chapters: &[Chapter], position: f64) -> Option<usize> {
    chapters
        .iter()
        .rposition(|chapter| chapter.start <= position + 0.001)
        .or(if chapters.is_empty() { None } else { Some(0) })
}

/// The chapter "next" jumps to; `None` on the last chapter.
pub fn next_chapter(chapters: &[Chapter], position: f64) -> Option<usize> {
    let current = chapter_index(chapters, position)?;
    (current + 1 < chapters.len()).then_some(current + 1)
}

/// The chapter "previous" jumps to: this one again once it has run a few seconds.
pub fn previous_chapter(chapters: &[Chapter], position: f64) -> Option<usize> {
    let current = chapter_index(chapters, position)?;
    let started = chapters.get(current)?.start;
    if position - started > RESTART_GRACE_SECS || current == 0 {
        Some(current)
    } else {
        Some(current - 1)
    }
}

/// Several files played back to back as one book.
#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    starts: Vec<f64>,
    durations: Vec<f64>,
    total: f64,
}

impl Timeline {
    /// `None` unless there is at least one file and every duration is known and sane.
    pub fn new(durations: &[f64]) -> Option<Self> {
        if durations.is_empty() || durations.iter().any(|d| !d.is_finite() || *d < 0.0) {
            return None;
        }
        let mut starts = Vec::with_capacity(durations.len());
        let mut total = 0.0;
        for duration in durations {
            starts.push(total);
            total += duration;
        }
        Some(Self {
            starts,
            durations: durations.to_vec(),
            total,
        })
    }

    pub fn total(&self) -> f64 {
        self.total
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    pub fn start_of(&self, file: usize) -> Option<f64> {
        self.starts.get(file).copied()
    }

    /// Position in the whole book of `position` seconds into `file`.
    pub fn position(&self, file: usize, position: f64) -> Option<f64> {
        let start = self.start_of(file)?;
        let duration = self.durations.get(file).copied()?;
        Some(start + position.clamp(0.0, duration))
    }

    /// File and offset for a position in the whole book; clamped to the book.
    pub fn locate(&self, position: f64) -> (usize, f64) {
        let position = if position.is_finite() {
            position.clamp(0.0, self.total)
        } else {
            0.0
        };
        let file = self
            .starts
            .iter()
            .rposition(|start| *start <= position)
            .unwrap_or(0);
        let start = self.starts.get(file).copied().unwrap_or(0.0);
        let duration = self.durations.get(file).copied().unwrap_or(0.0);
        // A boundary belongs to the next file; the very end to the last one.
        (file, (position - start).min(duration))
    }

    /// One chapter per file, for books whose files carry no chapters.
    pub fn file_chapters(&self, titles: &[String]) -> Vec<Chapter> {
        self.starts
            .iter()
            .zip(&self.durations)
            .enumerate()
            .filter_map(|(i, (start, duration))| {
                let fallback = format!("Part {}", i + 1);
                let title = titles
                    .get(i)
                    .map(String::as_str)
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or(&fallback);
                Chapter::new(title, *start, start + duration)
            })
            .collect()
    }
}

/// Where a skip of `by` seconds lands, never before the start or past the end.
pub fn skip_target(position: f64, by: f64, total: f64) -> f64 {
    (position + by).clamp(0.0, total.max(0.0))
}

/// Played to within the last seconds (or 98 %): the next play starts over.
pub fn is_finished(position: f64, duration: f64) -> bool {
    if !position.is_finite() || !duration.is_finite() || duration <= 0.0 {
        return false;
    }
    position >= duration * FINISHED_FRACTION
        || (duration > 4.0 * FINISHED_TAIL_SECS && duration - position <= FINISHED_TAIL_SECS)
}

/// Progress from 0 to 1.
pub fn progress(position: f64, duration: f64) -> f64 {
    if duration > 0.0 && position.is_finite() {
        (position / duration).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapters() -> Vec<Chapter> {
        [
            ("One", 0.0, 100.0),
            ("Two", 100.0, 250.0),
            ("Three", 250.0, 300.0),
        ]
        .iter()
        .filter_map(|(t, s, e)| Chapter::new(t, *s, *e))
        .collect()
    }

    #[test]
    fn chapter_navigation_follows_the_position() {
        let c = chapters();
        assert_eq!(chapter_index(&c, 120.0), Some(1));
        assert_eq!(next_chapter(&c, 120.0), Some(2));
        assert_eq!(next_chapter(&c, 280.0), None);
        assert_eq!(previous_chapter(&c, 120.0), Some(1));
        assert_eq!(previous_chapter(&c, 101.0), Some(0));
        assert_eq!(previous_chapter(&c, 1.0), Some(0));
        assert_eq!(chapter_index(&[], 1.0), None);
    }

    #[test]
    fn hostile_chapters_are_rejected_or_cleaned() {
        assert!(Chapter::new("x", f64::NAN, 1.0).is_none());
        assert!(Chapter::new("x", 5.0, 1.0).is_none());
        assert!(Chapter::new("x", -1.0, 1.0).is_none());
        let long = "a\u{7}".repeat(1000);
        let chapter = Chapter::new(&long, 0.0, 1.0).expect("chapter");
        assert_eq!(chapter.title.chars().count(), MAX_CHAPTER_TITLE);
        assert!(!chapter.title.contains('\u{7}'));
    }

    #[test]
    fn a_timeline_crosses_file_boundaries() {
        let t = Timeline::new(&[100.0, 50.0, 25.0]).expect("timeline");
        assert_eq!(t.total(), 175.0);
        assert_eq!(t.locate(0.0), (0, 0.0));
        assert_eq!(t.locate(100.0), (1, 0.0));
        assert_eq!(t.locate(149.0), (1, 49.0));
        assert_eq!(t.locate(175.0), (2, 25.0));
        assert_eq!(t.locate(9999.0), (2, 25.0));
        assert_eq!(t.locate(f64::NAN), (0, 0.0));
        assert_eq!(t.position(1, 10.0), Some(110.0));
        assert_eq!(t.position(5, 10.0), None);
        assert!(Timeline::new(&[]).is_none());
        assert!(Timeline::new(&[1.0, f64::INFINITY]).is_none());
    }

    #[test]
    fn files_become_chapters_when_the_book_has_none() {
        let t = Timeline::new(&[10.0, 20.0]).expect("timeline");
        let c = t.file_chapters(&["Intro".into()]);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].title, "Intro");
        assert_eq!(c[1].title, "Part 2");
        assert_eq!(c[1].start, 10.0);
    }

    #[test]
    fn skips_clamp_and_finished_follows_the_tail_rule() {
        assert_eq!(skip_target(10.0, -15.0, 100.0), 0.0);
        assert_eq!(skip_target(90.0, 30.0, 100.0), 100.0);
        assert!(is_finished(3580.0, 3600.0));
        assert!(is_finished(3571.0, 3600.0));
        assert!(!is_finished(1800.0, 3600.0));
        assert!(!is_finished(5.0, 20.0));
        assert!(is_finished(19.7, 20.0));
        assert!(!is_finished(f64::NAN, 20.0));
        assert_eq!(progress(50.0, 200.0), 0.25);
        assert_eq!(progress(1.0, 0.0), 0.0);
    }
}
