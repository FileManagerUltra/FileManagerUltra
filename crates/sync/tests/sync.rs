//! End-to-end sync behaviour on real temp folders and in-memory trees.
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fm_fs_local::LocalFs;
use fm_ops::CancelFlag;
use fm_sync::{
    Action, CompareMethod, DeletePolicy, Plan, Profile, Retention, RunOptions, RunReport,
    SnapshotStore, SyncMode, compare, run,
};
use fm_testkit::MemFs;
use fm_vfs::{AttrSet, Vfs};

struct Env {
    _tmp: tempfile::TempDir,
    left: PathBuf,
    right: PathBuf,
    store: SnapshotStore,
    profile: Profile,
}

fn env(mode: SyncMode) -> Env {
    let tmp = tempfile::tempdir().expect("tmp");
    let (left, right) = (tmp.path().join("left"), tmp.path().join("right"));
    fs::create_dir(&left).expect("mkdir");
    fs::create_dir(&right).expect("mkdir");
    let store = SnapshotStore::new(tmp.path().join("state"));
    let profile = Profile {
        id: "p1".into(),
        name: "test".into(),
        left: left.to_string_lossy().into_owned(),
        right: right.to_string_lossy().into_owned(),
        mode,
        // Never the real trash from a test.
        delete: Some(DeletePolicy::Permanent),
        ..Profile::default()
    };
    Env {
        _tmp: tmp,
        left,
        right,
        store,
        profile,
    }
}

fn vfs() -> Arc<dyn Vfs> {
    Arc::new(LocalFs::new())
}

fn plan_for(e: &Env) -> Plan {
    compare(
        &e.profile,
        &vfs(),
        &vfs(),
        Some(&e.store),
        &CancelFlag::new(),
        &mut |_, _| {},
    )
    .expect("compare")
}

fn opts(store: Option<&SnapshotStore>, confirmed: bool, now: SystemTime) -> RunOptions<'_> {
    RunOptions {
        confirmed,
        now,
        cancel: CancelFlag::new(),
        store,
    }
}

fn apply_at(e: &Env, plan: &Plan, now: SystemTime) -> RunReport {
    run(
        plan,
        vfs(),
        vfs(),
        opts(Some(&e.store), true, now),
        &mut |_| {},
    )
    .expect("run")
}

fn apply(e: &Env, plan: &Plan) -> RunReport {
    apply_at(e, plan, SystemTime::now())
}

fn write(root: &Path, rel: &str, data: &[u8], secs: u64) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().expect("parent")).expect("mkdirs");
    fs::write(&p, data).expect("write");
    let f = fs::OpenOptions::new().write(true).open(&p).expect("open");
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .expect("mtime");
}

fn read(root: &Path, rel: &str) -> Option<Vec<u8>> {
    fs::read(root.join(rel)).ok()
}

fn action(plan: &Plan, rel: &str) -> Action {
    plan.find(rel).expect("item").action
}

const T: u64 = 1_700_000_000;

#[test]
fn mirror_copies_updates_deletes_and_keeps_times() {
    let mut e = env(SyncMode::Mirror);
    e.profile.attrs = AttrSet::TIMES;
    write(&e.left, "new.txt", b"new", T);
    write(&e.left, "d/changed.txt", b"v2", T + 100);
    write(&e.left, "same.txt", b"same", T);
    write(&e.right, "d/changed.txt", b"v1", T);
    write(&e.right, "same.txt", b"same", T);
    write(&e.right, "extra.txt", b"x", T);

    let plan = plan_for(&e);
    assert_eq!(action(&plan, "new.txt"), Action::CopyToRight);
    assert_eq!(action(&plan, "d/changed.txt"), Action::CopyToRight);
    assert_eq!(action(&plan, "same.txt"), Action::None);
    assert_eq!(action(&plan, "extra.txt"), Action::DeleteRight);
    let report = apply(&e, &plan);
    assert!(report.failed.is_empty(), "{:?}", report.failed);

    assert_eq!(read(&e.right, "new.txt").as_deref(), Some(&b"new"[..]));
    assert_eq!(read(&e.right, "d/changed.txt").as_deref(), Some(&b"v2"[..]));
    assert_eq!(read(&e.right, "extra.txt"), None);
    let mtime = fs::metadata(e.right.join("d/changed.txt"))
        .and_then(|m| m.modified())
        .expect("mtime");
    assert_eq!(mtime, UNIX_EPOCH + Duration::from_secs(T + 100));
    assert!(plan_for(&e).is_empty_work(), "second run has nothing to do");
}

