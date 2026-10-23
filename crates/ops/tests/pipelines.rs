//! Behaviour of the copy/move/mkdir/rename pipelines and conflict handling,
//! driven through the public API only.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fm_ops::{
    CancelFlag, Choice, ConflictPolicy, Decision, OpCtx, OpReport, PlanLimits, Progress,
    SingleBackend, plan_transfer, transfer,
};
use fm_testkit::{FaultFs, MemFs};
use fm_vfs::{FmError, Vfs, VfsUri};

fn uri(s: &str) -> VfsUri {
    VfsUri::parse(s).expect("uri")
}

fn names(fs: &dyn Vfs, dir: &str) -> Vec<String> {
    let mut v: Vec<String> = fs
        .read_dir(&uri(dir))
        .expect("read_dir")
        .into_iter()
        .map(|e| e.name)
        .collect();
    v.sort();
    v
}

fn no_conflicts(_: &fm_ops::Conflict) -> Decision {
    Decision {
        choice: Choice::Cancel,
        apply_to_all: false,
    }
}

fn run(
    fs: Arc<dyn Vfs>,
    policy: ConflictPolicy,
    moving: bool,
    sources: &[&str],
    dest: &str,
) -> Result<OpReport, FmError> {
    let b = SingleBackend(fs);
    let mut progress = |_: &Progress| {};
    let mut conflict = no_conflicts;
    let mut ctx = OpCtx::new(policy, &mut progress, &mut conflict);
    let srcs: Vec<VfsUri> = sources.iter().map(|s| uri(s)).collect();
    if moving {
        fm_ops::move_(&b, &srcs, &uri(dest), &mut ctx)
    } else {
        fm_ops::copy(&b, &srcs, &uri(dest), &mut ctx)
    }
}

fn sample() -> MemFs {
    let fs = MemFs::new();
    fs.put_str("/src/a/one.txt", b"one").expect("put");
    fs.put_str("/src/a/b/two.txt", b"twotwo").expect("put");
    fs.put_str("/src/top.txt", b"top").expect("put");
    fs.mkdirs_str("/dst").expect("dst");
    fs
}

#[test]
fn copies_a_tree_and_reports_monotonic_progress() {
    let fs = sample();
    let b = SingleBackend(Arc::new(fs.clone()));
    let mut seen: Vec<Progress> = Vec::new();
    let mut progress = |p: &Progress| seen.push(p.clone());
    let mut conflict = no_conflicts;
    let mut ctx = OpCtx::new(ConflictPolicy::Ask, &mut progress, &mut conflict);
    let report = fm_ops::copy(
        &b,
        &[uri("/src/a"), uri("/src/top.txt")],
        &uri("/dst"),
        &mut ctx,
    )
    .expect("copy");
    assert!(report.is_clean());
    assert_eq!(fs.get_str("/dst/a/b/two.txt").expect("get"), b"twotwo");
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"top");
    assert_eq!(report.done, 5);
    assert_eq!(report.created, vec![uri("/dst/a"), uri("/dst/top.txt")]);
    assert_eq!(seen[0].items.total, 5);
    assert_eq!(seen[0].bytes.total, 3 + 6 + 3);
    assert_eq!(seen[0].bytes.done, 0);
    assert!(seen.windows(2).all(|w| w[0].bytes.done <= w[1].bytes.done));
    assert!(seen.windows(2).all(|w| w[0].items.done <= w[1].items.done));
    let last = seen.last().expect("progress");
    assert_eq!(last.items.done, last.items.total);
    assert_eq!(last.bytes.done, last.bytes.total);
}

#[test]
fn copy_between_backends_matches_byte_for_byte() {
    let mem = MemFs::new();
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    mem.put_str("/data/blob.bin", &payload).expect("put");
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().to_string_lossy().to_string();
    let local: Arc<dyn Vfs> = Arc::new(fm_fs_local::LocalFs::new());
    let memory: Arc<dyn Vfs> = Arc::new(mem);
    let prefix = root.clone();
    let resolver = move |u: &VfsUri| -> Result<Arc<dyn Vfs>, FmError> {
        Ok(if u.path_str().starts_with(&prefix) {
            local.clone()
        } else {
            memory.clone()
        })
    };
    let mut progress = |_: &Progress| {};
    let mut conflict = no_conflicts;
    let mut ctx = OpCtx::new(ConflictPolicy::Ask, &mut progress, &mut conflict);
    let report =
        fm_ops::copy(&resolver, &[uri("/data/blob.bin")], &uri(&root), &mut ctx).expect("copy");
    assert!(report.is_clean());
    let got = std::fs::read(dir.path().join("blob.bin")).expect("read");
    assert_eq!(got, payload);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("ls")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers, vec!["blob.bin".to_string()]);
}

