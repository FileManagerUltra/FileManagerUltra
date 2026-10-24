//! Planning pass: walk the sources, enforce the guards, compute totals — before any write
//! (design D2). A bad drop fails here, with the disk untouched.

use std::collections::BTreeSet;

use fm_vfs::{EntryKind, FmError, VfsUri};

use crate::ctx::Backends;
use crate::names::{keep_both_name, validate_name};

/// Hard caps so a hostile or accidental selection cannot allocate without bound.
#[derive(Debug, Clone, Copy)]
pub struct PlanLimits {
    pub max_items: usize,
    pub max_depth: usize,
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_items: 1_000_000,
            max_depth: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub src: VfsUri,
    pub dst: VfsUri,
    pub kind: PlanKind,
    pub size: u64,
}

/// One selected source and the contiguous slice of `Plan::items` it expands to
/// (pre-order: a directory precedes its contents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Top {
    pub src: VfsUri,
    pub dst: VfsUri,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<PlanItem>,
    pub tops: Vec<Top>,
    pub total_bytes: u64,
    /// Sources dropped on their own location — silently ignored.
    pub noops: usize,
}

impl Plan {
    pub fn total_items(&self) -> u64 {
        self.items.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.tops.is_empty()
    }
}

/// Plan copying/moving `sources` into the directory `dest_dir`. An item already in
/// `dest_dir` is a silent no-op.
pub fn plan_transfer(
    backends: &dyn Backends,
    sources: &[VfsUri],
    dest_dir: &VfsUri,
    limits: PlanLimits,
) -> Result<Plan, FmError> {
    plan_inner(backends, sources, dest_dir, limits, false)
}

/// Like [`plan_transfer`], but an item already in `dest_dir` is duplicated under the first
/// free `name (N).ext` instead of being skipped.
pub fn plan_duplicating(
    backends: &dyn Backends,
    sources: &[VfsUri],
    dest_dir: &VfsUri,
    limits: PlanLimits,
) -> Result<Plan, FmError> {
    plan_inner(backends, sources, dest_dir, limits, true)
}

fn plan_inner(
    backends: &dyn Backends,
    sources: &[VfsUri],
    dest_dir: &VfsUri,
    limits: PlanLimits,
    duplicate_in_place: bool,
) -> Result<Plan, FmError> {
    let dest_vfs = backends.vfs(dest_dir)?;
    if !dest_vfs.stat(dest_dir)?.is_dir() {
        return Err(FmError::NotADirectory(dest_dir.to_string()));
    }
    let mut plan = Plan::default();
    let mut seen = BTreeSet::new();
    for src in sources {
        if !seen.insert(src.clone()) {
            continue;
        }
        let name = src
            .file_name()
            .ok_or_else(|| FmError::InvalidParams("cannot transfer a filesystem root".into()))?;
        validate_name(name)?;
        let mut dst = dest_dir.join(name)?;
        if *src == dst {
            if !duplicate_in_place {
                plan.noops += 1;
                continue;
            }
            dst = keep_both_name(&*backends.vfs(dest_dir)?, dest_dir, name)?;
        }
        let vfs = backends.vfs(src)?;
        let entry = vfs.stat(src)?;
        if entry.is_dir() && src.is_same_or_descendant_of(dest_dir) {
            return Err(FmError::InvalidParams(format!(
                "cannot copy or move {name:?} into itself"
            )));
        }
        let start = plan.items.len();
        match &entry.kind {
            EntryKind::Dir => walk_dir(backends, src, &dst, 0, limits, &mut plan)?,
            EntryKind::File => push(&mut plan, src, &dst, PlanKind::File, entry.size, limits)?,
            EntryKind::Symlink { .. } => {
                push(&mut plan, src, &dst, PlanKind::Symlink, None, limits)?
            }
            EntryKind::Other => {
                return Err(FmError::Unsupported(
                    "special files (fifo, socket, device) cannot be copied",
                ));
            }
        }
        plan.tops.push(Top {
            src: src.clone(),
            dst,
            start,
            end: plan.items.len(),
        });
    }
    Ok(plan)
}

fn push(
    plan: &mut Plan,
    src: &VfsUri,
    dst: &VfsUri,
    kind: PlanKind,
    size: Option<u64>,
    limits: PlanLimits,
) -> Result<(), FmError> {
    if plan.items.len() >= limits.max_items {
        return Err(FmError::InvalidParams(format!(
            "selection too large (more than {} items)",
            limits.max_items
        )));
    }
    let size = size.unwrap_or(0);
    plan.total_bytes = plan.total_bytes.saturating_add(size);
    plan.items.push(PlanItem {
        src: src.clone(),
        dst: dst.clone(),
        kind,
        size,
    });
    Ok(())
}