#[test]
fn permanent_deletes_need_confirmation_and_nothing_runs_without_it() {
    let e = env(SyncMode::Mirror);
    write(&e.left, "a.txt", b"a", T);
    write(&e.right, "gone.txt", b"g", T);
    let plan = plan_for(&e);
    assert!(plan.guards.confirmation_required());
    let r = run(
        &plan,
        vfs(),
        vfs(),
        opts(None, false, SystemTime::now()),
        &mut |_| {},
    );
    assert!(r.is_err());
    assert!(read(&e.right, "gone.txt").is_some());
    assert!(read(&e.right, "a.txt").is_none(), "nothing ran");
}

#[test]
fn an_empty_source_trips_the_guard() {
    let mut e = env(SyncMode::Mirror);
    e.profile.delete = Some(DeletePolicy::Versioning {
        folder: e._tmp.path().join("ver").to_string_lossy().into_owned(),
        retention: Retention::default(),
    });
    for i in 0..4 {
        write(&e.right, &format!("f{i}"), b"x", T);
    }
    let plan = plan_for(&e);
    assert!(plan.guards.empty_side && plan.guards.over_limit);
    assert!(plan.guards.confirmation_required());
}

#[test]
fn update_never_deletes_and_leaves_newer_targets_alone() {
    let e = env(SyncMode::Update);
    write(&e.left, "old.txt", b"left", T);
    write(&e.right, "old.txt", b"right-edit", T + 500);
    write(&e.right, "only-right.txt", b"r", T);
    write(&e.left, "fresh.txt", b"f", T);
    let plan = plan_for(&e);
    assert_eq!(action(&plan, "old.txt"), Action::Skip);
    assert_eq!(action(&plan, "only-right.txt"), Action::None);
    assert_eq!(plan.guards.deletes, 0);
    apply(&e, &plan);
    assert_eq!(
        read(&e.right, "old.txt").as_deref(),
        Some(&b"right-edit"[..])
    );
    assert!(read(&e.right, "fresh.txt").is_some());
    assert!(read(&e.right, "only-right.txt").is_some());
}

#[test]
fn two_way_merges_then_propagates_changes_and_deletes_and_skips_conflicts() {
    let e = env(SyncMode::TwoWay);
    write(&e.left, "l.txt", b"l", T);
    write(&e.right, "r.txt", b"r", T);
    for f in ["edit.txt", "del.txt", "both.txt"] {
        write(&e.left, f, b"1", T);
        write(&e.right, f, b"1", T);
    }

    // First run has no history: a merge that copies missing files and deletes nothing.
    let plan = plan_for(&e);
    assert_eq!(action(&plan, "l.txt"), Action::CopyToRight);
    assert_eq!(action(&plan, "r.txt"), Action::CopyToLeft);
    assert_eq!(plan.guards.deletes, 0);
    let rep = apply(&e, &plan);
    assert!(rep.snapshot_saved);
    assert!(read(&e.left, "r.txt").is_some() && read(&e.right, "l.txt").is_some());

    write(&e.left, "edit.txt", b"22", T + 50);
    fs::remove_file(e.right.join("del.txt")).expect("rm");
    write(&e.right, "new-r.txt", b"n", T + 60);
    write(&e.left, "both.txt", b"left!", T + 70);
    write(&e.right, "both.txt", b"right", T + 80);

    let plan = plan_for(&e);
    assert_eq!(action(&plan, "edit.txt"), Action::CopyToRight);
    assert_eq!(action(&plan, "del.txt"), Action::DeleteLeft);
    assert_eq!(action(&plan, "new-r.txt"), Action::CopyToLeft);
    assert_eq!(
        action(&plan, "both.txt"),
        Action::Skip,
        "conflicts skip by default"
    );
    assert_eq!(plan.summary.conflicts, 1);
    apply(&e, &plan);
    assert_eq!(read(&e.right, "edit.txt").as_deref(), Some(&b"22"[..]));
    assert_eq!(read(&e.left, "del.txt"), None);
    assert_eq!(read(&e.left, "both.txt").as_deref(), Some(&b"left!"[..]));
    assert_eq!(read(&e.right, "both.txt").as_deref(), Some(&b"right"[..]));

    // The conflict is still pending, everything else settled.
    let plan = plan_for(&e);
    assert_eq!(plan.summary.conflicts, 1);
    assert!(
        plan.items
            .iter()
            .filter(|i| i.rel != "both.txt")
            .all(|i| !i.action.is_copy() && !i.action.is_delete())
    );
}

