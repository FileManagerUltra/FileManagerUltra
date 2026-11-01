//! Read-only git access on top of pure-Rust `gix`.
//!
//! No git binary, no hooks, no network: user/system config is ignored and no
//! external program can be spawned. Everything is bounded (entry count, blob
//! size) because repositories are untrusted input.

pub mod diff;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gix::bstr::ByteSlice;
use gix::dir::walk::EmissionMode;
use gix::status::UntrackedFiles;

/// Maximum changed entries kept per repository.
pub const MAX_ENTRIES: usize = 200_000;
/// Maximum bytes of a single blob that is loaded.
pub const MAX_BLOB: usize = 2 * 1024 * 1024;
/// Maximum parent levels walked looking for a repository.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

fn err(e: impl std::fmt::Display) -> GitError {
    GitError(e.to_string())
}

/// Status of one path; ordered by display priority (higher wins on merge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mark {
    Ignored,
    Untracked,
    Added,
    Renamed,
    Modified,
    Removed,
    Conflicted,
}

impl Mark {
    pub fn letter(self) -> char {
        match self {
            Mark::Ignored => '!',
            Mark::Untracked => '?',
            Mark::Added => 'A',
            Mark::Renamed => 'R',
            Mark::Modified => 'M',
            Mark::Removed => 'D',
            Mark::Conflicted => 'U',
        }
    }
}

/// Changed entries of one repository. Paths are `/`-separated, relative to the root.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    pub root: PathBuf,
    pub branch: Option<String>,
    own: HashMap<Box<str>, Mark>,
    dirs: HashMap<Box<str>, Mark>,
    /// The entry cap was hit; the map is partial.
    pub truncated: bool,
}

impl RepoStatus {
    pub fn len(&self) -> usize {
        self.own.len()
    }

    /// Tracked or untracked paths that differ (ignored paths do not count).
    pub fn changed(&self) -> usize {
        self.own.values().filter(|m| **m != Mark::Ignored).count()
    }

    pub fn is_empty(&self) -> bool {
        self.own.is_empty()
    }

    fn insert(&mut self, rel: &str, mark: Mark) {
        let rel = rel.trim_end_matches('/');
        if rel.is_empty() {
            return;
        }
        if !self.own.contains_key(rel) && self.own.len() >= MAX_ENTRIES {
            self.truncated = true;
            return;
        }
        let slot = self.own.entry(rel.into()).or_insert(mark);
        if mark > *slot {
            *slot = mark;
        }
        if mark == Mark::Ignored {
            return;
        }
        let mut end = rel.len();
        while let Some(i) = rel[..end].rfind('/') {
            end = i;
            let d = self.dirs.entry(rel[..end].into()).or_insert(mark);
            if mark > *d {
                *d = mark;
            }
        }
    }

    /// Mark of a file or folder given a root-relative `/` path.
    /// Folders report the strongest mark of their contents.
    pub fn mark(&self, rel: &str) -> Option<Mark> {
        let own = self.own.get(rel).copied();
        let sub = self.dirs.get(rel).copied();
        match (own, sub) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (None, None) => self.inherited(rel),
            (a, b) => a.or(b),
        }
    }

    /// Entries below a collapsed untracked/ignored folder inherit its mark.
    fn inherited(&self, rel: &str) -> Option<Mark> {
        let mut end = rel.len();
        while let Some(i) = rel[..end].rfind('/') {
            end = i;
            if let Some(m) = self.own.get(&rel[..end])
                && matches!(m, Mark::Untracked | Mark::Ignored)
            {
                return Some(*m);
            }
        }
        None
    }

    /// Mark for an absolute path inside this repository.
    pub fn mark_abs(&self, path: &Path) -> Option<Mark> {
        let rel = rel_string(path.strip_prefix(&self.root).ok()?)?;
        if rel.is_empty() {
            return self.dirs.values().max().copied();
        }
        self.mark(&rel)
    }
}

fn rel_string(rel: &Path) -> Option<String> {
    let mut out = String::new();
    for c in rel.components() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(c.as_os_str().to_str()?);
    }
    Some(out)
}

/// Find the repository work-tree root containing `dir` (walks parents, bounded).
/// Accepts both `.git` directories and `.git` files (worktrees, submodules).
pub fn discover(dir: &Path) -> Option<PathBuf> {
    let mut cur = Some(dir);
    for _ in 0..MAX_DEPTH {
        let d = cur?;
        if std::fs::symlink_metadata(d.join(".git")).is_ok() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

/// Repository root and root-relative `/` path of an absolute file path.
pub fn locate(file: &Path) -> Option<(PathBuf, String)> {
    let root = discover(file.parent()?)?;
    let rel = rel_string(file.strip_prefix(&root).ok()?)?;
    (!rel.is_empty()).then_some((root, rel))
}

type Stamp = (std::time::SystemTime, std::time::SystemTime);

/// Cheap fingerprint of repository state (`index` and `HEAD` mtimes).
pub fn fingerprint(root: &Path) -> Option<Stamp> {
    let git = resolve_git_dir(root)?;
    let m = |n: &str| {
        std::fs::metadata(git.join(n))
            .and_then(|m| m.modified())
            .ok()
    };
    Some((m("index")?, m("HEAD")?))
}

fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    let p = Path::new(target);
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    })
}

