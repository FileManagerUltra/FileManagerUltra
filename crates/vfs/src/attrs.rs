//! Full file attributes — pure data (capability `file-attributes`).
//!
//! [`Entry`](crate::Entry) stays cheap for listings; everything a sync or a faithful copy
//! needs (all three times, owner, ACL, xattrs, DOS flags) travels as [`Attrs`], fetched with
//! [`Vfs::stat_full`](crate::Vfs::stat_full) and applied with
//! [`Vfs::set_attrs`](crate::Vfs::set_attrs). Values read from a backend are untrusted:
//! every collection is capped here so a hostile server cannot make us allocate.

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

/// Most extended attributes kept per item; the rest are skipped and reported.
pub const MAX_XATTRS: usize = 256;
/// Longest xattr name accepted.
pub const MAX_XATTR_NAME: usize = 255;
/// Largest single xattr value accepted.
pub const MAX_XATTR_VALUE: usize = 64 * 1024;
/// Largest ACL / security-descriptor blob accepted.
pub const MAX_ACL_BYTES: usize = 64 * 1024;
/// Longest owner or group name accepted.
pub const MAX_OWNER_NAME: usize = 256;

/// One independently preservable attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttrGroup {
    /// Creation (birth) time.
    Btime,
    /// Modification time.
    Mtime,
    /// Access time.
    Atime,
    /// Unix permission bits.
    Mode,
    /// Owner and group (uid/gid and names).
    Owner,
    /// Access control list.
    Acl,
    /// Extended attributes.
    Xattrs,
    /// DOS flags (read-only, hidden, system, archive).
    Flags,
}

impl AttrGroup {
    pub const ALL: [AttrGroup; 8] = [
        Self::Btime,
        Self::Mtime,
        Self::Atime,
        Self::Mode,
        Self::Owner,
        Self::Acl,
        Self::Xattrs,
        Self::Flags,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }

    /// Short name for reports and logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::Btime => "creation time",
            Self::Mtime => "modification time",
            Self::Atime => "access time",
            Self::Mode => "permissions",
            Self::Owner => "owner/group",
            Self::Acl => "ACL",
            Self::Xattrs => "extended attributes",
            Self::Flags => "DOS flags",
        }
    }
}

/// A set of [`AttrGroup`]s (what to apply, or what a backend supports).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AttrSet(u8);

impl AttrSet {
    pub const NONE: AttrSet = AttrSet(0);
    pub const ALL: AttrSet = AttrSet(0xFF);
    /// Creation, modification and access time.
    pub const TIMES: AttrSet =
        AttrSet(AttrGroup::Btime.bit() | AttrGroup::Mtime.bit() | AttrGroup::Atime.bit());

    pub const fn of(groups: &[AttrGroup]) -> Self {
        let mut bits = 0;
        let mut rest = groups;
        while let [group, tail @ ..] = rest {
            bits |= group.bit();
            rest = tail;
        }
        Self(bits)
    }

    pub const fn contains(self, g: AttrGroup) -> bool {
        self.0 & g.bit() != 0
    }

    pub const fn union(self, other: AttrSet) -> AttrSet {
        AttrSet(self.0 | other.0)
    }

    pub const fn intersect(self, other: AttrSet) -> AttrSet {
        AttrSet(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = AttrGroup> {
        AttrGroup::ALL
            .into_iter()
            .filter(move |g| self.contains(*g))
    }
}

impl fmt::Debug for AttrSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl FromIterator<AttrGroup> for AttrSet {
    fn from_iter<T: IntoIterator<Item = AttrGroup>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Self::NONE, |s, g| s.union(Self::of(&[g])))
    }
}

/// What a backend can read and write at a location.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttrCaps {
    pub read: AttrSet,
    pub write: AttrSet,
}

impl AttrCaps {
    /// Nothing supported (the default for a backend without attribute support).
    pub const NONE: AttrCaps = AttrCaps {
        read: AttrSet::NONE,
        write: AttrSet::NONE,
    };
}

