//! Saving archives stored as numbered parts, and recovering from an interrupted save
//! (`archive-editing`, design D16). The new archive is cut into temporary parts of the
//! original size next to the old ones; the swap then moves every old part aside as a
//! backup, puts the new parts in place and removes the backups. Any failure puts the
//! backups back; a crash leaves them beside the parts for the next mount to find.

use std::io::Write;
use std::path::Path;

use fm_archive::split::{self, PartSet};
use fm_vfs::{FmError, Vfs, VfsUri, WriteOpts};

use crate::archive_ops::copy_cancellable;

/// What to do with the backup parts an interrupted save left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupAction {
    /// Put the previous version of the archive back.
    Restore,
    /// Delete the backups and keep the parts that are in place.
    Discard,
}

/// How many parts `total` bytes make at `size` bytes a part (one when there is no size).
pub(crate) fn part_count(total: u64, size: Option<u64>) -> u64 {
    size.map_or(1, |s| total.div_ceil(s.max(1)).max(1))
}

fn remove_all(vfs: &dyn Vfs, uris: &[VfsUri]) {
    for u in uris {
        let _ = vfs.remove(u);
    }
}

/// Write `from` as temporary parts named after `names`, `size` bytes each (the last
/// takes the rest). Each part is checked by size once written; on any failure the
/// temporary parts are removed.
pub(crate) fn write_temp_parts(
    vfs: &dyn Vfs,
    dir: &VfsUri,
    names: &[String],
    from: &Path,
    size: Option<u64>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<VfsUri>, FmError> {
    let mut src = std::fs::File::open(from)?;
    let mut left = src.metadata()?.len();
    let mut temps = Vec::with_capacity(names.len());
    let result = (|| -> Result<(), FmError> {
        for name in names {
            let uri = dir.join(&split::temp_name(name))?;
            temps.push(uri.clone());
            let want = size.map_or(left, |s| s.min(left));
            let mut dst = vfs.open_write(&uri, WriteOpts::default())?;
            let n = copy_cancellable(&mut src, &mut dst, want, cancelled)?;
            dst.flush()?;
            drop(dst);
            if n != want || vfs.stat(&uri)?.size != Some(n) {
                return Err(FmError::backend(
                    "archive",
                    format!("{name} was not written completely"),
                ));
            }
            left -= n;
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(temps),
        Err(e) => {
            remove_all(vfs, &temps);
            Err(e)
        }
    }
}

/// Put the backups back after a failed swap and say what happened.
fn rolled_back(
    vfs: &dyn Vfs,
    aside: &[(VfsUri, VfsUri)],
    placed: &[VfsUri],
    failed: &str,
    cause: &FmError,
) -> FmError {
    remove_all(vfs, placed);
    let mut stuck = Vec::new();
    for (original, backup) in aside.iter().rev() {
        if vfs.rename(backup, original).is_err() {
            stuck.push(backup.file_name().unwrap_or("backup").to_string());
        }
    }
    let text = if stuck.is_empty() {
        format!(
            "Could not save: {failed} could not be replaced, the previous parts are back ({cause})"
        )
    } else {
        format!(
            "Could not save: {failed} could not be replaced and these previous parts are kept \
             under their backup names: {}. Use Restore Previous Version to put them back \
             ({cause})",
            stuck.join(", ")
        )
    };
    FmError::backend("archive", text)
}

/// Replace the set `old` in `dir` by the set `new` (whose parts wait at `temps`).
/// Old parts that `new` does not reuse (a smaller set) are removed last, with the
/// backups. On failure the old set is restored and the temporary parts stay for the
/// caller to remove.
pub(crate) fn swap_parts(
    vfs: &dyn Vfs,
    dir: &VfsUri,
    old: &[String],
    new: &[String],
    temps: &[VfsUri],
) -> Result<(), FmError> {
    // Every name is built before the first rename, so a bad name changes nothing.
    // The last part goes aside first and the first part last: if a crash cuts the swap
    // short, the first part of the archive is the one that is missing for the shortest
    // time.
    let mut aside = Vec::with_capacity(old.len());
    for name in old.iter().rev() {
        aside.push((dir.join(name)?, dir.join(&split::backup_name(name))?));
    }
    let mut targets = Vec::with_capacity(new.len());
    for name in new {
        targets.push(dir.join(name)?);
    }
    let mut moved: Vec<(VfsUri, VfsUri)> = Vec::new();
    for (original, backup) in &aside {
        if let Err(e) = vfs.rename(original, backup) {
            let name = original.file_name().unwrap_or("a part");
            return Err(rolled_back(vfs, &moved, &[], name, &e));
        }
        moved.push((original.clone(), backup.clone()));
    }
    let mut placed: Vec<VfsUri> = Vec::new();
    for (temp, target) in temps.iter().zip(&targets) {
        if let Err(e) = vfs.rename(temp, target) {
            let name = target.file_name().unwrap_or("a part");
            return Err(rolled_back(vfs, &moved, &placed, name, &e));
        }
        placed.push(target.clone());
    }
    // A backup that cannot be removed stays; the next mount offers to clean it up.
    for (_, backup) in &moved {
        let _ = vfs.remove(backup);
    }
    Ok(())
}

/// Deal with the backups of an interrupted save in the folder of `first`.
/// `Restore` replaces whatever is there of the set by the backed-up parts; `Discard`
/// removes the backups, but only while the parts in place form a whole set.
pub(crate) fn recover(
    vfs: &dyn Vfs,
    dir: &VfsUri,
    first: &str,
    action: BackupAction,
) -> Result<u64, FmError> {
    let entries = vfs.read_dir(dir)?;
    let backups = split::backups_of(first, &entries);
    if backups.is_empty() {
        return Err(FmError::InvalidParams(
            "there are no backup parts for this archive".into(),
        ));
    }
    let parts = split::backed_up_parts(&backups).map_err(|_| {
        FmError::backend(
            "archive",
            "The backup parts are incomplete, so nothing was changed",
        )
    })?;
    match action {
        BackupAction::Discard => {
            PartSet::read(first, &entries).map_err(|_| {
                FmError::backend(
                    "archive",
                    "The parts in place are incomplete: restore the previous version instead",
                )
            })?;
            for b in &backups {
                vfs.remove(&dir.join(b)?)?;
            }
        }
        BackupAction::Restore => {
            let base = split::parse(first).map(|p| p.base).unwrap_or_default();
            for e in &entries {
                if split::parse(&e.name)
                    .is_some_and(|p| p.kind == split::SplitKind::Numbered && p.base == base)
                {
                    vfs.remove(&dir.join(&e.name)?)?;
                }
            }
            for part in &parts {
                vfs.rename(&dir.join(&split::backup_name(part))?, &dir.join(part)?)?;
            }
        }
    }
    Ok(backups.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_testkit::{FaultFs, MemFs};

    fn dir() -> VfsUri {
        VfsUri::parse("/set").expect("uri")
    }

    fn names(vfs: &dyn Vfs) -> Vec<String> {
        let mut v: Vec<String> = vfs
            .read_dir(&dir())
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        v.sort();
        v
    }

    fn part(n: u32) -> String {
        format!("a.7z.{n:03}")
    }

    /// A folder holding `count` old parts and as many temporary new ones.
    fn fixture(old: u32, new: u32) -> (FaultFs<MemFs>, Vec<String>, Vec<String>, Vec<VfsUri>) {
        let fs = FaultFs::new(MemFs::new());
        fs.inner().mkdirs_str("/set").expect("dir");
        let old_names: Vec<String> = (1..=old).map(part).collect();
        for n in &old_names {
            fs.inner()
                .put_str(&format!("/set/{n}"), format!("old-{n}").as_bytes())
                .expect("old");
        }
        let new_names: Vec<String> = (1..=new).map(part).collect();
        let mut temps = Vec::new();
        for n in &new_names {
            let t = split::temp_name(n);
            fs.inner()
                .put_str(&format!("/set/{t}"), format!("new-{n}").as_bytes())
                .expect("tmp");
            temps.push(dir().join(&t).expect("uri"));
        }
        (fs, old_names, new_names, temps)
    }

    fn content(fs: &FaultFs<MemFs>, name: &str) -> String {
        String::from_utf8(fs.inner().get_str(&format!("/set/{name}")).expect("read")).expect("utf8")
    }

    #[test]
    fn a_swap_replaces_the_set_and_removes_surplus_parts_last() {
        let (fs, old, new, temps) = fixture(5, 3);
        swap_parts(&fs, &dir(), &old, &new, &temps).expect("swap");
        assert_eq!(names(&fs), vec![part(1), part(2), part(3)]);
        assert_eq!(content(&fs, &part(2)), "new-a.7z.002");
    }

    #[test]
    fn a_larger_set_grows_beyond_the_old_parts() {
        let (fs, old, new, temps) = fixture(2, 4);
        swap_parts(&fs, &dir(), &old, &new, &temps).expect("swap");
        assert_eq!(names(&fs).len(), 4);
        assert_eq!(content(&fs, &part(4)), "new-a.7z.004");
    }

    #[test]
    fn a_failed_rename_at_any_step_restores_the_old_set() {
        // Five parts go aside (renames 1-5), then the new ones are placed (6-8).
        for failing in 1..=8 {
            let (fs, old, new, temps) = fixture(5, 3);
            fs.faults().fail_renames = Some(failing..=failing);
            let err = swap_parts(&fs, &dir(), &old, &new, &temps).expect_err("fails");
            assert!(err.to_string().contains("previous parts are back"), "{err}");
            fs.faults().fail_renames = None;
            // New parts already placed are removed again; the caller deletes the rest.
            let kept: Vec<String> = names(&fs)
                .into_iter()
                .filter(|n| !n.ends_with(".fmtmp"))
                .collect();
            assert_eq!(kept, old, "step {failing}");
            for n in &old {
                assert_eq!(content(&fs, n), format!("old-{n}"), "step {failing}");
            }
        }
    }

    #[test]
    fn a_failed_swap_names_the_part_and_keeps_stuck_backups_findable() {
        let (fs, old, new, temps) = fixture(3, 3);
        // The second new part fails, and so does every rename after it (the rollback).
        fs.faults().fail_renames = Some(5..=usize::MAX);
        let err = swap_parts(&fs, &dir(), &old, &new, &temps).expect_err("fails");
        let text = err.to_string();
        assert!(text.contains("a.7z.002"), "{text}");
        assert!(text.contains("backup names"), "{text}");
        fs.faults().fail_renames = None;
        let entries = fs.read_dir(&dir()).expect("list");
        assert!(!split::backups_of("a.7z.001", &entries).is_empty());
    }

    #[test]
    fn leftover_backups_are_restored_or_discarded() {
        // A crash after the new first part went in: new 1, backups of 1-3.
        let (fs, old, _, _) = fixture(3, 0);
        for n in &old {
            fs.rename(
                &dir().join(n).expect("u"),
                &dir().join(&split::backup_name(n)).expect("u"),
            )
            .expect("aside");
        }
        fs.inner().put_str("/set/a.7z.001", b"new").expect("new");
        let before = names(&fs);
        assert_eq!(before.len(), 4);

        recover(&fs, &dir(), "a.7z.001", BackupAction::Restore).expect("restore");
        assert_eq!(names(&fs), vec![part(1), part(2), part(3)]);
        assert_eq!(content(&fs, &part(1)), "old-a.7z.001");

        for n in &old {
            fs.rename(
                &dir().join(n).expect("u"),
                &dir().join(&split::backup_name(n)).expect("u"),
            )
            .expect("aside");
        }
        fs.inner().put_str("/set/a.7z.001", b"new").expect("new");
        recover(&fs, &dir(), "a.7z.001", BackupAction::Discard).expect("discard");
        assert_eq!(names(&fs), vec![part(1)]);
        assert_eq!(content(&fs, &part(1)), "new");
        assert!(recover(&fs, &dir(), "a.7z.001", BackupAction::Restore).is_err());
    }

    #[test]
    fn discard_refuses_while_the_parts_in_place_have_a_gap() {
        let (fs, old, _, _) = fixture(3, 0);
        for n in &old {
            fs.rename(
                &dir().join(n).expect("u"),
                &dir().join(&split::backup_name(n)).expect("u"),
            )
            .expect("aside");
        }
        // Only part 2 is in place: not a set.
        fs.inner().put_str("/set/a.7z.002", b"new").expect("new");
        let err = recover(&fs, &dir(), "a.7z.001", BackupAction::Discard).expect_err("gap");
        assert!(err.to_string().contains("incomplete"), "{err}");
        assert_eq!(names(&fs).len(), 4, "nothing was removed");
    }

    #[test]
    fn incomplete_backups_change_nothing() {
        let (fs, _, _, _) = fixture(0, 0);
        fs.inner().put_str("/set/a.7z.001", b"live").expect("live");
        fs.inner()
            .put_str(&format!("/set/{}", split::backup_name("a.7z.002")), b"b")
            .expect("backup");
        assert!(recover(&fs, &dir(), "a.7z.001", BackupAction::Restore).is_err());
        assert_eq!(content(&fs, "a.7z.001"), "live");
    }

    #[test]
    fn part_count_keeps_the_size_and_a_single_part_stays_one() {
        assert_eq!(part_count(300, Some(100)), 3);
        assert_eq!(part_count(301, Some(100)), 4);
        assert_eq!(part_count(0, Some(100)), 1);
        assert_eq!(part_count(5_000, None), 1);
    }
}