#[test]
fn vanished_source_fails_that_item_only() {
    let fs = FaultFs::new(MemFs::new());
    fs.inner().put_str("/src/d/keep.txt", b"keep").expect("put");
    fs.inner().put_str("/src/d/gone.txt", b"gone").expect("put");
    fs.inner().mkdirs_str("/dst").expect("dst");
    let b = SingleBackend(Arc::new(fs.clone()));
    let plan =
        plan_transfer(&b, &[uri("/src/d")], &uri("/dst"), PlanLimits::default()).expect("plan");
    fs.faults().vanished.push(uri("/src/d/gone.txt"));
    let mut progress = |_: &Progress| {};
    let mut conflict = no_conflicts;
    let mut ctx = OpCtx::new(ConflictPolicy::Ask, &mut progress, &mut conflict);
    let report = transfer::run_copy(&b, &plan, &mut ctx);
    assert_eq!(report.failed.len(), 1);
    assert!(matches!(report.failed[0].1, FmError::NotFound(_)));
    assert_eq!(names(fs.inner(), "/dst/d"), vec!["keep.txt".to_string()]);
}

#[test]
fn cancel_at_every_tick_leaves_no_partial_files() {
    for cancel_at in 1..=8usize {
        let fs = FaultFs::new(MemFs::new());
        fs.inner()
            .put_str("/src/big.bin", &vec![9u8; 8 * 1024])
            .expect("put");
        fs.inner().mkdirs_str("/dst").expect("dst");
        let flag = CancelFlag::new();
        let ticks = Arc::new(AtomicUsize::new(0));
        {
            let (flag, ticks) = (flag.clone(), ticks.clone());
            let mut f = fs.faults();
            f.read_chunk = Some(1024);
            f.read_hook = Some(Arc::new(move || {
                if ticks.fetch_add(1, Ordering::SeqCst) + 1 == cancel_at {
                    flag.cancel();
                }
            }));
        }
        let b = SingleBackend(Arc::new(fs.clone()));
        let mut progress = |_: &Progress| {};
        let mut conflict = no_conflicts;
        let mut ctx =
            OpCtx::new(ConflictPolicy::Ask, &mut progress, &mut conflict).with_cancel(flag);
        let report =
            fm_ops::copy(&b, &[uri("/src/big.bin")], &uri("/dst"), &mut ctx).expect("copy");
        assert!(report.cancelled, "tick {cancel_at}");
        assert!(names(fs.inner(), "/dst").is_empty(), "tick {cancel_at}");
    }
}

#[test]
fn same_device_move_is_a_rename() {
    let fs = sample();
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        true,
        &["/src/a"],
        "/dst",
    )
    .expect("move");
    assert!(report.is_clean());
    assert!(!fs.exists(&uri("/src/a")));
    assert_eq!(fs.get_str("/dst/a/b/two.txt").expect("get"), b"twotwo");
    assert_eq!(report.moved, vec![(uri("/src/a"), uri("/dst/a"))]);
}

#[test]
fn cross_device_move_copies_verifies_then_deletes() {
    let fs = FaultFs::new(sample());
    fs.faults().same_device = Some(false);
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        true,
        &["/src/a"],
        "/dst",
    )
    .expect("move");
    assert!(report.is_clean(), "{:?}", report.failed);
    assert!(!fs.inner().exists(&uri("/src/a")));
    assert_eq!(fs.inner().get_str("/dst/a/one.txt").expect("get"), b"one");
    assert_eq!(report.moved.len(), 1);
}

#[test]
fn rename_unsupported_falls_back_to_copy_and_delete() {
    let fs = FaultFs::new(sample());
    fs.faults().rename_unsupported = true;
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        true,
        &["/src/top.txt"],
        "/dst",
    )
    .expect("move");
    assert!(report.is_clean(), "{:?}", report.failed);
    assert!(!fs.inner().exists(&uri("/src/top.txt")));
    assert_eq!(fs.inner().get_str("/dst/top.txt").expect("get"), b"top");
    assert_eq!(names(fs.inner(), "/dst"), vec!["top.txt".to_string()]);
}