/// A format-tagged access control list. Blobs are opaque: they only round-trip between
/// locations of the same family, never translated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Acl {
    /// POSIX ACL xattr blobs (`system.posix_acl_access` / `system.posix_acl_default`).
    Posix {
        access: Option<Vec<u8>>,
        default: Option<Vec<u8>>,
    },
    /// Windows self-relative security descriptor (owner, group, DACL).
    WindowsSd(Vec<u8>),
}

impl Acl {
    /// Byte size of the carried blobs.
    pub fn len(&self) -> usize {
        match self {
            Self::Posix { access, default } => {
                access.as_ref().map_or(0, Vec::len) + default.as_ref().map_or(0, Vec::len)
            }
            Self::WindowsSd(b) => b.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Every attribute of one item. All fields are optional: `None` means "unknown / not
/// supported here", which is different from "absent".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attrs {
    pub btime: Option<SystemTime>,
    pub mtime: Option<SystemTime>,
    pub atime: Option<SystemTime>,
    /// Unix permission bits (`mode & 0o7777`).
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub acl: Option<Acl>,
    pub xattrs: BTreeMap<String, Vec<u8>>,
    /// Backend-defined DOS flag bits (`FILE_ATTRIBUTE_*` subset).
    pub dos_flags: Option<u32>,
    /// Attributes that were read but dropped (over a cap, hostile name), for the report.
    pub skipped: Vec<String>,
}

impl Attrs {
    /// Store an xattr unless it breaks a cap; a refused one is noted in `skipped`.
    pub fn insert_xattr(&mut self, name: &str, value: Vec<u8>) -> bool {
        let reason = if name.is_empty() || name.len() > MAX_XATTR_NAME || name.contains('\0') {
            Some("invalid name")
        } else if value.len() > MAX_XATTR_VALUE {
            Some("value too large")
        } else if self.xattrs.len() >= MAX_XATTRS && !self.xattrs.contains_key(name) {
            Some("too many attributes")
        } else {
            None
        };
        match reason {
            Some(why) => {
                if self.skipped.len() < MAX_XATTRS {
                    self.skipped
                        .push(format!("xattr {}: {why}", name.escape_debug()));
                }
                false
            }
            None => {
                self.xattrs.insert(name.to_string(), value);
                true
            }
        }
    }

    /// Store an ACL unless it is over the size cap (noted in `skipped`).
    pub fn set_acl(&mut self, acl: Acl) -> bool {
        if acl.len() > MAX_ACL_BYTES {
            self.skipped.push("ACL: too large".to_string());
            return false;
        }
        self.acl = Some(acl);
        true
    }

    /// Store an owner/group name unless it is oversized or contains NUL.
    pub fn bounded_name(name: &str) -> Option<String> {
        (!name.is_empty() && name.len() <= MAX_OWNER_NAME && !name.contains('\0'))
            .then(|| name.to_string())
    }

    /// The groups this value actually carries data for.
    pub fn present(&self) -> AttrSet {
        let mut s = AttrSet::NONE;
        let mut add = |cond: bool, g: AttrGroup| {
            if cond {
                s = s.union(AttrSet::of(&[g]));
            }
        };
        add(self.btime.is_some(), AttrGroup::Btime);
        add(self.mtime.is_some(), AttrGroup::Mtime);
        add(self.atime.is_some(), AttrGroup::Atime);
        add(self.mode.is_some(), AttrGroup::Mode);
        add(
            self.uid.is_some() || self.gid.is_some() || self.owner.is_some(),
            AttrGroup::Owner,
        );
        add(self.acl.is_some(), AttrGroup::Acl);
        add(!self.xattrs.is_empty(), AttrGroup::Xattrs);
        add(self.dos_flags.is_some(), AttrGroup::Flags);
        s
    }
}

/// Why one attribute was not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrFailKind {
    /// The backend (or this platform) cannot represent it.
    Unsupported,
    /// The OS or server refused (missing privilege, read-only share).
    PermissionDenied,
    /// Anything else, with a message safe to show.
    Other(String),
}

/// One attribute that was requested but not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrFailure {
    pub group: AttrGroup,
    pub kind: AttrFailKind,
}

