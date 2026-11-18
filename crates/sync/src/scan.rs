//! Bounded, cancellable scan of one side into a flat, sorted map of relative paths.
//! Names failing `validate_name`, leftover copy temp files and special files never enter
//! the tree; the entry count and depth are capped so a hostile or runaway folder fails
//! the scan instead of exhausting memory.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use fm_ops::{CancelFlag, validate_name};
use fm_vfs::{EntryKind, FmError, Vfs, VfsUri};
use serde::{Deserialize, Serialize};

use crate::glob::Glob;
use crate::profile::{Filters, SymlinkPolicy};

/// Entries a scan accepts before it gives up (a partial tree would plan wrong deletes).
pub const MAX_ENTRIES: usize = 1_000_000;
/// Deepest folder level followed.
pub const MAX_DEPTH: usize = 64;
/// Most nested folder links followed on one path.
const MAX_LINK_CHAIN: usize = 16;
/// Most warnings kept; the rest are counted.
const MAX_WARNINGS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    /// Link target text, for symlinks.
    pub link: Option<String>,
}

/// Root-relative, `/`-separated path → node.
pub type Tree = BTreeMap<String, Node>;

#[derive(Debug, Default)]
pub struct Scan {
    pub tree: Tree,
    pub warnings: Vec<String>,
    /// Warnings dropped beyond the cap.
    pub more_warnings: u64,
    /// Folders that could not be listed. Anything below them is unknown, not absent, so
    /// the planner never acts on those paths.
    pub unreadable: Vec<String>,
    /// The root folder does not exist (yet); it is treated as empty and created by a run.
    pub missing: bool,
}

impl Scan {
    fn warn(&mut self, msg: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(msg);
        } else {
            self.more_warnings += 1;
        }
    }
}

/// Compiled include/exclude lists.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    include: Vec<Glob>,
    exclude: Vec<Glob>,
}

impl Filter {
    pub fn compile(f: &Filters) -> Result<Filter, FmError> {
        let all = |v: &[String]| -> Result<Vec<Glob>, FmError> {
            v.iter().map(|g| Glob::compile(g)).collect()
        };
        Ok(Filter {
            include: all(&f.include)?,
            exclude: all(&f.exclude)?,
        })
    }

    pub fn is_excluded(&self, rel: &str) -> bool {
        self.exclude.iter().any(|g| g.is_match(rel))
    }

    /// Folders are always entered unless excluded; files must also match an include.
    pub fn allows(&self, rel: &str, is_dir: bool) -> bool {
        !self.is_excluded(rel)
            && (is_dir || self.include.is_empty() || self.include.iter().any(|g| g.is_match(rel)))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_entries: usize,
    pub max_depth: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_ENTRIES,
            max_depth: MAX_DEPTH,
        }
    }
}

struct Frame {
    uri: VfsUri,
    rel: String,
    depth: usize,
    /// Targets of the folder links entered on the way here (cycle guard).
    chain: Arc<Vec<String>>,
}

pub fn is_copy_temp(name: &str) -> bool {
    name.starts_with(".fmpart-") || name.starts_with(".fmbak-")
}