#[test]
fn failed_cross_device_move_keeps_the_source() {
    let fs = FaultFs::new(sample());
    {
        let mut f = fs.faults();
        f.same_device = Some(false);
        f.fail_write_after = Some(2);
    }
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        true,
        &["/src/a"],
        "/dst",
    )
    .expect("move");
    assert!(!report.failed.is_empty());
    assert!(report.moved.is_empty());
    assert_eq!(fs.inner().get_str("/src/a/one.txt").expect("src"), b"one");
    assert_eq!(
        fs.inner().get_str("/src/a/b/two.txt").expect("src"),
        b"twotwo"
    );
    assert!(!fs.inner().exists(&uri("/dst/a/one.txt")));
    assert!(!fs.inner().exists(&uri("/dst/a/b/two.txt")));
    assert!(
        names(fs.inner(), "/dst/a")
            .iter()
            .all(|n| !n.starts_with(".fm"))
    );
}

#[test]
fn copying_into_itself_is_rejected_before_any_write() {
    let fs = sample();
    let before = fs.len();
    let got = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        false,
        &["/src/a"],
        "/src/a/b",
    );
    assert!(matches!(got, Err(FmError::InvalidParams(_))));
    assert_eq!(fs.len(), before);
}

#[test]
fn copying_onto_itself_is_a_silent_noop() {
    let fs = sample();
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        false,
        &["/src/top.txt"],
        "/src",
    )
    .expect("copy");
    assert_eq!(report.done, 0);
    assert!(report.is_clean());
    assert_eq!(fs.get_str("/src/top.txt").expect("get"), b"top");
}

#[test]
fn create_file_makes_empty_file_and_never_overwrites() {
    let fs = sample();
    let b = SingleBackend(Arc::new(fs.clone()));
    for bad in ["", "..", "a/b", "x\0y"] {
        assert!(matches!(
            fm_ops::create_file(&b, &uri("/dst"), bad),
            Err(FmError::InvalidParams(_))
        ));
    }
    let made = fm_ops::create_file(&b, &uri("/dst"), "new.txt").expect("create");
    assert_eq!(made, uri("/dst/new.txt"));
    assert_eq!(fs.get_str("/dst/new.txt").expect("get"), b"");
    fs.put_str("/dst/keep.txt", b"data").expect("put");
    assert!(matches!(
        fm_ops::create_file(&b, &uri("/dst"), "keep.txt"),
        Err(FmError::AlreadyExists(_))
    ));
    assert_eq!(fs.get_str("/dst/keep.txt").expect("get"), b"data");
}

#[test]
fn mkdir_and_rename_validate_names() {
    let fs = sample();
    let b = SingleBackend(Arc::new(fs.clone()));
    for bad in ["", "..", "a/b", "x\0y"] {
        assert!(matches!(
            fm_ops::mkdir(&b, &uri("/dst"), bad),
            Err(FmError::InvalidParams(_))
        ));
        assert!(matches!(
            fm_ops::rename(&b, &uri("/src/top.txt"), bad),
            Err(FmError::InvalidParams(_))
        ));
    }
    let made = fm_ops::mkdir(&b, &uri("/dst"), "new").expect("mkdir");
    assert_eq!(made, uri("/dst/new"));
    assert!(matches!(
        fm_ops::mkdir(&b, &uri("/dst"), "new"),
        Err(FmError::AlreadyExists(_))
    ));
    fs.put_str("/src/other.txt", b"o").expect("put");
    assert!(matches!(
        fm_ops::rename(&b, &uri("/src/top.txt"), "other.txt"),
        Err(FmError::AlreadyExists(_))
    ));
    let renamed = fm_ops::rename(&b, &uri("/src/top.txt"), "renamed.txt").expect("rename");
    assert_eq!(renamed, uri("/src/renamed.txt"));
    assert!(!fs.exists(&uri("/src/top.txt")));
    assert_eq!(fs.get_str("/src/other.txt").expect("get"), b"o");
}

// --- conflicts -------------------------------------------------------------

fn conflicting() -> MemFs {
    let fs = sample();
    fs.put_str("/dst/top.txt", b"OLD").expect("put");
    fs
}

fn ask_with(fs: &MemFs, sources: &[&str], choice: Choice, apply_to_all: bool) -> (OpReport, usize) {
    let b = SingleBackend(Arc::new(fs.clone()));
    let asked = AtomicUsize::new(0);
    let mut progress = |_: &Progress| {};
    let mut conflict = |_: &fm_ops::Conflict| {
        asked.fetch_add(1, Ordering::SeqCst);
        Decision {
            choice,
            apply_to_all,
        }
    };
    let mut ctx = OpCtx::new(ConflictPolicy::Ask, &mut progress, &mut conflict);
    let srcs: Vec<VfsUri> = sources.iter().map(|s| uri(s)).collect();
    let report = fm_ops::copy(&b, &srcs, &uri("/dst"), &mut ctx).expect("copy");
    (report, asked.load(Ordering::SeqCst))
}

