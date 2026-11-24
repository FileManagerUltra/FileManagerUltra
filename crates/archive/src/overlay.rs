//! Staged changes to one mounted archive (design D6): new and replaced files (their bytes
//! spooled to a private temporary folder), new folders, and deletions recorded as markers
//! that hide what the archive holds. A rename is "add at the new place, pointing at the
//! same data, and mark the old place deleted", so it costs no copying. The archive file is
//! not touched until Save; Discard drops the overlay.
//!
//! Paths are archive-relative, `/`-separated, without a leading slash (`""` is the root).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use fm_vfs::FmError;

use crate::index::{Index, NodeId, NodeKind};
use crate::spool::SpoolDir;

/// Most bytes staged for one archive before Save is needed.
pub const MAX_STAGED: u64 = 8 << 30;

/// Where a staged file's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Data {
    /// A member of the archive as mounted (moved or renamed, not rewritten).
    Base(NodeId),
    /// A file in the overlay's private folder.
    Spool(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OvEntry {
    File {
        data: Data,
        size: u64,
        modified: Option<SystemTime>,
    },
    /// A folder made in the overlay. `opaque`: it replaces a deleted item of the same
    /// name, so nothing the archive held below that name shows through.
    Dir {
        opaque: bool,
        modified: Option<SystemTime>,
    },
    /// The archive's item at this path (and everything below it) is gone.
    Deleted,
}

/// What a path shows with the overlay applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Base(NodeId),
    Over(OvEntry),
}

/// One line of `archive.changes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// `added`, `replaced`, `folder` or `deleted`.
    pub kind: &'static str,
}

/// A password change applied on Save: every member is encrypted with `new`, or stored
/// without encryption when it is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordChange {
    pub new: Option<crate::Secret>,
    /// 7z: hide the file names too (asks for the password before listing).
    pub encrypt_names: bool,
}

#[derive(Default)]
pub struct Overlay {
    entries: BTreeMap<String, OvEntry>,
    spool: Option<SpoolDir>,
    staged: u64,
    seq: u64,
    password: Option<PasswordChange>,
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn exists(e: &str) -> FmError {
    FmError::AlreadyExists(e.to_string())
}

fn missing(e: &str) -> FmError {
    FmError::NotFound(e.to_string())
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.password.is_none()
    }