#[test]
fn a_run_that_died_before_saving_history_never_causes_deletes() {
    let e = env(SyncMode::TwoWay);
    write(&e.left, "a.txt", b"a", T);
    apply(&e, &plan_for(&e));
    // The next run copies a new file but "crashes" before the snapshot is written.
    write(&e.left, "b.txt", b"b", T);
    let plan = plan_for(&e);
    run(
        &plan,
        vfs(),
        vfs(),
        opts(None, true, SystemTime::now()),
        &mut |_| {},
    )
    .expect("run");
    let plan = plan_for(&e);
    assert_eq!(plan.guards.deletes, 0);
    assert!(plan.is_empty_work(), "b.txt is simply equal now");
}

#[test]
fn versioning_archives_deleted_files_and_prunes_by_count() {
    let mut e = env(SyncMode::Mirror);
    let ver = e._tmp.path().join("ver");
    e.profile.delete = Some(DeletePolicy::Versioning {
        folder: ver.to_string_lossy().into_owned(),
        retention: Retention {
            max_versions: Some(2),
            ..Retention::default()
        },
    });
    write(&e.left, "keep.txt", b"k", T);
    write(&e.right, "keep.txt", b"k", T);
    for (i, day) in [1u64, 2, 3].into_iter().enumerate() {
        let rel = format!("d/gone{i}.txt");
        write(&e.right, &rel, format!("v{i}").as_bytes(), T);
        let plan = plan_for(&e);
        assert_eq!(action(&plan, &rel), Action::DeleteRight);
        let now = UNIX_EPOCH + Duration::from_secs(T + day * 86_400);
        let rep = apply_at(&e, &plan, now);
        assert!(rep.failed.is_empty(), "{:?}", rep.failed);
        assert_eq!(read(&e.right, &rel), None);
    }
    let mut runs: Vec<String> = fs::read_dir(&ver)
        .expect("ver")
        .map(|d| d.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    runs.sort();
    assert_eq!(runs.len(), 2, "oldest run pruned: {runs:?}");
    let newest = ver.join(runs.last().expect("run"));
    assert_eq!(read(&newest, "d/gone2.txt").as_deref(), Some(&b"v2"[..]));
}

#[test]
fn filters_keep_excluded_paths_out_of_the_plan() {
    let mut e = env(SyncMode::Mirror);
    e.profile.filters.exclude = vec!["*.tmp".into(), "cache/".into()];
    write(&e.left, "a.txt", b"a", T);
    write(&e.left, "x.tmp", b"t", T);
    write(&e.left, "cache/c.bin", b"c", T);
    write(&e.right, "keep.tmp", b"stay", T);
    let plan = plan_for(&e);
    assert!(plan.find("x.tmp").is_none() && plan.find("cache/c.bin").is_none());
    assert!(
        plan.find("keep.tmp").is_none(),
        "excluded paths are never deleted"
    );
    assert_eq!(plan.guards.deletes, 0);
}

#[test]
fn checksum_catches_same_size_and_time_with_other_content() {
    let mut e = env(SyncMode::Mirror);
    e.profile.compare = CompareMethod::ChecksumAlways;
    write(&e.left, "f.bin", b"AAAA", T);
    write(&e.right, "f.bin", b"BBBB", T);
    assert_eq!(action(&plan_for(&e), "f.bin"), Action::CopyToRight);
    e.profile.compare = CompareMethod::SizeMtime;
    assert_eq!(action(&plan_for(&e), "f.bin"), Action::None);
}

#[test]
fn row_overrides_are_validated() {
    let e = env(SyncMode::Mirror);
    write(&e.left, "a.txt", b"a", T);
    write(&e.right, "b.txt", b"b", T);
    let mut plan = plan_for(&e);
    assert!(plan.set_action("a.txt", Action::Skip).is_ok());
    assert!(
        plan.set_action("a.txt", Action::CopyToLeft).is_err(),
        "never writes the source"
    );
    assert!(
        plan.set_action("a.txt", Action::DeleteRight).is_err(),
        "nothing to delete there"
    );
    assert!(plan.set_action("nope", Action::Skip).is_err());
    plan.set_action("b.txt", Action::None)
        .expect("keep the extra file");
    assert_eq!(plan.guards.deletes, 0);
}

#[cfg(unix)]
#[test]
fn an_unreadable_folder_is_unknown_not_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let e = env(SyncMode::Mirror);
    write(&e.left, "locked/inside.txt", b"i", T);
    write(&e.right, "locked/inside.txt", b"i", T);
    write(&e.right, "locked/only-right.txt", b"r", T);
    let locked = e.left.join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    let readable = fs::read_dir(&locked).is_ok();
    let plan = plan_for(&e);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
    if readable {
        return; // running as root: permissions don't apply
    }
    assert_eq!(plan.guards.deletes, 0);
    assert_eq!(action(&plan, "locked/only-right.txt"), Action::Skip);
}