/// Scan `root` on `vfs`. `progress` receives the running entry count.
pub fn scan(
    vfs: &dyn Vfs,
    root: &VfsUri,
    filter: &Filter,
    symlinks: SymlinkPolicy,
    limits: ScanLimits,
    cancel: &CancelFlag,
    progress: &mut dyn FnMut(usize),
) -> Result<Scan, FmError> {
    let root_entry = vfs.stat(root)?;
    if !root_entry.is_dir() {
        return Err(FmError::NotADirectory(root.to_string()));
    }
    let mut out = Scan::default();
    let mut stack = vec![Frame {
        uri: root.clone(),
        rel: String::new(),
        depth: 0,
        chain: Arc::new(Vec::new()),
    }];
    while let Some(frame) = stack.pop() {
        if cancel.is_cancelled() {
            return Err(FmError::Cancelled);
        }
        let entries = match vfs.read_dir(&frame.uri) {
            Ok(e) => e,
            // The root must be readable; a vanished or locked subfolder is a warning, and
            // its contents are then simply absent (never "deleted" — see planner guard).
            Err(e) if frame.depth > 0 => {
                out.warn(format!("{}: {e}", frame.rel));
                out.unreadable.push(frame.rel.clone());
                continue;
            }
            Err(e) => return Err(e),
        };
        for entry in entries {
            if validate_name(&entry.name).is_err() {
                out.warn(format!("skipped unsafe name {:?}", entry.name));
                continue;
            }
            if is_copy_temp(&entry.name) {
                continue;
            }
            let rel = if frame.rel.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", frame.rel, entry.name)
            };
            let child_uri = match frame.uri.join(&entry.name) {
                Ok(u) => u,
                Err(_) => {
                    out.warn(format!("skipped unsafe name {:?}", entry.name));
                    continue;
                }
            };
            let (kind, descend, chain) = match &entry.kind {
                EntryKind::Dir => (NodeKind::Dir, true, frame.chain.clone()),
                EntryKind::File => (NodeKind::File, false, frame.chain.clone()),
                EntryKind::Symlink { .. } if symlinks == SymlinkPolicy::Skip => continue,
                EntryKind::Symlink { target } if symlinks == SymlinkPolicy::Follow => {
                    if !entry.link_to_dir {
                        out.warn(format!("{rel}: file links are not followed; skipped"));
                        continue;
                    }
                    if frame.chain.contains(target) || frame.chain.len() >= MAX_LINK_CHAIN {
                        out.warn(format!("{rel}: link cycle, not entered"));
                        continue;
                    }
                    let mut chain = (*frame.chain).clone();
                    chain.push(target.clone());
                    (NodeKind::Dir, true, Arc::new(chain))
                }
                EntryKind::Symlink { .. } => (NodeKind::Symlink, false, frame.chain.clone()),
                _ => {
                    out.warn(format!("{rel}: special file skipped"));
                    continue;
                }
            };
            if !filter.allows(&rel, kind == NodeKind::Dir) {
                continue;
            }
            if out.tree.len() >= limits.max_entries {
                return Err(FmError::InvalidParams(format!(
                    "more than {} entries; narrow the folder or add filters",
                    limits.max_entries
                )));
            }
            let link = match &entry.kind {
                EntryKind::Symlink { target } if kind == NodeKind::Symlink => Some(target.clone()),
                _ => None,
            };
            out.tree.insert(
                rel.clone(),
                Node {
                    kind,
                    size: if kind == NodeKind::File {
                        entry.size.unwrap_or(0)
                    } else {
                        0
                    },
                    mtime: entry.modified,
                    link,
                },
            );
            if out.tree.len() % 500 == 0 {
                progress(out.tree.len());
            }
            if descend {
                if frame.depth + 1 > limits.max_depth {
                    return Err(FmError::InvalidParams(format!(
                        "folders nested deeper than {} levels",
                        limits.max_depth
                    )));
                }
                stack.push(Frame {
                    uri: child_uri,
                    rel,
                    depth: frame.depth + 1,
                    chain,
                });
            }
        }
    }
    progress(out.tree.len());
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use fm_testkit::MemFs;

    fn run(fs: &MemFs, f: &Filters, sym: SymlinkPolicy, lim: ScanLimits) -> Result<Scan, FmError> {
        scan(
            fs,
            &VfsUri::parse("/r").expect("uri"),
            &Filter::compile(f).expect("filter"),
            sym,
            lim,
            &CancelFlag::new(),
            &mut |_| {},
        )
    }

    fn tree() -> MemFs {
        let fs = MemFs::new();
        fs.put_str("/r/a.txt", b"a").expect("put");
        fs.put_str("/r/d/b.txt", b"bb").expect("put");
        fs.put_str("/r/d/e/c.tmp", b"c").expect("put");
        fs.put_str("/r/.fmpart-1-1", b"x").expect("put");
        fs
    }

    #[test]
    fn scans_relative_paths_and_skips_copy_temps() {
        let s = run(
            &tree(),
            &Filters::default(),
            SymlinkPolicy::Preserve,
            ScanLimits::default(),
        )
        .expect("scan");
        let keys: Vec<&str> = s.tree.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a.txt", "d", "d/b.txt", "d/e", "d/e/c.tmp"]);
        assert_eq!(s.tree.get("d/b.txt").map(|n| n.size), Some(2));
    }

    #[test]
    fn filters_hide_paths_and_excluded_folders_are_not_entered() {
        let f = Filters {
            include: vec![],
            exclude: vec!["*.tmp".into(), "e".into()],
        };
        let s = run(&tree(), &f, SymlinkPolicy::Preserve, ScanLimits::default()).expect("scan");
        assert!(!s.tree.contains_key("d/e") && !s.tree.contains_key("d/e/c.tmp"));
        let f = Filters {
            include: vec!["*.txt".into()],
            exclude: vec![],
        };
        let s = run(&tree(), &f, SymlinkPolicy::Preserve, ScanLimits::default()).expect("scan");
        assert!(s.tree.contains_key("a.txt") && !s.tree.contains_key("d/e/c.tmp"));
        assert!(
            s.tree.contains_key("d/e"),
            "folders stay visible under includes"
        );
    }

    #[test]
    fn caps_fail_the_scan_instead_of_truncating() {
        let lim = ScanLimits {
            max_entries: 3,
            ..ScanLimits::default()
        };
        assert!(run(&tree(), &Filters::default(), SymlinkPolicy::Preserve, lim).is_err());
        let lim = ScanLimits {
            max_depth: 1,
            ..ScanLimits::default()
        };
        assert!(run(&tree(), &Filters::default(), SymlinkPolicy::Preserve, lim).is_err());
    }

    #[test]
    fn symlinks_follow_the_policy() {
        let fs = tree();
        fs.put_symlink(&VfsUri::parse("/r/l").expect("uri"), "a.txt")
            .expect("link");
        let s = run(
            &fs,
            &Filters::default(),
            SymlinkPolicy::Preserve,
            ScanLimits::default(),
        )
        .expect("scan");
        assert_eq!(
            s.tree.get("l").and_then(|n| n.link.as_deref()),
            Some("a.txt")
        );
        let s = run(
            &fs,
            &Filters::default(),
            SymlinkPolicy::Skip,
            ScanLimits::default(),
        )
        .expect("scan");
        assert!(!s.tree.contains_key("l"));
    }

    #[test]
    fn cancel_stops_the_scan() {
        let c = CancelFlag::new();
        c.cancel();
        let r = scan(
            &tree(),
            &VfsUri::parse("/r").expect("uri"),
            &Filter::default(),
            SymlinkPolicy::Preserve,
            ScanLimits::default(),
            &c,
            &mut |_| {},
        );
        assert!(matches!(r, Err(FmError::Cancelled)));
    }

    #[test]
    fn a_file_root_is_rejected() {
        let fs = tree();
        let r = scan(
            &fs,
            &VfsUri::parse("/r/a.txt").expect("uri"),
            &Filter::default(),
            SymlinkPolicy::Preserve,
            ScanLimits::default(),
            &CancelFlag::new(),
            &mut |_| {},
        );
        assert!(r.is_err());
    }
}
