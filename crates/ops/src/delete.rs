//! Delete pipelines: trash (the default) and confirmed permanent delete.

use fm_vfs::{EntryKind, FmError, VfsUri};

use crate::ctx::{Backends, OpCtx, OpReport};
use crate::plan::PlanLimits;
use crate::trash::trash_local;

/// Move each source to the platform trash. A failure on one item is reported and the job
/// continues; nothing is ever deleted permanently here.
pub fn trash_items(sources: &[VfsUri], ctx: &mut OpCtx<'_>) -> OpReport {
    let mut report = OpReport::default();
    let mut progress = crate::ctx::Progress::default();
    progress.items.total = sources.len() as u64;
    (ctx.on_progress)(&progress);
    for src in sources {
        if ctx.cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }
        progress.current = src.file_name().unwrap_or_default().to_string();
        match trash_local(src) {
            Ok(()) => {
                report.done += 1;
                report.trashed.push(src.clone());
            }
            Err(e) => report.failed.push((src.clone(), e)),
        }
        progress.items.done += 1;
        (ctx.on_progress)(&progress);
    }
    report
}

/// Permanently remove the sources, bottom-up through `Vfs::remove`. Symlinks are removed as
/// links and never followed. Callers must have confirmed with the user first.
pub fn delete_permanent(
    backends: &dyn Backends,
    sources: &[VfsUri],
    ctx: &mut OpCtx<'_>,
) -> Result<OpReport, FmError> {
    let limits = PlanLimits::default();
    let mut order: Vec<VfsUri> = Vec::new();
    for src in sources {
        if src.file_name().is_none() {
            return Err(FmError::InvalidParams(
                "cannot delete a filesystem root".into(),
            ));
        }
        collect_post_order(backends, src, 0, limits, &mut order)?;
    }
    let mut report = OpReport::default();
    let mut progress = crate::ctx::Progress::default();
    progress.items.total = order.len() as u64;
    (ctx.on_progress)(&progress);
    for uri in order {
        if ctx.cancel.is_cancelled() {
            report.cancelled = true;
            break;
        }
        progress.current = uri.file_name().unwrap_or_default().to_string();
        match backends.vfs(&uri).and_then(|vfs| vfs.remove(&uri)) {
            Ok(()) | Err(FmError::NotFound(_)) => report.done += 1,
            Err(e) => report.failed.push((uri, e)),
        }
        progress.items.done += 1;
        (ctx.on_progress)(&progress);
    }
    Ok(report)
}

fn collect_post_order(
    backends: &dyn Backends,
    uri: &VfsUri,
    depth: usize,
    limits: PlanLimits,
    out: &mut Vec<VfsUri>,
) -> Result<(), FmError> {
    if depth > limits.max_depth || out.len() >= limits.max_items {
        return Err(FmError::InvalidParams(
            "selection too large or too deeply nested to delete".into(),
        ));
    }
    let vfs = backends.vfs(uri)?;
    if matches!(vfs.stat(uri)?.kind, EntryKind::Dir) {
        for child in vfs.read_dir(uri)? {
            collect_post_order(backends, &child.uri, depth + 1, limits, out)?;
        }
    }
    out.push(uri.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConflictPolicy;
    use crate::ctx::{Choice, Decision, SingleBackend};
    use fm_testkit::MemFs;
    use std::sync::Arc;

    fn uri(s: &str) -> VfsUri {
        VfsUri::parse(s).expect("uri")
    }

    fn ctx_run<T>(f: impl FnOnce(&mut OpCtx<'_>) -> T) -> T {
        let mut progress = |_: &crate::ctx::Progress| {};
        let mut conflict = |_: &crate::ctx::Conflict| Decision {
            choice: Choice::Cancel,
            apply_to_all: false,
        };
        let mut ctx = OpCtx::new(ConflictPolicy::Ask, &mut progress, &mut conflict);
        f(&mut ctx)
    }

    #[test]
    fn permanent_delete_removes_a_tree_bottom_up() {
        let fs = MemFs::new();
        fs.put_str("/d/a/b/f.txt", b"1").expect("put");
        fs.put_str("/d/a/g.txt", b"2").expect("put");
        fs.put_str("/keep.txt", b"3").expect("put");
        let b = SingleBackend(Arc::new(fs.clone()));
        let report = ctx_run(|ctx| delete_permanent(&b, &[uri("/d")], ctx)).expect("delete");
        assert!(report.is_clean());
        assert_eq!(report.done, 5);
        assert!(report.trashed.is_empty());
        assert!(!fs.exists(&uri("/d")));
        assert!(fs.exists(&uri("/keep.txt")));
    }

    #[test]
    fn roots_are_refused() {
        let b = SingleBackend(Arc::new(MemFs::new()));
        let got = ctx_run(|ctx| delete_permanent(&b, &[uri("/")], ctx));
        assert!(matches!(got, Err(FmError::InvalidParams(_))));
    }

    #[test]
    fn trash_failure_never_deletes() {
        let remote = uri("smb://nas/share/f.txt");
        let report = ctx_run(|ctx| trash_items(std::slice::from_ref(&remote), ctx));
        assert_eq!(report.failed.len(), 1);
        assert!(matches!(report.failed[0].1, FmError::Unsupported(_)));
        assert!(report.trashed.is_empty());
    }
}
