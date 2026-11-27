//! Shows a large archive's folders while it is still being read (`archive-browsing`
//! "Large archives list as they are read"). Formats that read from the start (compressed
//! tars, RAR through a tool) add entries to the mount's live index as they go; this
//! copies what a folder has so far into the directory cache every so often, so the pane
//! and the tree fill up instead of waiting. The job that opens the archive stays in
//! charge: its result replaces the partial listing, whether it worked or failed.

use std::time::{Duration, Instant};

use fm_vfs::VfsUri;

use crate::session::Session;

/// Shortest time between two copies.
const MIN_INTERVAL: Duration = Duration::from_millis(250);
/// Longest time between two copies, however many entries there are.
const MAX_INTERVAL: Duration = Duration::from_secs(10);

/// Spaces the copies out: the more entries, the more a copy costs a frame.
#[derive(Debug, Default)]
pub struct Pacer {
    next: Option<Instant>,
}

impl Pacer {
    /// Whether a copy is due at `now`, with `entries` read in all; if so, the next one is
    /// scheduled.
    fn due(&mut self, now: Instant, entries: usize) -> bool {
        if self.next.is_some_and(|at| now < at) {
            return false;
        }
        let extra = Duration::from_millis((entries / 100) as u64);
        self.next = Some(now + (MIN_INTERVAL + extra).min(MAX_INTERVAL));
        true
    }

    fn reset(&mut self) {
        self.next = None;
    }
}

/// What the pane shows while an archive is read.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub name: String,
    /// Entries read so far, for formats that list as they read.
    pub entries: usize,
    pub secs: u64,
    /// How far the read is, when the format knows.
    pub percent: Option<u32>,
}

/// How the open of the archive behind `location` is going, `None` when none is running.
pub fn reading(s: &Session, location: &VfsUri) -> Option<Reading> {
    let mount = s.archives.for_uri(location)?;
    let elapsed = mount.opening_for()?;
    let percent = mount
        .progress()
        .filter(|(_, total)| *total > 0)
        .map(|(done, total)| (done.min(total) as f64 * 100.0 / total as f64).round() as u32);
    Some(Reading {
        name: crate::archive_cmds::container_of(location)
            .and_then(|r| r.container.file_name().map(str::to_string))
            .unwrap_or_default(),
        entries: mount.read_so_far().unwrap_or(0),
        secs: elapsed.as_secs(),
        percent,
    })
}

/// Copy what the archives being opened have read so far into the folders waiting for
/// them. `true` when a listing changed (the UI should repaint). Called from `poll`.
pub(crate) fn tick(s: &mut Session, now: Instant) -> bool {
    let reading = s.archives.being_read();
    if reading.is_empty() {
        s.archives.fill.reset();
        return false;
    }
    let entries = reading.iter().filter_map(|m| m.read_so_far()).sum();
    if !s.archives.fill.due(now, entries) {
        return false;
    }
    let waiting: Vec<VfsUri> = s
        .dirs
        .unfinished()
        .filter(|dir| {
            dir.archive_handle()
                .is_some_and(|h| reading.iter().any(|m| m.root().archive_handle() == Some(h)))
        })
        .cloned()
        .collect();
    let mut changed = false;
    for dir in waiting {
        let Some((found, _)) = reading
            .iter()
            .find_map(|m| m.partial_read_dir(&dir))
            .filter(|(found, _)| !found.is_empty())
        else {
            continue;
        };
        if s.dirs.set_partial(&dir, found.into()).is_some() {
            s.after_read(&dir);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dircache::{DirState, ReadOutcome};
    use fm_archive::{Meta, NodeKind};

    #[test]
    fn copies_come_further_apart_for_bigger_archives() {
        let mut p = Pacer::default();
        let t0 = Instant::now();
        assert!(p.due(t0, 0));
        assert!(!p.due(t0 + Duration::from_millis(100), 0));
        assert!(p.due(t0 + Duration::from_millis(300), 0));
        let t1 = t0 + Duration::from_secs(1);
        assert!(p.due(t1, 500_000));
        assert!(!p.due(t1 + Duration::from_secs(4), 500_000), "5 s apart");
        assert!(p.due(t1 + Duration::from_secs(6), 500_000));
        let t2 = t1 + Duration::from_secs(60);
        assert!(p.due(t2, usize::MAX / 2), "capped at 10 s");
        assert!(p.due(t2 + Duration::from_secs(10), 0));
    }

    #[test]
    fn a_waiting_folder_fills_while_the_archive_is_read_and_the_job_result_replaces_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("big.tar.gz");
        std::fs::write(&path, b"not read in this test").expect("file");
        let mut s = Session::new().expect("session");
        let container = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
        let root = crate::archive_cmds::mount(&mut s, &container).expect("mount");
        let mount = s.archives.for_uri(&root).cloned().expect("lazy");
        s.dirs.begin_load(root.clone(), 9);
        let t0 = Instant::now();

        assert!(!tick(&mut s, t0), "nothing is being read");
        assert!(reading(&s, &root).is_none());
        let open = mount.open_guard();
        assert!(!tick(&mut s, t0), "nothing read yet");
        assert_eq!(s.dirs.state(&root), DirState::Loading { job: 9 });

        let add = |name: &str| {
            open.live()
                .add(name, NodeKind::File, Meta::default())
                .expect("add");
        };
        add("a.txt");
        add("src/b.txt");
        assert!(tick(&mut s, t0 + Duration::from_secs(1)));
        assert!(s.dirs.is_filling(&root));
        assert_eq!(
            s.dirs.entries(&root).map(|e| e.len()),
            Some(2),
            "a.txt, src"
        );
        let now = reading(&s, &root).expect("reading");
        assert_eq!((now.name.as_str(), now.entries), ("big.tar.gz", 3));

        add("c.txt");
        assert!(!tick(&mut s, t0 + Duration::from_secs(1)), "paced");
        assert!(tick(&mut s, t0 + Duration::from_secs(3)));
        assert_eq!(s.dirs.entries(&root).map(|e| e.len()), Some(3));

        drop(open);
        assert!(
            !tick(&mut s, t0 + Duration::from_secs(9)),
            "the open is over"
        );
        s.dirs
            .apply(&root, 9, ReadOutcome::Err("it failed".into()))
            .expect("job result");
        assert!(matches!(s.dirs.state(&root), DirState::Failed { .. }));
    }
}
