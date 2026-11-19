//! Writing a buffer the app produced (a filled or signed PDF) to a file, the same way a
//! copy writes: into a temporary name next to the target, committed by rename, with the
//! conflict prompt when the target exists and progress while it streams. A failed or
//! cancelled write never leaves a partial file in place of the target.

use std::io::Write;

use fm_vfs::{Entry, EntryKind, FmError, VfsUri, WriteOpts};

use crate::ConflictPolicy;
use crate::ctx::{Backends, Choice, Conflict, ConflictSide, OpCtx, OpReport, Progress};
use crate::names::keep_both_name;
use crate::transfer::{commit, temp_name};

const CHUNK: usize = 1024 * 1024;

/// Write `bytes` to `dest`. `replace` means the caller already confirmed replacing an
/// existing file (no prompt); otherwise an existing target goes through the policy.
/// The report's `created` names the file written (it may be a "keep both" name).
pub fn write_bytes(
    backends: &dyn Backends,
    dest: &VfsUri,
    bytes: &[u8],
    replace: bool,
    ctx: &mut OpCtx<'_>,
) -> OpReport {
    let mut report = OpReport::default();
    match write_inner(backends, dest, bytes, replace, ctx) {
        Ok(Some(written)) => {
            report.done = 1;
            report.created.push(written);
        }
        Ok(None) => report.skipped = 1,
        Err(FmError::Cancelled) => report.cancelled = true,
        Err(e) => report.failed.push((dest.clone(), e)),
    }
    report
}