    /// Drop every staged change (Discard) and the spooled bytes.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.spool = None;
        self.staged = 0;
        self.password = None;
    }

    pub fn password_change(&self) -> Option<&PasswordChange> {
        self.password.as_ref()
    }

    pub fn set_password_change(&mut self, change: Option<PasswordChange>) {
        self.password = change;
    }

    /// Whether a proper ancestor of `path` hides the archive's items below it.
    fn base_hidden_by_ancestor(&self, path: &str) -> bool {
        let mut at = path;
        while !at.is_empty() {
            at = parent_of(at);
            if at.is_empty() {
                break;
            }
            if matches!(
                self.entries.get(at),
                Some(OvEntry::Deleted | OvEntry::Dir { opaque: true, .. })
            ) {
                return true;
            }
        }
        false
    }

    /// The archive's own node at `path`, if nothing above it hides it (its own overlay
    /// entry is not considered).
    fn base_at(&self, index: &Index, path: &str) -> Option<NodeId> {
        if self.base_hidden_by_ancestor(path) {
            return None;
        }
        index.lookup(path)
    }

    pub fn resolve(&self, index: &Index, path: &str) -> Option<Resolved> {
        match self.entries.get(path) {
            Some(OvEntry::Deleted) => None,
            Some(e) => Some(Resolved::Over(e.clone())),
            None => self.base_at(index, path).map(Resolved::Base),
        }
    }

    fn is_dir(index: &Index, r: &Resolved) -> bool {
        match r {
            Resolved::Base(id) => index.node(*id).is_some_and(|n| n.kind == NodeKind::Dir),
            Resolved::Over(OvEntry::Dir { .. }) => true,
            Resolved::Over(_) => false,
        }
    }

    /// The children of the folder at `dir` with the overlay applied, as `(name, what)`.
    pub fn children(&self, index: &Index, dir: &str) -> Result<Vec<(String, Resolved)>, FmError> {
        let here = self.resolve(index, dir).ok_or_else(|| missing(dir))?;
        if !Self::is_dir(index, &here) {
            return Err(FmError::NotADirectory(dir.to_string()));
        }
        let mut out = Vec::new();
        if let Resolved::Base(id) = here {
            for &child in index.children(id) {
                let Some(node) = index.node(child) else {
                    continue;
                };
                let path = join(dir, &node.name);
                if !self.entries.contains_key(&path) {
                    out.push((node.name.clone(), Resolved::Base(child)));
                }
            }
        }
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        for (path, e) in self.entries.range(prefix.clone()..) {
            let Some(rest) = path.strip_prefix(&prefix) else {
                break;
            };
            if rest.is_empty() || rest.contains('/') || *e == OvEntry::Deleted {
                continue;
            }
            out.push((rest.to_string(), Resolved::Over(e.clone())));
        }
        Ok(out)
    }

    fn need_dir(&self, index: &Index, path: &str) -> Result<(), FmError> {
        let parent = parent_of(path);
        match self.resolve(index, parent) {
            Some(r) if Self::is_dir(index, &r) => Ok(()),
            Some(_) => Err(FmError::NotADirectory(parent.to_string())),
            None => Err(missing(parent)),
        }
    }

    /// A new folder at `path`.
    pub fn create_dir(&mut self, index: &Index, path: &str) -> Result<(), FmError> {
        if path.is_empty() {
            return Err(exists("/"));
        }
        self.need_dir(index, path)?;
        if self.resolve(index, path).is_some() {
            return Err(exists(path));
        }
        let opaque = self.entries.get(path) == Some(&OvEntry::Deleted);
        self.entries.insert(
            path.to_string(),
            OvEntry::Dir {
                opaque,
                modified: Some(SystemTime::now()),
            },
        );
        Ok(())
    }

    /// A private file to write a new member into, and its name in the spool folder.
    pub fn spool_file(&mut self) -> Result<PathBuf, FmError> {
        if self.spool.is_none() {
            self.spool = Some(SpoolDir::create("fm-stage")?);
        }
        self.seq += 1;
        let dir = self
            .spool
            .as_ref()
            .map(|s| s.path().to_path_buf())
            .ok_or_else(|| missing("spool"))?;
        Ok(dir.join(format!("{}", self.seq)))
    }

    /// Whether `more` bytes still fit in the staging cap.
    pub fn reserve(&mut self, more: u64) -> Result<(), FmError> {
        let total = self.staged.saturating_add(more);
        if total > MAX_STAGED {
            return Err(FmError::backend(
                "archive",
                "the changes to this archive are larger than 8 GiB; save it first",
            ));
        }
        self.staged = total;
        Ok(())
    }

    /// Check that `path` can take a written file (its folder exists, it is not a folder).
    pub fn can_write(&self, index: &Index, path: &str) -> Result<(), FmError> {
        self.need_dir(index, path)?;
        match self.resolve(index, path) {
            Some(r) if Self::is_dir(index, &r) => Err(FmError::NotADirectory(path.to_string())),
            _ => Ok(()),
        }
    }

    /// The written file `spooled` (of `size` bytes) is now the member at `path`.
    pub fn put_file(
        &mut self,
        index: &Index,
        path: &str,
        spooled: PathBuf,
        size: u64,
    ) -> Result<(), FmError> {
        self.can_write(index, path)?;
        self.drop_spool_of(path);
        self.entries.insert(
            path.to_string(),
            OvEntry::File {
                data: Data::Spool(spooled),
                size,
                modified: Some(SystemTime::now()),
            },
        );
        Ok(())
    }

    fn drop_spool_of(&mut self, path: &str) {
        if let Some(OvEntry::File {
            data: Data::Spool(p),
            ..
        }) = self.entries.get(path)
        {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Remove the file or empty folder at `path`.
    pub fn remove(&mut self, index: &Index, path: &str) -> Result<(), FmError> {
        let r = self.resolve(index, path).ok_or_else(|| missing(path))?;
        if Self::is_dir(index, &r) && !self.children(index, path)?.is_empty() {
            return Err(FmError::NotEmpty(path.to_string()));
        }
        self.drop_spool_of(path);
        if self.base_at(index, path).is_some() {
            self.entries.insert(path.to_string(), OvEntry::Deleted);
        } else {
            self.entries.remove(path);
        }
        Ok(())
    }

    /// Move `from` (and everything below it) to `to`, which must not exist.
    pub fn rename(&mut self, index: &Index, from: &str, to: &str) -> Result<(), FmError> {
        if from.is_empty() || to.is_empty() {
            return Err(FmError::InvalidParams(
                "cannot rename the archive root".into(),
            ));
        }
        if to == from || to.starts_with(&format!("{from}/")) {
            return Err(FmError::InvalidParams(
                "cannot move a folder into itself".into(),
            ));
        }
        let top = self.resolve(index, from).ok_or_else(|| missing(from))?;
        if self.resolve(index, to).is_some() {
            return Err(exists(to));
        }
        self.need_dir(index, to)?;
        // Everything below `from`, as the merged view shows it, parents first.
        let mut moved: Vec<(String, Resolved)> = vec![(String::new(), top)];
        let mut i = 0;
        while let Some((rel, r)) = moved.get(i).cloned() {
            i += 1;
            if Self::is_dir(index, &r) {
                let here = if rel.is_empty() {
                    from.to_string()
                } else {
                    format!("{from}/{rel}")
                };
                for (name, child) in self.children(index, &here)? {
                    moved.push((join(&rel, &name), child));
                }
            }
        }
        let mut placed = Vec::with_capacity(moved.len());
        for (rel, r) in &moved {
            let dest = if rel.is_empty() {
                to.to_string()
            } else {
                format!("{to}/{rel}")
            };
            let entry = match r {
                Resolved::Base(id) => match index.node(*id) {
                    Some(n) if n.kind == NodeKind::Dir => OvEntry::Dir {
                        opaque: false,
                        modified: n.meta.modified,
                    },
                    Some(n) => OvEntry::File {
                        data: Data::Base(*id),
                        size: n.meta.size.unwrap_or(0),
                        modified: n.meta.modified,
                    },
                    None => continue,
                },
                Resolved::Over(e) => e.clone(),
            };
            let entry = match entry {
                // The new place may hide a deleted item of the same name.
                OvEntry::Dir { modified, .. } => OvEntry::Dir {
                    opaque: self.base_at(index, &dest).is_some()
                        || self.entries.get(&dest) == Some(&OvEntry::Deleted),
                    modified,
                },
                other => other,
            };
            placed.push((dest, entry));
        }
        // Take the old subtree out without deleting spooled bytes (they moved).
        let prefix = format!("{from}/");
        let stale: Vec<String> = self
            .entries
            .range(from.to_string()..)
            .take_while(|(k, _)| k.as_str() == from || k.starts_with(&prefix))
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            self.entries.remove(&k);
        }
        if self.base_at(index, from).is_some() {
            self.entries.insert(from.to_string(), OvEntry::Deleted);
        }
        for (dest, entry) in placed {
            self.entries.insert(dest, entry);
        }
        Ok(())
    }

    /// The staged changes, one per changed item that is not inside a new folder; a
    /// password change comes first, with an empty path.
    pub fn changes(&self, index: &Index) -> Vec<Change> {
        let mut out = Vec::new();
        if let Some(change) = &self.password {
            out.push(Change {
                path: String::new(),
                kind: if change.new.is_some() {
                    "password"
                } else {
                    "no password"
                },
            });
        }
        for (path, e) in &self.entries {
            // Items inside a folder added in the overlay are part of that change.
            let mut at = path.as_str();
            let mut inside_new = false;
            while !at.is_empty() {
                at = parent_of(at);
                if at.is_empty() {
                    break;
                }
                if matches!(self.entries.get(at), Some(OvEntry::Dir { .. }))
                    && index.lookup(at).is_none()
                {
                    inside_new = true;
                    break;
                }
            }
            if inside_new {
                continue;
            }
            let kind = match e {
                OvEntry::Deleted => "deleted",
                OvEntry::Dir { .. } => "folder",
                OvEntry::File { .. } if self.base_at(index, path).is_some() => "replaced",
                OvEntry::File { .. } => "added",
            };
            out.push(Change {
                path: path.clone(),
                kind,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Meta;

    fn base() -> Index {
        let mut idx = Index::new();
        for (p, k) in [
            ("www", NodeKind::Dir),
            ("www/index.html", NodeKind::File),
            ("www/img/logo.png", NodeKind::File),
            ("readme", NodeKind::File),
        ] {
            idx.add(
                p,
                k,
                Meta {
                    size: Some(3),
                    ..Meta::default()
                },
            )
            .expect("add");
        }
        idx
    }

    fn names(ov: &Overlay, idx: &Index, dir: &str) -> Vec<String> {
        let mut v: Vec<String> = ov
            .children(idx, dir)
            .expect("dir")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        v.sort();
        v
    }

    fn spooled(ov: &mut Overlay, bytes: &[u8]) -> PathBuf {
        let p = ov.spool_file().expect("spool");
        std::fs::write(&p, bytes).expect("write");
        p
    }

    #[test]
    fn adds_folders_and_files_over_the_archive() {
        let idx = base();
        let mut ov = Overlay::default();
        ov.create_dir(&idx, "docs").expect("mkdir");
        assert!(ov.create_dir(&idx, "www").is_err(), "exists");
        assert!(ov.create_dir(&idx, "nope/x").is_err(), "no parent");
        let p = spooled(&mut ov, b"pdf");
        ov.put_file(&idx, "docs/report.pdf", p, 3).expect("put");
        assert_eq!(names(&ov, &idx, ""), vec!["docs", "readme", "www"]);
        assert_eq!(names(&ov, &idx, "docs"), vec!["report.pdf"]);
        assert!(
            ov.put_file(&idx, "www", PathBuf::new(), 0).is_err(),
            "a folder is not a file"
        );
        assert_eq!(
            ov.changes(&idx),
            vec![Change {
                path: "docs".into(),
                kind: "folder"
            }]
        );
    }

    #[test]
    fn deleting_hides_and_recreating_is_opaque() {
        let idx = base();
        let mut ov = Overlay::default();
        assert!(matches!(ov.remove(&idx, "www"), Err(FmError::NotEmpty(_))));
        ov.remove(&idx, "www/img/logo.png").expect("rm");
        ov.remove(&idx, "www/img").expect("rm implicit folder");
        ov.remove(&idx, "www/index.html").expect("rm");
        ov.remove(&idx, "www").expect("rm empty folder");
        assert_eq!(names(&ov, &idx, ""), vec!["readme"]);
        assert!(ov.resolve(&idx, "www/index.html").is_none());
        ov.create_dir(&idx, "www").expect("again");
        assert!(
            names(&ov, &idx, "www").is_empty(),
            "the old contents stay hidden"
        );
        // Removing something only the overlay added leaves no marker.
        ov.create_dir(&idx, "tmp").expect("tmp");
        ov.remove(&idx, "tmp").expect("rm tmp");
        assert!(ov.changes(&idx).iter().all(|c| c.path != "tmp"));
    }

    #[test]
    fn renaming_a_folder_moves_its_subtree_without_copying() {
        let idx = base();
        let mut ov = Overlay::default();
        let p = spooled(&mut ov, b"new");
        ov.put_file(&idx, "www/new.txt", p.clone(), 3).expect("put");
        ov.rename(&idx, "www", "site").expect("rename");
        assert_eq!(names(&ov, &idx, ""), vec!["readme", "site"]);
        assert_eq!(
            names(&ov, &idx, "site"),
            vec!["img", "index.html", "new.txt"]
        );
        assert_eq!(names(&ov, &idx, "site/img"), vec!["logo.png"]);
        assert!(ov.resolve(&idx, "www").is_none());
        let Some(Resolved::Over(OvEntry::File {
            data: Data::Spool(moved),
            ..
        })) = ov.resolve(&idx, "site/new.txt")
        else {
            panic!("spooled file moved")
        };
        assert_eq!(moved, p);
        assert!(p.exists(), "bytes not deleted by the move");
        let base_id = idx.lookup("www/index.html").expect("id");
        assert_eq!(
            ov.resolve(&idx, "site/index.html"),
            Some(Resolved::Over(OvEntry::File {
                data: Data::Base(base_id),
                size: 3,
                modified: None
            }))
        );
        assert!(
            ov.rename(&idx, "site", "site/inner").is_err(),
            "into itself"
        );
        assert!(ov.rename(&idx, "readme", "site").is_err(), "target exists");
    }

    #[test]
    fn replacing_and_discarding() {
        let idx = base();
        let mut ov = Overlay::default();
        let p = spooled(&mut ov, b"v2");
        ov.put_file(&idx, "readme", p.clone(), 2).expect("replace");
        assert_eq!(
            ov.changes(&idx),
            vec![Change {
                path: "readme".into(),
                kind: "replaced"
            }]
        );
        let q = spooled(&mut ov, b"v3");
        ov.put_file(&idx, "readme", q, 2).expect("replace again");
        assert!(!p.exists(), "the older staged copy is freed");
        ov.clear();
        assert!(ov.is_empty());
        assert_eq!(
            ov.resolve(&idx, "readme"),
            idx.lookup("readme").map(Resolved::Base)
        );
        assert!(ov.reserve(MAX_STAGED + 1).is_err());
    }
}
