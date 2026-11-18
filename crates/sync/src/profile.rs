//! Sync profile: everything a saved pair needs, with serde defaults so older or partial
//! files still load, and validation that runs before a profile is stored or used.

use std::collections::BTreeSet;

use fm_vfs::{AttrGroup, AttrSet, FmError, Scheme, VfsUri};
use serde::{Deserialize, Serialize};

use crate::glob::Glob;

/// Longest profile name, in bytes.
pub const MAX_NAME_BYTES: usize = 120;
/// Most filter globs per list.
pub const MAX_FILTERS: usize = 256;
/// Smallest remote polling interval, in seconds.
pub const MIN_POLL_SECS: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    /// Target becomes identical to the source; extras are deleted per [`DeletePolicy`].
    Mirror,
    /// Source is copied when new or newer; nothing is deleted.
    #[default]
    Update,
    /// Changes propagate both ways, judged against the last-sync snapshot.
    TwoWay,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareMethod {
    /// Size and modification time only; no content is read.
    #[default]
    SizeMtime,
    /// As above, plus a checksum when sizes match but times differ.
    Checksum,
    /// Checksum whenever sizes match (slow, most reliable).
    ChecksumAlways,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymlinkPolicy {
    /// Recreate the link; never follow it.
    #[default]
    Preserve,
    /// Descend into links to folders (cycle-guarded).
    Follow,
    Skip,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnAttrFailure {
    /// Copy the file, report what was not preserved.
    #[default]
    Warn,
    /// Count the file as failed (it is not recorded as synced).
    Error,
    Ignore,
}

/// What happens to a conflicting item. Only `Skip` exists for now; rules come later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictRule {
    #[default]
    Skip,
}

/// How long archived versions are kept. A value of zero or `None` disables that limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    pub max_versions: Option<u32>,
    pub max_age_days: Option<u32>,
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletePolicy {
    Permanent,
    /// Platform trash; local targets only.
    Trash,
    /// Move into `folder/<timestamp>/<relative path>` on the target's backend.
    Versioning {
        folder: String,
        retention: Retention,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    /// When non-empty a file must match one of these. Folders are always entered.
    pub include: Vec<String>,
    /// Excludes win over includes.
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchSettings {
    pub enabled: bool,
    /// Poll interval for remote sides (at least [`MIN_POLL_SECS`]).
    pub poll_secs: u32,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_secs: 30,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Stable id (file name of the snapshot); never shown.
    pub id: String,
    pub name: String,
    pub left: String,
    pub right: String,
    pub mode: SyncMode,
    /// One-way modes copy from this side.
    pub source: Side,
    pub compare: CompareMethod,
    /// Modification times closer than this are equal (FAT/SMB/SFTP rounding).
    pub tolerance_secs: u32,
    pub filters: Filters,
    pub symlinks: SymlinkPolicy,
    #[serde(with = "attr_set_serde")]
    pub attrs: AttrSet,
    pub on_attr_failure: OnAttrFailure,
    /// `None` means the default for the target (Trash locally, Permanent remotely).
    pub delete: Option<DeletePolicy>,
    pub max_delete_percent: u8,
    pub conflicts: ConflictRule,
    pub watch: WatchSettings,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            left: String::new(),
            right: String::new(),
            mode: SyncMode::default(),
            source: Side::Left,
            compare: CompareMethod::default(),
            tolerance_secs: 2,
            filters: Filters::default(),
            symlinks: SymlinkPolicy::default(),
            attrs: AttrSet::ALL,
            on_attr_failure: OnAttrFailure::default(),
            delete: None,
            max_delete_percent: 50,
            conflicts: ConflictRule::default(),
            watch: WatchSettings::default(),
        }
    }
}

impl Profile {
    pub fn left_uri(&self) -> Result<VfsUri, FmError> {
        parse_endpoint(&self.left)
    }

    pub fn right_uri(&self) -> Result<VfsUri, FmError> {
        parse_endpoint(&self.right)
    }

    pub fn uri(&self, side: Side) -> Result<VfsUri, FmError> {
        match side {
            Side::Left => self.left_uri(),
            Side::Right => self.right_uri(),
        }
    }

    /// Where deletes land: for one-way modes the target, for two-way both sides.
    fn delete_targets(&self) -> Vec<Side> {
        match self.mode {
            SyncMode::TwoWay => vec![Side::Left, Side::Right],
            _ => vec![self.source.other()],
        }
    }

    /// The policy in force for `side`: the configured one or the default for its scheme.
    pub fn delete_policy(&self, side: Side) -> DeletePolicy {
        if let Some(p) = &self.delete {
            return p.clone();
        }
        let local = self
            .uri(side)
            .map(|u| u.scheme() == Scheme::File)
            .unwrap_or(true);
        if local {
            DeletePolicy::Trash
        } else {
            DeletePolicy::Permanent
        }
    }

    /// Reject anything that must not be stored or run. Messages never echo credentials.
    pub fn validate(&self) -> Result<(), FmError> {
        let bad = |m: &str| Err(FmError::InvalidParams(m.to_string()));
        if self.name.trim().is_empty() || self.name.len() > MAX_NAME_BYTES {
            return bad("the profile needs a name of at most 120 bytes");
        }
        if self.name.chars().any(char::is_control) {
            return bad("the profile name contains control characters");
        }
        if !valid_id(&self.id) {
            return bad("the profile id is invalid");
        }
        let (l, r) = (self.left_uri()?, self.right_uri()?);
        if l == r || l.is_same_or_descendant_of(&r) || r.is_same_or_descendant_of(&l) {
            return bad("the two folders are the same or one contains the other");
        }
        if self.max_delete_percent > 100 {
            return bad("max_delete_percent must be between 0 and 100");
        }
        if self.tolerance_secs > 3 * 3600 + 1 {
            return bad("the time tolerance is too large");
        }
        if self.watch.poll_secs < MIN_POLL_SECS || self.watch.poll_secs > 24 * 3600 {
            return bad("the polling interval must be between 10 seconds and 24 hours");
        }
        for list in [&self.filters.include, &self.filters.exclude] {
            if list.len() > MAX_FILTERS {
                return bad("too many filters");
            }
            for g in list {
                Glob::compile(g)?;
            }
        }
        for side in self.delete_targets() {
            let target = self.uri(side)?;
            match self.delete_policy(side) {
                DeletePolicy::Trash if target.scheme() != Scheme::File => {
                    return bad(
                        "Trash only works for local folders; use Permanent or Versioning for SMB/SFTP",
                    );
                }
                DeletePolicy::Versioning { folder, .. } => {
                    let f = parse_endpoint(&folder)?;
                    if f.scheme() != target.scheme() || f.authority() != target.authority() {
                        return bad(
                            "the versioning folder must be on the same server as the target",
                        );
                    }
                    if target.is_same_or_descendant_of(&f) || f.is_same_or_descendant_of(&target) {
                        // The archive inside the target would be scanned and mirrored.
                        return bad("the versioning folder must not overlap the target folder");
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Parse a profile endpoint, refusing embedded credentials without echoing them.
pub fn parse_endpoint(s: &str) -> Result<VfsUri, FmError> {
    if let Some((_, rest)) = s.split_once("://") {
        let authority = rest.split('/').next().unwrap_or("");
        if authority.contains('@') && authority.split('@').next().is_some_and(|u| u.contains(':')) {
            return Err(FmError::InvalidParams(
                "a profile folder must not contain a password; connect to the server first"
                    .to_string(),
            ));
        }
    }
    VfsUri::parse(s)
}

/// Distinct ids for a set of profiles: `p1`, `p2`, ... first unused.
pub fn fresh_id(existing: &BTreeSet<String>) -> String {
    (1u32..)
        .map(|n| format!("p{n}"))
        .find(|id| !existing.contains(id))
        .unwrap_or_else(|| "p0".to_string())
}

mod attr_set_serde {
    use super::{AttrGroup, AttrSet};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(set: &AttrSet, s: S) -> Result<S::Ok, S::Error> {
        let names: Vec<&str> = set.iter().map(key).collect();
        names.serialize(s)
    }

    /// Stable on-disk names (the display labels may change).
    fn key(g: AttrGroup) -> &'static str {
        match g {
            AttrGroup::Btime => "btime",
            AttrGroup::Mtime => "mtime",
            AttrGroup::Atime => "atime",
            AttrGroup::Mode => "mode",
            AttrGroup::Owner => "owner",
            AttrGroup::Acl => "acl",
            AttrGroup::Xattrs => "xattrs",
            AttrGroup::Flags => "flags",
        }
    }

    /// Unknown names are ignored so a newer file still loads.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<AttrSet, D::Error> {
        let names = Vec::<String>::deserialize(d)?;
        let groups: Vec<AttrGroup> = AttrGroup::ALL
            .into_iter()
            .filter(|g| names.iter().any(|n| n == key(*g)))
            .collect();
        Ok(AttrSet::of(&groups))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Profile {
        Profile {
            id: "p1".into(),
            name: "Docs".into(),
            left: "/tmp/a".into(),
            right: "/tmp/b".into(),
            ..Profile::default()
        }
    }

    #[test]
    fn a_default_pair_is_valid() {
        base().validate().expect("valid");
    }

    #[test]
    fn nested_and_identical_folders_are_rejected() {
        let mut p = base();
        p.right = "/tmp/a/inner".into();
        assert!(p.validate().is_err());
        p.right = "/tmp".into();
        assert!(p.validate().is_err());
        p.right = "/tmp/a".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn passwords_in_uris_are_refused_without_being_echoed() {
        let mut p = base();
        p.right = "sftp://u:hunter2@host/dir".into();
        let e = p.validate().expect_err("rejected").to_string();
        assert!(!e.contains("hunter2"), "{e}");
    }

    #[test]
    fn trash_on_a_remote_target_is_invalid() {
        let mut p = base();
        p.right = "sftp://host/dir".into();
        p.delete = Some(DeletePolicy::Trash);
        assert!(p.validate().is_err());
        p.delete = Some(DeletePolicy::Permanent);
        p.validate().expect("permanent is fine");
        p.delete = None;
        assert_eq!(p.delete_policy(Side::Right), DeletePolicy::Permanent);
        assert_eq!(p.delete_policy(Side::Left), DeletePolicy::Trash);
    }

    #[test]
    fn versioning_must_live_on_the_target_server_outside_the_target() {
        let mut p = base();
        p.right = "sftp://host/dir".into();
        p.delete = Some(DeletePolicy::Versioning {
            folder: "sftp://host/versions".into(),
            retention: Retention::default(),
        });
        p.validate().expect("ok");
        p.delete = Some(DeletePolicy::Versioning {
            folder: "sftp://host/dir/versions".into(),
            retention: Retention::default(),
        });
        assert!(p.validate().is_err());
        p.delete = Some(DeletePolicy::Versioning {
            folder: "/tmp/versions".into(),
            retention: Retention::default(),
        });
        assert!(p.validate().is_err());
    }

    #[test]
    fn bad_limits_are_rejected() {
        let mut p = base();
        p.watch.poll_secs = 1;
        assert!(p.validate().is_err());
        let mut p = base();
        p.max_delete_percent = 101;
        assert!(p.validate().is_err());
        let mut p = base();
        p.id = "../x".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn missing_and_unknown_fields_load_with_defaults() {
        let p: Profile =
            serde_json::from_str(r#"{"id":"p1","name":"n","future":1,"attrs":["mtime","bogus"]}"#)
                .expect("loads");
        assert_eq!(p.tolerance_secs, 2);
        assert_eq!(p.attrs, AttrSet::of(&[AttrGroup::Mtime]));
        let json = serde_json::to_string(&base()).expect("ser");
        let back: Profile = serde_json::from_str(&json).expect("de");
        assert_eq!(back, base());
    }
}
