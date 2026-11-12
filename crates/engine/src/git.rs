//! Per-repository git status cache (capability `git-status`).
//!
//! Status is computed once per repository on the `local` job lane and shared by every
//! pane inside it. Only changed entries are kept, at most [`MAX_REPOS`] repositories are
//! cached, and nothing here blocks the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use fm_git::{Mark, RepoStatus};
use fm_vfs::{Scheme, VfsUri};

use crate::jobs::JobLane;
use crate::session::Session;

/// Repositories kept in the cache (least recently shown is dropped).
pub const MAX_REPOS: usize = 8;
const DEBOUNCE: Duration = Duration::from_millis(300);
const STAMP_POLL: Duration = Duration::from_secs(2);
const ROOT_CACHE_CAP: usize = 256;
const MAX_LOOKUPS: usize = 16;
/// A repository nobody shows is dropped after this long.
const GRACE: Duration = Duration::from_secs(30);

type Stamp = Option<fm_git::Stamp>;
type Done = Vec<(PathBuf, Stamp, Result<RepoStatus, String>)>;

/// Result of a background look at the disk: repository roots for folders and the
/// repositories whose `.git/index`/`HEAD` changed.
struct Checked {
    roots: Vec<(PathBuf, Option<PathBuf>)>,
    changed: Vec<PathBuf>,
}

struct Repo {
    root: PathBuf,
    status: Option<Arc<RepoStatus>>,
    stamp: Stamp,
    dirty_at: Option<Instant>,
    inflight: bool,
    used: Instant,
    error: Option<String>,
}

/// Shared git state of the session.
#[derive(Default)]
pub struct GitState {
    repos: Vec<Repo>,
    roots: HashMap<PathBuf, Option<PathBuf>>,
    done: Arc<Mutex<Done>>,
    checked: Arc<Mutex<Vec<Checked>>>,
    /// Folders whose repository root is being looked up (a sleeping disk can take seconds).
    looking: std::collections::HashSet<PathBuf>,
    checking: bool,
    last_stamp_check: Option<Instant>,
}

impl std::fmt::Debug for GitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitState")
            .field("repos", &self.repos.len())
            .finish()
    }
}

impl GitState {
    /// The repository root of `dir` once known. The lookup touches the disk, so it runs on
    /// a thread; until it lands the folder simply shows no git marks.
    fn root_of(&mut self, dir: &Path) -> Option<PathBuf> {
        if let Some(hit) = self.roots.get(dir) {
            return hit.clone();
        }
        if self.looking.len() < MAX_LOOKUPS && self.looking.insert(dir.to_path_buf()) {
            self.spawn_check(vec![dir.to_path_buf()], Vec::new());
        }
        None
    }

    /// Look at the disk off the UI thread: discover the roots of `dirs` and compare the
    /// stamps of `repos`.
    fn spawn_check(&mut self, dirs: Vec<PathBuf>, repos: Vec<(PathBuf, Stamp)>) {
        let out = self.checked.clone();
        let spawned = std::thread::Builder::new()
            .name("git-roots".into())
            .spawn(move || {
                let roots = dirs
                    .into_iter()
                    .map(|d| {
                        let root = fm_git::discover(&d);
                        (d, root)
                    })
                    .collect();
                let changed = repos
                    .into_iter()
                    .filter(|(root, stamp)| fm_git::fingerprint(root) != *stamp)
                    .map(|(root, _)| root)
                    .collect();
                out.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(Checked { roots, changed });
            });
        if spawned.is_err() {
            self.looking.clear();
            self.checking = false;
        }
    }

    fn repo_mut(&mut self, root: &Path) -> &mut Repo {
        if let Some(i) = self.repos.iter().position(|r| r.root == root) {
            return &mut self.repos[i];
        }
        if self.repos.len() >= MAX_REPOS
            && let Some(oldest) = self
                .repos
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.inflight)
                .min_by_key(|(_, r)| r.used)
                .map(|(i, _)| i)
        {
            self.repos.swap_remove(oldest);
        }
        self.repos.push(Repo {
            root: root.to_path_buf(),
            status: None,
            stamp: None,
            dirty_at: Some(Instant::now() - DEBOUNCE),
            inflight: false,
            used: Instant::now(),
            error: None,
        });
        let last = self.repos.len() - 1;
        &mut self.repos[last]
    }

    /// Mark every cached repository stale (`git.refresh`).
    pub fn invalidate_all(&mut self) {
        self.refresh_roots();
        let now = Instant::now() - DEBOUNCE;
        for r in &mut self.repos {
            r.dirty_at = Some(now);
        }
    }

    /// Re-discover every cached root in the background (a folder may have become a repo).
    fn refresh_roots(&mut self) {
        if self.checking {
            return;
        }
        self.checking = true;
        let dirs = self.roots.keys().cloned().collect();
        self.spawn_check(dirs, Vec::new());
    }

    /// A working-tree folder changed: its repository needs a rescan.
    pub fn touch_dir(&mut self, dir: &Path) {
        let Some(root) = self.root_of(dir) else {
            return;
        };
        if let Some(r) = self.repos.iter_mut().find(|r| r.root == root)
            && r.dirty_at.is_none()
        {
            r.dirty_at = Some(Instant::now());
        }
    }

    /// Number of cached repositories (tests, diagnostics).
    pub fn cached(&self) -> usize {
        self.repos.len()
    }
}