#[test]
fn hostile_names_and_big_trees_copy_without_surprises() {
    let src = fm_testkit::hostile_tree("/l", 300);
    let dst = MemFs::new();
    dst.mkdirs_str("/r").expect("mkdir");
    let (src, dst): (Arc<dyn Vfs>, Arc<dyn Vfs>) = (Arc::new(src), Arc::new(dst));
    let profile = Profile {
        id: "m".into(),
        name: "mem".into(),
        left: "/l".into(),
        right: "/r".into(),
        mode: SyncMode::Mirror,
        ..Profile::default()
    };
    let plan = compare(
        &profile,
        &src,
        &dst,
        None,
        &CancelFlag::new(),
        &mut |_, _| {},
    )
    .expect("compare");
    assert!(
        plan.items
            .iter()
            .all(|i| !i.rel.split('/').any(|s| s == ".." || s.is_empty()))
    );
    let rep = run(
        &plan,
        src,
        dst,
        opts(None, true, SystemTime::now()),
        &mut |_| {},
    )
    .expect("run");
    assert!(rep.failed.is_empty(), "{:?}", rep.failed);
    assert!(rep.copied >= 300);
}

#[test]
fn a_missing_target_folder_is_created_by_the_first_sync() {
    let mut e = env(SyncMode::Mirror);
    fs::remove_dir(&e.right).expect("rm");
    e.right = e.right.join("deep").join("target");
    e.profile.right = e.right.to_string_lossy().into_owned();
    write(&e.left, "a/b.txt", b"hi", T);
    let plan = plan_for(&e);
    assert_eq!(action(&plan, "a/b.txt"), Action::CopyToRight);
    let report = apply(&e, &plan);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(read(&e.right, "a/b.txt").as_deref(), Some(&b"hi"[..]));
}

#[test]
fn a_missing_source_folder_is_an_error_not_a_creation() {
    let mut e = env(SyncMode::Mirror);
    fs::remove_dir(&e.left).expect("rm");
    e.profile.left = e.left.to_string_lossy().into_owned();
    let res = compare(
        &e.profile,
        &vfs(),
        &vfs(),
        Some(&e.store),
        &CancelFlag::new(),
        &mut |_, _| {},
    );
    assert!(res.is_err());
    assert!(!e.left.exists());
}