fn write_inner(
    backends: &dyn Backends,
    dest: &VfsUri,
    bytes: &[u8],
    replace: bool,
    ctx: &mut OpCtx<'_>,
) -> Result<Option<VfsUri>, FmError> {
    let vfs = backends.vfs(dest)?;
    let parent = dest
        .parent()
        .ok_or_else(|| FmError::NotADirectory(dest.to_string()))?;
    let mut target = dest.clone();
    let mut overwrite = false;
    match vfs.stat(dest) {
        Ok(existing) if existing.kind == EntryKind::Dir => {
            return Err(FmError::AlreadyExists(dest.to_string()));
        }
        Ok(existing) => {
            let choice = if replace {
                Choice::Overwrite
            } else {
                decide(ctx, dest, bytes.len() as u64, &existing)
            };
            match choice {
                Choice::Skip => return Ok(None),
                Choice::Cancel => return Err(FmError::Cancelled),
                Choice::Overwrite => overwrite = true,
                Choice::KeepBoth => {
                    let name = dest.file_name().unwrap_or_default().to_string();
                    target = keep_both_name(&*vfs, &parent, &name)?;
                }
            }
        }
        Err(FmError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    let mut progress = Progress::default();
    progress.items.total = 1;
    progress.bytes.total = bytes.len() as u64;
    progress.current = target.file_name().unwrap_or_default().to_string();
    let tmp = parent.join(&temp_name(".fmpart"))?;
    let streamed = stream(&*vfs, &tmp, bytes, ctx, &mut progress);
    match streamed.and_then(|()| commit(&*vfs, &tmp, &target, &parent, overwrite)) {
        Ok(()) => {}
        Err(FmError::Unsupported(_)) if !overwrite => {
            // No rename on this backend: write in place (the target does not exist).
            let _ = vfs.remove(&tmp);
            progress.bytes.done = 0;
            if let Err(e) = stream(&*vfs, &target, bytes, ctx, &mut progress) {
                let _ = vfs.remove(&target);
                return Err(e);
            }
        }
        Err(e) => {
            let _ = vfs.remove(&tmp);
            return Err(e);
        }
    }
    progress.items.done = 1;
    (ctx.on_progress)(&progress);
    Ok(Some(target))
}

/// `dest`, or `<stem> 2.<ext>`, `<stem> 3.<ext>` … when it exists (first free name).
pub fn free_name(backends: &dyn Backends, dest: &VfsUri) -> Result<VfsUri, FmError> {
    let vfs = backends.vfs(dest)?;
    let parent = dest
        .parent()
        .ok_or_else(|| FmError::NotADirectory(dest.to_string()))?;
    let name = dest.file_name().unwrap_or_default().to_string();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    for n in 1..1000u32 {
        let candidate = if n == 1 {
            dest.clone()
        } else {
            parent.join(&format!("{stem} {n}{ext}"))?
        };
        match vfs.stat(&candidate) {
            Err(FmError::NotFound(_)) => return Ok(candidate),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
    }
    Err(FmError::AlreadyExists(dest.to_string()))
}

/// Refuse when `uri` is no longer exactly `original` (changed by another program since
/// it was loaded): size first, then the bytes themselves.
pub fn verify_unchanged(
    backends: &dyn Backends,
    uri: &VfsUri,
    original: &[u8],
) -> Result<(), FmError> {
    use std::io::Read;
    let vfs = backends.vfs(uri)?;
    let changed = || {
        FmError::InvalidParams(
            "The file changed on disk since it was opened; nothing was written. Save a copy instead."
                .into(),
        )
    };
    let entry = vfs.stat(uri)?;
    if entry.size.is_some_and(|s| s != original.len() as u64) {
        return Err(changed());
    }
    let mut r = vfs.open_read(uri)?;
    let mut buf = vec![0u8; CHUNK];
    let mut at = 0usize;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let end = at + n;
        if original.get(at..end) != buf.get(..n) {
            return Err(changed());
        }
        at = end;
    }
    if at != original.len() {
        return Err(changed());
    }
    Ok(())
}

fn decide(ctx: &mut OpCtx<'_>, dest: &VfsUri, size: u64, existing: &Entry) -> Choice {
    match ctx.policy {
        ConflictPolicy::Overwrite => Choice::Overwrite,
        ConflictPolicy::KeepBoth => Choice::KeepBoth,
        ConflictPolicy::Skip => Choice::Skip,
        ConflictPolicy::Ask => {
            let conflict = Conflict {
                incoming: ConflictSide {
                    uri: dest.clone(),
                    size: Some(size),
                    modified: None,
                },
                existing: ConflictSide {
                    uri: existing.uri.clone(),
                    size: existing.size,
                    modified: existing.modified,
                },
            };
            (ctx.on_conflict)(&conflict).choice
        }
    }
}

fn stream(
    vfs: &dyn fm_vfs::Vfs,
    to: &VfsUri,
    bytes: &[u8],
    ctx: &mut OpCtx<'_>,
    progress: &mut Progress,
) -> Result<(), FmError> {
    let mut w = vfs.open_write(to, WriteOpts::default())?;
    for chunk in bytes.chunks(CHUNK) {
        if ctx.cancel.is_cancelled() {
            return Err(FmError::Cancelled);
        }
        w.write_all(chunk)?;
        progress.bytes.done += chunk.len() as u64;
        (ctx.on_progress)(progress);
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::{Decision, SingleBackend};
    use std::sync::Arc;

    fn run(
        vfs: Arc<dyn fm_vfs::Vfs>,
        dest: &VfsUri,
        bytes: &[u8],
        replace: bool,
        answer: Choice,
    ) -> OpReport {
        let backends = SingleBackend(vfs);
        let mut on_progress = |_: &Progress| {};
        let mut on_conflict = |_: &Conflict| Decision {
            choice: answer,
            apply_to_all: false,
        };
        let mut ctx = OpCtx::new(ConflictPolicy::Ask, &mut on_progress, &mut on_conflict);
        write_bytes(&backends, dest, bytes, replace, &mut ctx)
    }

    #[test]
    fn writes_new_files_and_asks_on_conflict() {
        let tmp = tempfile::tempdir().expect("tmp");
        let vfs: Arc<dyn fm_vfs::Vfs> = Arc::new(fm_fs_local::LocalFs::new());
        let dest = VfsUri::parse(&tmp.path().join("out.pdf").to_string_lossy()).expect("uri");
        let r = run(vfs.clone(), &dest, b"one", false, Choice::Skip);
        assert_eq!(r.done, 1);
        assert_eq!(
            std::fs::read(tmp.path().join("out.pdf")).expect("read"),
            b"one"
        );
        // Existing: skip keeps it.
        let r = run(vfs.clone(), &dest, b"two", false, Choice::Skip);
        assert_eq!(r.skipped, 1);
        assert_eq!(
            std::fs::read(tmp.path().join("out.pdf")).expect("read"),
            b"one"
        );
        // Keep both writes next to it.
        let r = run(vfs.clone(), &dest, b"three", false, Choice::KeepBoth);
        assert_eq!(r.done, 1);
        assert_ne!(r.created[0], dest);
        // Confirmed replace overwrites without asking.
        let r = run(vfs.clone(), &dest, b"four", true, Choice::Cancel);
        assert_eq!(r.done, 1);
        assert_eq!(
            std::fs::read(tmp.path().join("out.pdf")).expect("read"),
            b"four"
        );
        // No temporary files are left behind.
        let names: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().all(|n| !n.starts_with(".fm")), "{names:?}");
    }

    #[test]
    fn free_names_and_change_checks() {
        let tmp = tempfile::tempdir().expect("tmp");
        let vfs: Arc<dyn fm_vfs::Vfs> = Arc::new(fm_fs_local::LocalFs::new());
        let backends = SingleBackend(vfs);
        let at = |n: &str| VfsUri::parse(&tmp.path().join(n).to_string_lossy()).expect("uri");
        let dest = at("form (filled).pdf");
        assert_eq!(free_name(&backends, &dest).expect("free"), dest);
        std::fs::write(tmp.path().join("form (filled).pdf"), b"x").expect("w");
        assert_eq!(
            free_name(&backends, &dest).expect("free"),
            at("form (filled) 2.pdf")
        );
        std::fs::write(tmp.path().join("orig.pdf"), b"abc").expect("w");
        assert!(verify_unchanged(&backends, &at("orig.pdf"), b"abc").is_ok());
        assert!(verify_unchanged(&backends, &at("orig.pdf"), b"abd").is_err());
        assert!(verify_unchanged(&backends, &at("orig.pdf"), b"ab").is_err());
    }

    #[test]
    fn failures_leave_the_target_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let vfs: Arc<dyn fm_vfs::Vfs> = Arc::new(fm_fs_local::LocalFs::new());
        let dest = VfsUri::parse(
            &tmp.path()
                .join("missing-dir")
                .join("x.pdf")
                .to_string_lossy(),
        )
        .expect("uri");
        let r = run(vfs, &dest, b"x", false, Choice::Skip);
        assert_eq!(r.failed.len(), 1);
    }
}