impl Session {
    /// Status of the repository that contains the local folder `uri`, once computed.
    /// Registers the repository for tracking; never blocks.
    pub fn git_view(&mut self, uri: &VfsUri) -> Option<Arc<RepoStatus>> {
        if !self.prefs.git_enabled || uri.scheme() != Scheme::File {
            return None;
        }
        let dir = uri.local_path()?;
        let root = self.git.root_of(&dir)?;
        let repo = self.git.repo_mut(&root);
        repo.used = Instant::now();
        repo.status.clone()
    }

    /// Like [`Session::git_view`] but only reads what is already cached: used for rows that
    /// must not start tracking a repository (tree folders).
    pub fn git_peek(&mut self, uri: &VfsUri) -> Option<Arc<RepoStatus>> {
        if !self.prefs.git_enabled || uri.scheme() != Scheme::File {
            return None;
        }
        let root = self.git.root_of(&uri.local_path()?)?;
        self.git
            .repos
            .iter()
            .find(|r| r.root == root)
            .and_then(|r| r.status.clone())
    }

    /// Drive refreshes: rescan dirty repositories and notice `.git/index`/`HEAD` changes.
    pub(crate) fn git_tick(&mut self, changed: &[VfsUri]) {
        if !self.prefs.git_enabled {
            if self.git.cached() > 0 {
                self.git = GitState::default();
            }
            return;
        }
        for uri in changed {
            if let Some(dir) = uri.local_path() {
                self.git.touch_dir(&dir);
            }
        }
        let now = Instant::now();
        let check = self
            .git
            .last_stamp_check
            .is_none_or(|t| t.elapsed() >= STAMP_POLL);
        if check {
            self.git.last_stamp_check = Some(now);
            if !self.git.checking {
                self.git.checking = true;
                let dirs = self.git.roots.keys().cloned().collect();
                let repos = self
                    .git
                    .repos
                    .iter()
                    .filter(|r| r.status.is_some() && r.dirty_at.is_none())
                    .map(|r| (r.root.clone(), r.stamp))
                    .collect();
                self.git.spawn_check(dirs, repos);
            }
        }
        self.git
            .repos
            .retain(|r| r.inflight || r.used.elapsed() < GRACE);
        let due: Vec<PathBuf> = self
            .git
            .repos
            .iter()
            .filter(|r| !r.inflight && r.dirty_at.is_some_and(|t| t.elapsed() >= DEBOUNCE))
            .map(|r| r.root.clone())
            .collect();
        for root in due {
            self.git_spawn(root);
        }
    }

    fn git_spawn(&mut self, root: PathBuf) {
        let Some(repo) = self.git.repos.iter_mut().find(|r| r.root == root) else {
            return;
        };
        repo.inflight = true;
        repo.dirty_at = None;
        let done = self.git.done.clone();
        self.jobs.submit(JobLane::Local, move |_ctx| {
            // The stamp is taken before the scan so a change during it triggers another.
            let stamp = fm_git::fingerprint(&root);
            let result = fm_git::status(&root).map_err(|e| e.to_string());
            done.lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((root, stamp, result));
            Ok(())
        });
    }

    /// Apply finished scans; returns whether anything visible changed.
    pub(crate) fn pump_git(&mut self) -> bool {
        let mut changed = self.pump_git_checks();
        let finished: Done =
            std::mem::take(&mut *self.git.done.lock().unwrap_or_else(|p| p.into_inner()));
        for (root, stamp, result) in finished {
            let Some(repo) = self.git.repos.iter_mut().find(|r| r.root == root) else {
                continue;
            };
            repo.inflight = false;
            repo.stamp = stamp;
            match result {
                Ok(status) => {
                    repo.error = None;
                    if repo.status.as_deref() != Some(&status) {
                        if status.truncated {
                            self.notice =
                                Some("Git status incomplete: too many changed files".to_string());
                        }
                        repo.status = Some(Arc::new(status));
                        changed = true;
                    }
                }
                Err(e) => {
                    changed |= repo.error.replace(e).is_none();
                    if repo.status.is_none() {
                        repo.status = Some(Arc::new(RepoStatus::default()));
                    }
                }
            }
        }
        changed
    }

    fn pump_git_checks(&mut self) -> bool {
        let batches: Vec<Checked> =
            std::mem::take(&mut *self.git.checked.lock().unwrap_or_else(|p| p.into_inner()));
        if batches.is_empty() {
            return false;
        }
        let now = Instant::now();
        let mut changed = false;
        for batch in batches {
            for (dir, root) in batch.roots {
                self.git.looking.remove(&dir);
                if self.git.roots.len() >= ROOT_CACHE_CAP && !self.git.roots.contains_key(&dir) {
                    self.git.roots.clear();
                }
                changed |= self.git.roots.insert(dir, root.clone()) != Some(root);
            }
            for root in batch.changed {
                if let Some(r) = self.git.repos.iter_mut().find(|r| r.root == root)
                    && r.dirty_at.is_none()
                {
                    r.dirty_at = Some(now);
                }
            }
        }
        self.git.checking = false;
        changed
    }

    /// Whether git work is outstanding (keeps the UI repainting until it lands).
    pub(crate) fn git_busy(&self) -> bool {
        self.git
            .repos
            .iter()
            .any(|r| r.inflight || r.dirty_at.is_some())
            || !self.git.looking.is_empty()
            || self.git.checking
    }
}