#[test]
fn nothing_is_overwritten_without_a_decision() {
    let fs = conflicting();
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Skip,
        false,
        &["/src/top.txt"],
        "/dst",
    )
    .expect("copy");
    assert_eq!(report.skipped, 1);
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"OLD");

    let fs = conflicting();
    let (report, asked) = ask_with(&fs, &["/src/top.txt"], Choice::Skip, false);
    assert_eq!((report.skipped, asked), (1, 1));
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"OLD");
}

#[test]
fn non_ask_policies_never_call_the_dialog() {
    let fs = conflicting();
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Overwrite,
        false,
        &["/src/top.txt"],
        "/dst",
    )
    .expect("copy");
    assert!(report.is_clean());
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"top");
}

#[test]
fn ask_asks_and_overwrite_replaces_without_leftovers() {
    let fs = conflicting();
    let (report, asked) = ask_with(&fs, &["/src/top.txt"], Choice::Overwrite, false);
    assert_eq!(asked, 1);
    assert!(report.is_clean());
    assert!(report.created.is_empty());
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"top");
    assert_eq!(names(&fs, "/dst"), vec!["top.txt".to_string()]);
}

#[test]
fn keep_both_writes_a_numbered_sibling() {
    let fs = conflicting();
    let (report, _) = ask_with(&fs, &["/src/top.txt"], Choice::KeepBoth, false);
    assert_eq!(report.created, vec![uri("/dst/top (2).txt")]);
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"OLD");
    assert_eq!(fs.get_str("/dst/top (2).txt").expect("get"), b"top");
}

#[test]
fn apply_to_all_asks_once() {
    let fs = sample();
    for n in 0..4 {
        fs.put_str(&format!("/src/m/f{n}.txt"), b"new")
            .expect("put");
        fs.put_str(&format!("/dst/m/f{n}.txt"), b"old")
            .expect("put");
    }
    let (report, asked) = ask_with(&fs, &["/src/m"], Choice::Skip, true);
    assert_eq!(asked, 1);
    assert_eq!(report.skipped, 4);
    assert_eq!(fs.get_str("/dst/m/f3.txt").expect("get"), b"old");
}

#[test]
fn cancel_decision_ends_the_job() {
    let fs = conflicting();
    fs.put_str("/src/zzz.txt", b"z").expect("put");
    let (report, _) = ask_with(
        &fs,
        &["/src/top.txt", "/src/zzz.txt"],
        Choice::Cancel,
        false,
    );
    assert!(report.cancelled);
    assert!(!fs.exists(&uri("/dst/zzz.txt")));
    assert_eq!(fs.get_str("/dst/top.txt").expect("get"), b"OLD");
    assert_eq!(names(&fs, "/dst"), vec!["top.txt".to_string()]);
}

#[test]
fn failed_overwrite_keeps_the_old_destination() {
    let fs = FaultFs::new(conflicting());
    fs.faults().fail_write_after = Some(1);
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Overwrite,
        false,
        &["/src/top.txt"],
        "/dst",
    )
    .expect("copy");
    assert_eq!(report.failed.len(), 1);
    assert_eq!(fs.inner().get_str("/dst/top.txt").expect("get"), b"OLD");
    assert_eq!(names(fs.inner(), "/dst"), vec!["top.txt".to_string()]);
}

#[test]
fn existing_directory_in_the_way_is_a_failure_not_an_overwrite() {
    let fs = sample();
    fs.mkdirs_str("/dst/top.txt").expect("dir");
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Overwrite,
        false,
        &["/src/top.txt"],
        "/dst",
    )
    .expect("copy");
    assert!(matches!(report.failed[0].1, FmError::AlreadyExists(_)));
}

#[test]
fn backends_without_rename_still_copy_in_place() {
    let fs = FaultFs::new(sample());
    fs.faults().rename_unsupported = true;
    let report = run(
        Arc::new(fs.clone()),
        ConflictPolicy::Ask,
        false,
        &["/src/a"],
        "/dst",
    )
    .expect("copy");
    assert!(report.is_clean(), "{:?}", report.failed);
    assert_eq!(
        fs.inner().get_str("/dst/a/b/two.txt").expect("get"),
        b"twotwo"
    );
    assert!(
        names(fs.inner(), "/dst/a")
            .iter()
            .all(|n| !n.starts_with(".fm"))
    );
}