impl fmt::Display for AttrFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            AttrFailKind::Unsupported => write!(f, "{}: not supported here", self.group.label()),
            AttrFailKind::PermissionDenied => {
                write!(f, "{}: permission denied", self.group.label())
            }
            AttrFailKind::Other(m) => write!(f, "{}: {m}", self.group.label()),
        }
    }
}

/// Outcome of [`Vfs::set_attrs`](crate::Vfs::set_attrs): best effort, never all-or-nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttrReport {
    pub applied: AttrSet,
    pub failed: Vec<AttrFailure>,
}

impl AttrReport {
    pub fn ok(&mut self, g: AttrGroup) {
        self.applied = self.applied.union(AttrSet::of(&[g]));
    }

    pub fn fail(&mut self, group: AttrGroup, kind: AttrFailKind) {
        self.failed.push(AttrFailure { group, kind });
    }

    /// True when everything requested was applied.
    pub fn is_complete(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn failed_group(&self, g: AttrGroup) -> Option<&AttrFailure> {
        self.failed.iter().find(|f| f.group == g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_operations() {
        let s = AttrSet::of(&[AttrGroup::Mode, AttrGroup::Mtime]);
        assert!(s.contains(AttrGroup::Mode));
        assert!(!s.contains(AttrGroup::Owner));
        assert_eq!(
            s.intersect(AttrSet::TIMES),
            AttrSet::of(&[AttrGroup::Mtime])
        );
        assert_eq!(s.iter().count(), 2);
        assert!(AttrSet::NONE.is_empty());
        assert_eq!(AttrSet::ALL.iter().count(), AttrGroup::ALL.len());
    }

    #[test]
    fn hostile_xattrs_are_skipped_and_reported() {
        let mut a = Attrs::default();
        assert!(a.insert_xattr("user.ok", vec![1]));
        assert!(!a.insert_xattr("bad\0name", vec![1]));
        assert!(!a.insert_xattr("", vec![1]));
        assert!(!a.insert_xattr(&"x".repeat(MAX_XATTR_NAME + 1), vec![1]));
        assert!(!a.insert_xattr("user.big", vec![0; MAX_XATTR_VALUE + 1]));
        assert_eq!(a.xattrs.len(), 1);
        assert_eq!(a.skipped.len(), 4);
    }

    #[test]
    fn xattr_count_is_capped() {
        let mut a = Attrs::default();
        for i in 0..MAX_XATTRS + 10 {
            a.insert_xattr(&format!("user.k{i}"), vec![]);
        }
        assert_eq!(a.xattrs.len(), MAX_XATTRS);
        // Replacing an existing name is still allowed at the cap.
        assert!(a.insert_xattr("user.k0", vec![9]));
    }

    #[test]
    fn oversized_acl_is_refused() {
        let mut a = Attrs::default();
        assert!(!a.set_acl(Acl::WindowsSd(vec![0; MAX_ACL_BYTES + 1])));
        assert!(a.acl.is_none());
        assert!(a.set_acl(Acl::Posix {
            access: Some(vec![1; 8]),
            default: None
        }));
    }

    #[test]
    fn present_reports_filled_groups() {
        let a = Attrs {
            mode: Some(0o644),
            mtime: Some(SystemTime::UNIX_EPOCH),
            ..Default::default()
        };
        assert_eq!(
            a.present(),
            AttrSet::of(&[AttrGroup::Mode, AttrGroup::Mtime])
        );
    }

    #[test]
    fn bounded_name_rejects_nul_and_length() {
        assert_eq!(Attrs::bounded_name("alice").as_deref(), Some("alice"));
        assert!(Attrs::bounded_name("a\0b").is_none());
        assert!(Attrs::bounded_name(&"n".repeat(MAX_OWNER_NAME + 1)).is_none());
    }
}
