//! Last-sync snapshot for two-way sync: per path, what each side looked like when the two
//! were last known equal. One versioned JSON file per profile, written atomically and
//! owner-only. A corrupt, foreign or newer-version file is treated as missing (reported),
//! which makes the next run a merge that deletes nothing.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fm_vfs::FmError;
use serde::{Deserialize, Serialize};

use crate::scan::{Node, NodeKind};

pub const SNAPSHOT_VERSION: u32 = 1;
/// Larger files are refused instead of being read into memory.
const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024 * 1024;

/// What one side looked like at the last sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub kind: NodeKind,
    pub size: u64,
    pub mtime_ms: Option<i64>,
}

impl Fingerprint {
    pub fn of(node: &Node) -> Self {
        Self {
            kind: node.kind,
            size: node.size,
            mtime_ms: node.mtime.map(to_ms),
        }
    }

    /// Does `node` still look like this? Times within `tolerance` count as equal.
    pub fn matches(&self, node: &Node, tolerance: Duration) -> bool {
        if self.kind != node.kind {
            return false;
        }
        match node.kind {
            NodeKind::Dir => true,
            NodeKind::Symlink => self.size == node.size,
            NodeKind::File => {
                self.size == node.size
                    && match (self.mtime_ms, node.mtime.map(to_ms)) {
                        (Some(a), Some(b)) => a.abs_diff(b) <= tolerance.as_millis() as u64,
                        (None, None) => true,
                        _ => false,
                    }
            }
        }
    }
}

fn to_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub left: Fingerprint,
    pub right: Fingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub left: String,
    pub right: String,
    pub entries: BTreeMap<String, Pair>,
}

impl Snapshot {
    pub fn new(left: &str, right: &str) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            left: left.to_string(),
            right: right.to_string(),
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub enum Loaded {
    Missing,
    Found(Snapshot),
    /// Present but unusable; the reason is for the user. Treat as missing.
    Invalid(String),
}

/// Snapshot files in one directory.
#[derive(Clone, Debug)]
pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, id: &str) -> Result<PathBuf, FmError> {
        let ok = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(FmError::InvalidParams("invalid profile id".into()));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    /// `left`/`right` must match what the snapshot was made for: a profile pointed at other
    /// folders must not reuse another pair's history.
    pub fn load(&self, id: &str, left: &str, right: &str) -> Result<Loaded, FmError> {
        let path = self.path(id)?;
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::Missing),
            Err(e) => return Err(FmError::Io(e)),
        };
        if meta.len() > MAX_SNAPSHOT_BYTES {
            return Ok(Loaded::Invalid("the snapshot file is too large".into()));
        }
        let bytes = std::fs::read(&path).map_err(FmError::Io)?;
        let snap: Snapshot = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(_) => return Ok(Loaded::Invalid("the snapshot file is corrupt".into())),
        };
        if snap.version != SNAPSHOT_VERSION {
            return Ok(Loaded::Invalid(format!(
                "the snapshot has an unknown version ({})",
                snap.version
            )));
        }
        if snap.left != left || snap.right != right {
            return Ok(Loaded::Invalid(
                "the snapshot belongs to different folders".into(),
            ));
        }
        Ok(Loaded::Found(snap))
    }

    /// Atomic: write a temp file next to the target, then rename it over.
    pub fn save(&self, id: &str, snap: &Snapshot) -> Result<(), FmError> {
        let path = self.path(id)?;
        create_private_dir(&self.dir)?;
        let bytes = serde_json::to_vec(snap)
            .map_err(|e| FmError::backend("sync", format!("snapshot encode: {e}")))?;
        let tmp = self.dir.join(format!("{id}.json.tmp"));
        write_private(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            FmError::Io(e)
        })
    }

    pub fn delete(&self, id: &str) -> Result<(), FmError> {
        match std::fs::remove_file(self.path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(FmError::Io(e)),
        }
    }
}

fn create_private_dir(dir: &Path) -> Result<(), FmError> {
    std::fs::create_dir_all(dir).map_err(FmError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(FmError::Io)?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), FmError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(FmError::Io)?;
    f.write_all(bytes).map_err(FmError::Io)?;
    f.sync_all().map_err(FmError::Io)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    fn snap() -> Snapshot {
        let mut s = Snapshot::new("/a", "/b");
        let fp = Fingerprint {
            kind: NodeKind::File,
            size: 3,
            mtime_ms: Some(1_000),
        };
        s.entries.insert(
            "x.txt".into(),
            Pair {
                left: fp.clone(),
                right: fp,
            },
        );
        s
    }

    #[test]
    fn round_trips_and_is_owner_only() {
        let d = tempfile::tempdir().expect("tmp");
        let store = SnapshotStore::new(d.path().join("state"));
        assert!(matches!(store.load("p1", "/a", "/b"), Ok(Loaded::Missing)));
        store.save("p1", &snap()).expect("save");
        let Ok(Loaded::Found(back)) = store.load("p1", "/a", "/b") else {
            panic!("expected a snapshot");
        };
        assert_eq!(back, snap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&d.path().join("state")), 0o700);
            assert_eq!(mode(&d.path().join("state/p1.json")), 0o600);
        }
    }

    #[test]
    fn corrupt_foreign_or_newer_snapshots_are_treated_as_missing() {
        let d = tempfile::tempdir().expect("tmp");
        let store = SnapshotStore::new(d.path());
        std::fs::write(d.path().join("p1.json"), b"{not json").expect("write");
        assert!(matches!(
            store.load("p1", "/a", "/b"),
            Ok(Loaded::Invalid(_))
        ));
        let mut s = snap();
        s.version = 99;
        store.save("p1", &s).expect("save");
        assert!(matches!(
            store.load("p1", "/a", "/b"),
            Ok(Loaded::Invalid(_))
        ));
        store.save("p1", &snap()).expect("save");
        assert!(matches!(
            store.load("p1", "/a", "/other"),
            Ok(Loaded::Invalid(_))
        ));
    }

    #[test]
    fn hostile_ids_never_reach_the_filesystem() {
        let store = SnapshotStore::new("/nonexistent-state-dir");
        for id in ["", "../x", "a/b", "a.json", &"x".repeat(65)] {
            assert!(store.load(id, "/a", "/b").is_err(), "{id}");
            assert!(store.save(id, &snap()).is_err(), "{id}");
        }
    }

    #[test]
    fn fingerprints_use_the_tolerance() {
        let node = |ms: u64| Node {
            kind: NodeKind::File,
            size: 3,
            mtime: Some(UNIX_EPOCH + Duration::from_millis(ms)),
            link: None,
        };
        let fp = Fingerprint::of(&node(10_000));
        assert!(fp.matches(&node(11_500), Duration::from_secs(2)));
        assert!(!fp.matches(&node(13_000), Duration::from_secs(2)));
    }
}
