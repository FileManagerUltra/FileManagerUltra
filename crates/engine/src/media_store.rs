//! Owner-only Media queue persistence. All file access runs on the local job lane.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use fm_media::{BANDS, EqSettings, MAX_QUEUE_ITEMS, Queue, RepeatMode};
use fm_vfs::{FmError, Scheme, VfsUri};
use serde::{Deserialize, Serialize};

use crate::media_audio_cmds::{MAX_PRESET_NAME, MAX_USER_PRESETS, UserPreset};
use crate::media_resume::{MAX_RESUME, ResumePoint};
use crate::{Session, jobs::JobLane};

const FILE: &str = "media-queue.json";
const MAX_BYTES: u64 = 170 * 1024 * 1024;
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    paths: Vec<String>,
    index: Option<usize>,
    volume: f32,
    shuffle: bool,
    repeat: Repeat,
    #[serde(default)]
    eq: Option<EqSnapshot>,
    #[serde(default)]
    resume: Vec<ResumePoint>,
    #[serde(default)]
    groups: Vec<SavedGroup>,
}

/// A playlist folder: a run of `len` items starting at `start`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedGroup {
    start: usize,
    len: usize,
    name: String,
    dir: Option<String>,
    /// Absent in older files: the folder keeps its default (automatic folders start locked).
    #[serde(default)]
    locked: Option<bool>,
}