fn walk_dir(
    backends: &dyn Backends,
    src: &VfsUri,
    dst: &VfsUri,
    depth: usize,
    limits: PlanLimits,
    plan: &mut Plan,
) -> Result<(), FmError> {
    if depth > limits.max_depth {
        return Err(FmError::InvalidParams(format!(
            "directory nesting deeper than {} levels",
            limits.max_depth
        )));
    }
    push(plan, src, dst, PlanKind::Dir, None, limits)?;
    let mut children = backends.vfs(src)?.read_dir(src)?;
    children.sort_by(|a, b| a.name.cmp(&b.name));
    for child in children {
        validate_name(&child.name)?;
        let child_dst = dst.join(&child.name)?;
        match &child.kind {
            EntryKind::Dir => walk_dir(backends, &child.uri, &child_dst, depth + 1, limits, plan)?,
            EntryKind::File => push(
                plan,
                &child.uri,
                &child_dst,
                PlanKind::File,
                child.size,
                limits,
            )?,
            EntryKind::Symlink { .. } => push(
                plan,
                &child.uri,
                &child_dst,
                PlanKind::Symlink,
                None,
                limits,
            )?,
            // Special files inside a tree are left behind, never opened.
            EntryKind::Other => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::SingleBackend;
    use fm_testkit::MemFs;
    use std::sync::Arc;

    fn uri(s: &str) -> VfsUri {
        VfsUri::parse(s).expect("uri")
    }

    fn backends(fs: &MemFs) -> SingleBackend {
        SingleBackend(Arc::new(fs.clone()))
    }

    fn tree() -> MemFs {
        let fs = MemFs::new();
        fs.put_str("/src/a/one.txt", b"1234").expect("put");
        fs.put_str("/src/a/b/two.txt", b"12").expect("put");
        fs.put_str("/src/top.txt", b"123456").expect("put");
        fs.mkdirs_str("/dst").expect("dst");
        fs
    }

    #[test]
    fn totals_and_preorder() {
        let fs = tree();
        let plan = plan_transfer(
            &backends(&fs),
            &[uri("/src/a"), uri("/src/top.txt")],
            &uri("/dst"),
            PlanLimits::default(),
        )
        .expect("plan");
        assert_eq!(plan.tops.len(), 2);
        assert_eq!(plan.total_items(), 5);
        assert_eq!(plan.total_bytes, 4 + 2 + 6);
        assert_eq!(plan.items[0].kind, PlanKind::Dir);
        assert_eq!(plan.items[0].dst, uri("/dst/a"));
        assert_eq!((plan.tops[0].start, plan.tops[0].end), (0, 4));
    }

    #[test]
    fn into_itself_and_descendant_are_rejected() {
        let fs = tree();
        let b = backends(&fs);
        for dest in ["/src/a", "/src/a/b"] {
            let got = plan_transfer(&b, &[uri("/src/a")], &uri(dest), PlanLimits::default());
            assert!(matches!(got, Err(FmError::InvalidParams(_))), "{dest}");
        }
    }

    #[test]
    fn same_uri_is_a_silent_noop() {
        let fs = tree();
        let plan = plan_transfer(
            &backends(&fs),
            &[uri("/src/top.txt")],
            &uri("/src"),
            PlanLimits::default(),
        )
        .expect("plan");
        assert!(plan.is_empty());
        assert_eq!(plan.noops, 1);
    }

    #[test]
    fn duplicating_numbers_items_already_in_the_destination() {
        let fs = tree();
        let plan = plan_duplicating(
            &backends(&fs),
            &[uri("/src/top.txt")],
            &uri("/src"),
            PlanLimits::default(),
        )
        .expect("plan");
        assert_eq!(plan.noops, 0);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].dst, uri("/src/top (2).txt"));
    }

    #[test]
    fn over_cap_fails_before_anything_else() {
        let fs = tree();
        let limits = PlanLimits {
            max_items: 2,
            ..PlanLimits::default()
        };
        let got = plan_transfer(&backends(&fs), &[uri("/src/a")], &uri("/dst"), limits);
        assert!(matches!(got, Err(FmError::InvalidParams(_))));
    }

    #[test]
    fn depth_cap_applies() {
        let fs = tree();
        let limits = PlanLimits {
            max_depth: 0,
            ..PlanLimits::default()
        };
        let got = plan_transfer(&backends(&fs), &[uri("/src/a")], &uri("/dst"), limits);
        assert!(matches!(got, Err(FmError::InvalidParams(_))));
    }

    #[test]
    fn missing_source_and_file_destination_error() {
        let fs = tree();
        let b = backends(&fs);
        assert!(matches!(
            plan_transfer(&b, &[uri("/nope")], &uri("/dst"), PlanLimits::default()),
            Err(FmError::NotFound(_))
        ));
        assert!(matches!(
            plan_transfer(
                &b,
                &[uri("/src/a")],
                &uri("/src/top.txt"),
                PlanLimits::default()
            ),
            Err(FmError::NotADirectory(_))
        ));
    }

    #[test]
    fn duplicates_collapse_and_root_is_refused() {
        let fs = tree();
        let b = backends(&fs);
        let plan = plan_transfer(
            &b,
            &[uri("/src/top.txt"), uri("/src/top.txt")],
            &uri("/dst"),
            PlanLimits::default(),
        )
        .expect("plan");
        assert_eq!(plan.tops.len(), 1);
        assert!(plan_transfer(&b, &[uri("/")], &uri("/dst"), PlanLimits::default()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_planned_as_links_not_followed() {
        use fm_fs_local::LocalFs;
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).expect("mk");
        std::fs::create_dir_all(root.join("dst")).expect("mk");
        std::fs::write(root.join("secret"), b"outside").expect("w");
        std::os::unix::fs::symlink(root.join("secret"), root.join("src/link")).expect("ln");
        let b = SingleBackend(Arc::new(LocalFs::new()));
        let s = uri(&root.join("src").to_string_lossy());
        let d = uri(&root.join("dst").to_string_lossy());
        let plan = plan_transfer(&b, &[s], &d, PlanLimits::default()).expect("plan");
        let link = plan
            .items
            .iter()
            .find(|i| i.src.file_name() == Some("link"));
        assert_eq!(link.map(|i| i.kind), Some(PlanKind::Symlink));
        assert_eq!(plan.total_bytes, 0);
    }
}