/// Open read-only. Repository-local `filter.*` drivers are removed from the in-memory
/// config: they are shell commands, and a cloned repo must never get to run one.
fn open(root: &Path) -> Result<gix::Repository, GitError> {
    let mut repo = gix::open_opts(root, gix::open::Options::isolated()).map_err(err)?;
    {
        let mut snapshot = repo.config_snapshot_mut();
        let drivers: Vec<_> = snapshot
            .sections_by_name("filter")
            .into_iter()
            .flatten()
            .filter_map(|section| section.header().subsection_name().map(ToOwned::to_owned))
            .collect();
        for name in drivers {
            while snapshot
                .remove_section("filter", Some(name.as_ref()))
                .is_some()
            {}
        }
    }
    Ok(repo)
}

fn branch_of(repo: &gix::Repository) -> Option<String> {
    match repo.head_name() {
        Ok(Some(name)) => Some(name.shorten().to_string()),
        Ok(None) => repo
            .head_id()
            .ok()
            .map(|id| format!("({})", id.to_string().chars().take(7).collect::<String>())),
        Err(_) => None,
    }
}

/// Compute the status (changed entries only) of the repository at `root`.
pub fn status(root: &Path) -> Result<RepoStatus, GitError> {
    use gix::status::Item;
    use gix::status::index_worktree::Item as WtItem;
    use gix::status::index_worktree::iter::Summary;

    let repo = open(root)?;
    let mut out = RepoStatus {
        root: root.to_path_buf(),
        branch: branch_of(&repo),
        ..Default::default()
    };
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(err)?
        .untracked_files(UntrackedFiles::Collapsed)
        .dirwalk_options(|o| o.emit_ignored(Some(EmissionMode::CollapseDirectory)))
        .into_iter(None)
        .map_err(err)?;
    for item in iter {
        let item = item.map_err(err)?;
        let Ok(path) = item.location().to_str() else {
            continue;
        };
        let mark = match &item {
            Item::TreeIndex(change) => tree_mark(change),
            Item::IndexWorktree(WtItem::DirectoryContents { entry, .. }) => match entry.status {
                gix::dir::entry::Status::Ignored(_) => Mark::Ignored,
                gix::dir::entry::Status::Untracked => Mark::Untracked,
                _ => continue,
            },
            Item::IndexWorktree(wt) => match wt.summary() {
                Some(Summary::Removed) => Mark::Removed,
                Some(Summary::Added | Summary::IntentToAdd) => Mark::Added,
                Some(Summary::Renamed | Summary::Copied) => Mark::Renamed,
                Some(Summary::Conflict) => Mark::Conflicted,
                Some(_) => Mark::Modified,
                None => continue,
            },
        };
        out.insert(path, mark);
        if out.truncated {
            break;
        }
    }
    Ok(out)
}

fn tree_mark(change: &gix::diff::index::Change) -> Mark {
    use gix::diff::index::Change;
    match change {
        Change::Addition { .. } => Mark::Added,
        Change::Deletion { .. } => Mark::Removed,
        Change::Modification { .. } => Mark::Modified,
        Change::Rewrite { .. } => Mark::Renamed,
    }
}

/// Where committed/staged content comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rev {
    /// Committed content at any revision spec (`HEAD`, branch, tag, sha).
    Commit(String),
    /// Staged content.
    Index,
}

/// Content of a file at `rev`; `rel` is root-relative with `/` separators.
/// Returns `Ok(None)` when the file does not exist at that revision.
pub fn blob(root: &Path, rev: &Rev, rel: &str) -> Result<Option<Vec<u8>>, GitError> {
    if rel.contains('\0') || rel.starts_with('/') || rel.split('/').any(|c| c == "..") {
        return Err(GitError("invalid path".into()));
    }
    let spec = match rev {
        Rev::Commit(r) => {
            if r.is_empty() || r.len() > 256 || r.starts_with('-') || r.contains('\0') {
                return Err(GitError("invalid revision".into()));
            }
            format!("{r}:{rel}")
        }
        Rev::Index => format!(":{rel}"),
    };
    let repo = open(root)?;
    let Ok(id) = repo.rev_parse_single(spec.as_bytes().as_bstr()) else {
        return Ok(None);
    };
    let obj = repo.find_object(id.detach()).map_err(err)?;
    if obj.data.len() > MAX_BLOB {
        return Err(GitError(format!(
            "file larger than {} MiB",
            MAX_BLOB / 1024 / 1024
        )));
    }
    Ok(Some(obj.data.clone()))
}