/// Re-aim folder ranges after dropping the items whose `kept` flag is false.
fn remap_groups(groups: &[SavedGroup], kept: &[bool]) -> Vec<SavedGroup> {
    let before = |at: usize| kept.iter().take(at).filter(|k| **k).count();
    groups
        .iter()
        .filter_map(|group| {
            let end = group.start.checked_add(group.len)?;
            if end > kept.len() {
                return None;
            }
            let start = before(group.start);
            let len = before(end) - start;
            (len > 0 || group.len == 0).then(|| SavedGroup {
                start,
                len,
                name: group.name.clone(),
                dir: group.dir.clone(),
                locked: group.locked,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EqSnapshot {
    enabled: bool,
    bands: Vec<f32>,
    preset: Option<String>,
    user: Vec<(String, Vec<f32>)>,
}

impl EqSnapshot {
    /// `None` when the saved values are not usable; the equalizer then stays at its default.
    fn restore(&self) -> Option<(EqSettings, Option<String>, Vec<UserPreset>)> {
        let settings = EqSettings {
            enabled: self.enabled,
            bands: self.bands.as_slice().try_into().ok()?,
        };
        settings.validate().ok()?;
        if self.user.len() > MAX_USER_PRESETS {
            return None;
        }
        let user = self
            .user
            .iter()
            .map(|(name, bands)| {
                let bands: [f32; BANDS] = bands.as_slice().try_into().ok()?;
                let valid = EqSettings {
                    enabled: true,
                    bands,
                };
                (valid.validate().is_ok() && name.chars().count() <= MAX_PRESET_NAME).then(|| {
                    UserPreset {
                        name: name.clone(),
                        bands,
                    }
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some((settings, self.preset.clone(), user))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Repeat {
    Off,
    All,
    One,
}

impl From<RepeatMode> for Repeat {
    fn from(value: RepeatMode) -> Self {
        match value {
            RepeatMode::Off => Self::Off,
            RepeatMode::All => Self::All,
            RepeatMode::One => Self::One,
        }
    }
}

impl From<Repeat> for RepeatMode {
    fn from(value: Repeat) -> Self {
        match value {
            Repeat::Off => Self::Off,
            Repeat::All => Self::All,
            Repeat::One => Self::One,
        }
    }
}

#[derive(Debug)]
struct Restored {
    snapshot: Snapshot,
    missing: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct MediaStore {
    dir: Option<PathBuf>,
    load: Option<Receiver<Result<Option<Restored>, String>>>,
    save: Option<(Snapshot, Receiver<Result<(), String>>)>,
    saved: Option<Snapshot>,
    last_check: Option<Instant>,
}

impl MediaStore {
    fn busy(&self) -> bool {
        self.load.is_some() || self.save.is_some()
    }
}

impl Session {
    /// Begin restoration on a worker; edits made before it finishes win over saved state.
    pub fn media_store_start(&mut self, dir: PathBuf) {
        let (tx, rx) = sync_channel(1);
        let path = dir.join(FILE);
        self.jobs.submit(JobLane::Local, move |_| {
            let result =
                read_snapshot(&path).map_err(|error| fm_vfs::redact::redact(&error.to_string()));
            let _ = tx.send(result);
            Ok(())
        });
        self.media_store = MediaStore {
            dir: Some(dir),
            load: Some(rx),
            saved: Some(snapshot(self)),
            ..Default::default()
        };
    }

    pub(crate) fn media_store_busy(&self) -> bool {
        self.media_store.busy()
    }

    pub(crate) fn media_store_tick(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.media_store.load {
            match rx.try_recv() {
                Ok(result) => {
                    self.media_store.load = None;
                    match result {
                        Ok(Some(restored)) => {
                            if self.media_store.saved.as_ref() == Some(&snapshot(self)) {
                                match self.restore_media(&restored.snapshot) {
                                    Ok(()) => {
                                        for missing in restored.missing {
                                            let message = fm_vfs::redact::redact(&format!(
                                                "Media item unavailable: {missing}"
                                            ));
                                            self.messages.push(&message);
                                            self.notice = Some(message);
                                        }
                                        // Rewrite after skipping vanished items.
                                        self.media_store.saved = None;
                                    }
                                    Err(error) => self.messages.push(&error.to_string()),
                                }
                            } else {
                                let message = "Saved media queue was not restored because the queue changed during startup.";
                                self.messages.push(message);
                                self.notice = Some(message.into());
                                self.media_store.saved = None;
                            }
                            changed = true;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let message = format!("Cannot restore media queue: {error}");
                            self.messages.push(&message);
                            self.notice = Some(message);
                            // Do not overwrite an unreadable state with an empty queue.
                            self.media_store.dir = None;
                            changed = true;
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.media_store.load = None;
                    self.media_store.dir = None;
                    self.messages
                        .push("Cannot restore media queue: background job stopped.");
                    self.notice =
                        Some("Cannot restore media queue: background job stopped.".into());
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some((_, rx)) = &self.media_store.save {
            match rx.try_recv() {
                Ok(result) => {
                    if let Some((saved, _)) = self.media_store.save.take() {
                        match result {
                            Ok(()) => self.media_store.saved = Some(saved),
                            Err(error) => {
                                self.messages
                                    .push(&format!("Cannot save media queue: {error}"));
                                self.notice = Some(format!("Cannot save media queue: {error}"));
                                self.media_store.dir = None;
                                changed = true;
                            }
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.media_store.save = None;
                    self.media_store.dir = None;
                    self.messages
                        .push("Cannot save media queue: background job stopped.");
                    self.notice = Some("Cannot save media queue: background job stopped.".into());
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.media_store.load.is_some() || self.media_store.save.is_some() {
            return changed;
        }
        if self
            .media_store
            .last_check
            .is_some_and(|last| last.elapsed() < SAVE_INTERVAL)
        {
            return changed;
        }
        self.media_store.last_check = Some(Instant::now());
        let Some(dir) = self.media_store.dir.clone() else {
            return changed;
        };
        let current = snapshot(self);
        if self.media_store.saved.as_ref() == Some(&current) {
            return changed;
        }
        let pending = current.clone();
        let (tx, rx) = sync_channel(1);
        self.jobs.submit(JobLane::Local, move |_| {
            let result = write_snapshot(&dir, &pending)
                .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
            let _ = tx.send(result);
            Ok(())
        });
        self.media_store.save = Some((current, rx));
        changed
    }

    /// Called only during app shutdown; wait for any prior write before committing the last state.
    pub fn media_store_finish(&mut self) -> Result<(), FmError> {
        let Some(dir) = self.media_store.dir.clone() else {
            return Ok(());
        };
        if let Some(rx) = self.media_store.load.take() {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(Some(restored)))
                    if self.media_store.saved.as_ref() == Some(&snapshot(self)) =>
                {
                    self.restore_media(&restored.snapshot)?;
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(FmError::backend("media", error)),
                Err(_) => return Err(FmError::backend("media", "Timed out restoring media queue")),
            }
        }
        if let Some((_, rx)) = self.media_store.save.take() {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(FmError::backend("media", error)),
                Err(_) => return Err(FmError::backend("media", "Timed out saving media queue")),
            }
        }
        let current = snapshot(self);
        let (tx, rx) = sync_channel(1);
        self.jobs.submit(JobLane::Local, move |_| {
            let result = write_snapshot(&dir, &current)
                .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
            let _ = tx.send(result);
            Ok(())
        });
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(FmError::backend("media", error)),
            Err(_) => Err(FmError::backend("media", "Timed out saving media queue")),
        }
    }

    fn restore_media(&mut self, saved: &Snapshot) -> Result<(), FmError> {
        let mut queue = Queue::default();
        for path in &saved.paths {
            queue.enqueue(local_uri(path)?)?;
        }
        if let Some(index) = saved.index {
            queue.select(index)?;
        }
        for group in &saved.groups {
            // A damaged folder entry only loses the folder, never the queue.
            let made = if group.len == 0 {
                queue.create_group(&group.name, group.dir.clone())
            } else {
                queue.group_range(group.start, group.len, &group.name, group.dir.clone())
            };
            if let (Ok(id), Some(locked)) = (made, group.locked) {
                let _ = queue.set_group_locked(id, locked);
            }
        }
        self.media.resume = saved.resume.clone();
        queue.set_repeat(saved.repeat.into());
        queue.set_shuffle(saved.shuffle, 0x9e37_79b9);
        self.media_set_restored(queue, saved.volume)?;
        if let Some((settings, preset, user)) = saved.eq.as_ref().and_then(EqSnapshot::restore) {
            self.media_restore_eq(settings, preset, user)?;
        }
        Ok(())
    }
}

fn snapshot(session: &Session) -> Snapshot {
    // Remote items are not persisted; the saved index follows the kept items.
    let current = session.media.queue.current_index();
    let mut index = None;
    let mut paths = Vec::new();
    let mut kept = Vec::new();
    for (at, uri) in session.media.queue.items().iter().enumerate() {
        kept.push(uri.local_path().is_some());
        if uri.local_path().is_none() {
            continue;
        }
        if current == Some(at) {
            index = Some(paths.len());
        }
        paths.push(
            if uri.scheme() == Scheme::File && uri.authority().user.is_none() {
                uri.path_str().to_string()
            } else {
                uri.to_string()
            },
        );
    }
    let groups: Vec<SavedGroup> = session
        .media
        .queue
        .groups()
        .into_iter()
        .map(|g| SavedGroup {
            start: g.start,
            len: g.len,
            name: g.name,
            dir: g.dir,
            locked: Some(g.locked),
        })
        .collect();
    Snapshot {
        groups: remap_groups(&groups, &kept),
        paths,
        index,
        volume: session.media.volume,
        shuffle: session.media.queue.shuffle(),
        repeat: session.media.queue.repeat().into(),
        resume: session.media.resume_snapshot(),
        eq: Some({
            let eq = session.media.gain.eq();
            EqSnapshot {
                enabled: eq.enabled,
                bands: eq.bands.to_vec(),
                preset: session.media.extras.eq_preset.clone(),
                user: session
                    .media
                    .extras
                    .user_presets
                    .iter()
                    .map(|p| (p.name.clone(), p.bands.to_vec()))
                    .collect(),
            }
        }),
    }
}

fn read_snapshot(path: &Path) -> Result<Option<Restored>, FmError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > MAX_BYTES {
        return Err(FmError::InvalidParams(
            "saved media queue is too large".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(FmError::InvalidParams(
            "saved media queue is too large".into(),
        ));
    }
    let mut saved: Snapshot = serde_json::from_slice(&bytes)
        .map_err(|_| FmError::InvalidParams("invalid saved media queue".into()))?;
    if saved.paths.len() > MAX_QUEUE_ITEMS
        || !saved.volume.is_finite()
        || !(0.0..=100.0).contains(&saved.volume)
        || saved.index.is_some_and(|index| index >= saved.paths.len())
    {
        return Err(FmError::InvalidParams(
            "invalid saved media queue settings".into(),
        ));
    }
    let mut paths = Vec::with_capacity(saved.paths.len());
    let mut missing = Vec::new();
    let mut selected = None;
    let mut kept = Vec::with_capacity(saved.paths.len());
    for (index, path) in saved.paths.iter().enumerate() {
        let uri = local_uri(path)?;
        let Some(local) = uri.local_path() else {
            return Err(FmError::InvalidParams("invalid saved local path".into()));
        };
        let result = std::fs::metadata(local);
        kept.push(matches!(&result, Ok(metadata) if metadata.is_file()));
        match result {
            Ok(metadata) if metadata.is_file() => {
                if saved.index == Some(index) {
                    selected = Some(paths.len());
                }
                paths.push(uri.path_str().to_string());
            }
            Ok(_) => missing.push(uri.file_name().unwrap_or("item").to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(uri.file_name().unwrap_or("item").to_string());
            }
            Err(error) => return Err(error.into()),
        }
    }
    saved.groups = remap_groups(&saved.groups, &kept);
    saved.groups.truncate(fm_media::MAX_GROUPS);
    saved.paths = paths;
    saved.index = selected;
    saved.resume.retain(ResumePoint::is_valid);
    saved.resume.truncate(MAX_RESUME);
    Ok(Some(Restored {
        snapshot: saved,
        missing,
    }))
}

fn write_snapshot(dir: &Path, snapshot: &Snapshot) -> Result<(), FmError> {
    for path in &snapshot.paths {
        local_uri(path)?;
    }
    let bytes = serde_json::to_vec(snapshot)
        .map_err(|_| FmError::InvalidParams("cannot encode media queue".into()))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(FmError::InvalidParams(
            "media queue is too large to save".into(),
        ));
    }
    crate::prefs::write_atomic(dir, FILE, &bytes)
}

fn local_uri(path: &str) -> Result<VfsUri, FmError> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(FmError::InvalidParams(
            "unsupported saved media path".into(),
        ));
    }
    let uri = VfsUri::parse(&format!("file://{path}"))
        .map_err(|_| FmError::InvalidParams("invalid saved media path".into()))?;
    if uri.local_path().is_none() {
        return Err(FmError::InvalidParams("invalid saved media path".into()));
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    #[test]
    fn folder_ranges_follow_the_items_that_are_kept() {
        let group = |start, len| SavedGroup {
            start,
            len,
            name: "g".into(),
            dir: None,
            locked: None,
        };
        let kept = [true, false, true, true, false, true];
        let out = remap_groups(
            &[
                group(1, 3),
                group(2, 1),
                group(4, 1),
                group(6, 0),
                group(5, 9),
            ],
            &kept,
        );
        let shape: Vec<_> = out.iter().map(|g| (g.start, g.len)).collect();
        assert_eq!(shape, [(1, 2), (1, 1), (4, 0)]);
    }

    use super::*;

    fn wait(session: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.media_store_busy() {
            session.poll();
            assert!(Instant::now() < deadline, "media store did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn restores_queue_paused_and_skips_missing_items() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.mp3");
        let b = dir.path().join("b.mp3");
        std::fs::write(&a, b"a").expect("a");
        std::fs::write(&b, b"b").expect("b");
        let mut original = Session::new().expect("session");
        original.media_store_start(dir.path().to_owned());
        wait(&mut original);
        for path in [&a, &b] {
            original
                .media
                .queue
                .enqueue(VfsUri::parse(path.to_str().expect("utf8")).expect("uri"))
                .expect("enqueue");
        }
        original.media.queue.select(1).expect("select");
        original.media.queue.set_repeat(RepeatMode::All);
        original.media.queue.set_shuffle(true, 5);
        original
            .media_set_restored(original.media.queue.clone(), 42.0)
            .expect("volume");
        original
            .dispatch("media.eq", &serde_json::json!({"preset": "Vocal"}))
            .expect("eq");
        original
            .dispatch("media.eq", &serde_json::json!({"save_as": "Mine"}))
            .expect("save preset");
        original.media_store_finish().expect("save");
        std::fs::remove_file(a).expect("remove");

        let mut restored = Session::new().expect("session");
        restored.media_store_start(dir.path().to_owned());
        wait(&mut restored);
        assert_eq!(restored.media.queue.items().len(), 1);
        assert_eq!(restored.media.queue.current_index(), Some(0));
        assert_eq!(restored.media.volume, 42.0);
        assert!(restored.media.queue.shuffle());
        assert_eq!(restored.media.queue.repeat(), RepeatMode::All);
        assert!(!restored.media.playing);
        assert!(restored.media.gain.eq().enabled);
        assert_eq!(restored.media.extras.user_presets.len(), 1);
        assert_eq!(restored.media.extras.eq_preset.as_deref(), Some("Mine"));
        assert!(
            restored
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("a.mp3"))
        );
        restored.media_store_finish().expect("save filtered queue");
    }

    #[test]
    fn remote_items_are_not_persisted_and_the_index_follows_local_ones() {
        let mut session = Session::new().expect("session");
        for raw in ["sftp://nas/a.mp3", "/music/b.mp3", "/music/c.mp3"] {
            session
                .media
                .queue
                .enqueue(VfsUri::parse(raw).expect("uri"))
                .expect("enqueue");
        }
        session.media.queue.select(2).expect("select");
        let saved = snapshot(&session);
        assert_eq!(saved.paths, ["/music/b.mp3", "/music/c.mp3"]);
        assert_eq!(saved.index, Some(1));
        session.media.queue.select(0).expect("select");
        assert_eq!(snapshot(&session).index, None);
        let dir = tempfile::tempdir().expect("tempdir");
        write_snapshot(dir.path(), &saved).expect("local items save");
    }

    #[test]
    fn startup_edits_win_and_invalid_state_is_not_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = Session::new().expect("session");
        std::fs::write(dir.path().join(FILE), b"{bad").expect("write");
        session.media_store_start(dir.path().to_owned());
        wait(&mut session);
        session.media_store_finish().expect("disabled");
        assert_eq!(std::fs::read(dir.path().join(FILE)).expect("read"), b"{bad");
        assert!(
            session
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("restore"))
        );

        let existing = dir.path().join("existing.mp3");
        let edited = dir.path().join("edited.mp3");
        std::fs::write(&existing, b"").expect("file");
        let saved = Snapshot {
            paths: vec![existing.to_str().expect("utf8").to_string()],
            index: Some(0),
            volume: 100.0,
            shuffle: false,
            repeat: Repeat::Off,
            eq: None,
            resume: Vec::new(),
            groups: Vec::new(),
        };
        write_snapshot(dir.path(), &saved).expect("save");
        let mut session = Session::new().expect("session");
        session.media_store_start(dir.path().to_owned());
        session
            .media
            .queue
            .enqueue(VfsUri::parse(edited.to_str().expect("utf8")).expect("uri"))
            .expect("enqueue");
        wait(&mut session);
        assert_eq!(
            session.media.queue.items()[0].file_name(),
            Some("edited.mp3")
        );
        assert!(
            session
                .messages
                .recent()
                .any(|message| message.text.contains("changed during startup"))
        );
    }

    #[test]
    fn rejects_credentials_and_invalid_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let saved = Snapshot {
            paths: vec!["sftp://alice:secret@example.com/song.mp3".into()],
            index: Some(0),
            volume: 100.0,
            shuffle: false,
            repeat: Repeat::Off,
            eq: None,
            resume: Vec::new(),
            groups: Vec::new(),
        };
        assert!(write_snapshot(dir.path(), &saved).is_err());
        assert!(!dir.path().join(FILE).exists());
        std::fs::write(
            dir.path().join(FILE),
            serde_json::to_vec(&saved).expect("json"),
        )
        .expect("write");
        let error = read_snapshot(&dir.path().join(FILE)).expect_err("reject");
        assert!(!error.to_string().contains("secret"));
        let invalid = Snapshot {
            paths: vec![],
            index: Some(0),
            ..saved
        };
        std::fs::write(
            dir.path().join(FILE),
            serde_json::to_vec(&invalid).expect("json"),
        )
        .expect("write");
        assert!(read_snapshot(&dir.path().join(FILE)).is_err());
        let file = std::fs::File::create(dir.path().join(FILE)).expect("file");
        file.set_len(MAX_BYTES + 1).expect("sparse oversized file");
        assert!(
            read_snapshot(&dir.path().join(FILE))
                .expect_err("oversized")
                .to_string()
                .contains("too large")
        );
    }

    #[cfg(unix)]
    #[test]
    fn state_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("config");
        let saved = Snapshot {
            paths: vec![],
            index: None,
            volume: 100.0,
            shuffle: false,
            repeat: Repeat::Off,
            eq: None,
            resume: Vec::new(),
            groups: Vec::new(),
        };
        write_snapshot(&dir, &saved).expect("save");
        assert_eq!(
            std::fs::metadata(&dir).expect("dir").permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(dir.join(FILE))
                .expect("file")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
