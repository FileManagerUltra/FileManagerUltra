//! Symbolic links through copy/move: recreated as links, never followed, optionally
//! re-targeted when they point inside the copied tree (capability `security`).
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use fm_ops::{Choice, ConflictPolicy, Decision, OpCtx, OpReport, Progress, SingleBackend};
use fm_testkit::MemFs;
use fm_vfs::{EntryKind, Vfs, VfsUri};

fn uri(s: &str) -> VfsUri {
    VfsUri::parse(s).expect("uri")
}

fn copy(fs: &MemFs, source: &str, dest: &str, retarget: bool, policy: ConflictPolicy) -> OpReport {
    let b = SingleBackend(Arc::new(fs.clone()));
    let mut progress = |_: &Progress| {};
    let mut conflict = |_: &fm_ops::Conflict| Decision {
        choice: Choice::Cancel,
        apply_to_all: false,
    };
    let mut ctx = OpCtx::new(policy, &mut progress, &mut conflict).with_retarget_links(retarget);
    fm_ops::copy(&b, &[uri(source)], &uri(dest), &mut ctx).expect("copy")
}

fn link_target(fs: &MemFs, path: &str) -> String {
    match fs.stat(&uri(path)).expect("stat").kind {
        EntryKind::Symlink { target } => target,
        other => panic!("{path} is {other:?}, expected a symlink"),
    }
}

fn tree() -> MemFs {
    let fs = MemFs::new();
    fs.put_str("/src/proj/lib/a.js", b"code").expect("put");
    fs.put_symlink(&uri("/src/proj/bin/rel"), "../lib/a.js")
        .expect("rel");
    fs.put_symlink(&uri("/src/proj/bin/abs"), "/src/proj/lib/a.js")
        .expect("abs");
    fs.put_symlink(&uri("/src/proj/bin/out"), "/etc/passwd")
        .expect("out");
    fs.put_symlink(&uri("/src/proj/bin/dangling"), "/src/proj/missing")
        .expect("dangling");
    fs.mkdirs_str("/dst").expect("dst");
    fs
}

#[test]
fn links_are_recreated_as_links_and_retargeted_inside_the_tree() {
    let fs = tree();
    let report = copy(&fs, "/src/proj", "/dst", true, ConflictPolicy::Ask);
    assert!(report.is_clean(), "{:?}", report.failed);
    assert_eq!(link_target(&fs, "/dst/proj/bin/rel"), "../lib/a.js");
    assert_eq!(
        link_target(&fs, "/dst/proj/bin/abs"),
        "/dst/proj/lib/a.js",
        "inside the copied tree: follows the copy"
    );
    assert_eq!(
        link_target(&fs, "/dst/proj/bin/out"),
        "/etc/passwd",
        "outside the tree: untouched"
    );
    assert_eq!(
        link_target(&fs, "/dst/proj/bin/dangling"),
        "/dst/proj/missing"
    );
    assert_eq!(fs.get_str("/dst/proj/lib/a.js").expect("file"), b"code");
}

#[test]
fn retargeting_off_copies_the_target_text_verbatim() {
    let fs = tree();
    let report = copy(&fs, "/src/proj", "/dst", false, ConflictPolicy::Ask);
    assert!(report.is_clean(), "{:?}", report.failed);
    assert_eq!(link_target(&fs, "/dst/proj/bin/abs"), "/src/proj/lib/a.js");
    assert_eq!(link_target(&fs, "/dst/proj/bin/rel"), "../lib/a.js");
}

#[test]
fn links_are_never_followed() {
    let fs = tree();
    copy(&fs, "/src/proj", "/dst", true, ConflictPolicy::Ask);
    let kind = fs.stat(&uri("/dst/proj/bin/out")).expect("stat").kind;
    assert!(matches!(kind, EntryKind::Symlink { .. }));
    assert!(!fs.exists(&uri("/dst/proj/bin/out/passwd")));
}

#[test]
fn a_copied_link_alone_keeps_its_target() {
    let fs = tree();
    let report = copy(&fs, "/src/proj/bin/abs", "/dst", true, ConflictPolicy::Ask);
    assert!(report.is_clean(), "{:?}", report.failed);
    assert_eq!(link_target(&fs, "/dst/abs"), "/src/proj/lib/a.js");
}

#[test]
fn existing_links_follow_the_conflict_policy() {
    let fs = tree();
    fs.put_symlink(&uri("/dst/proj/bin/rel"), "old")
        .expect("existing");
    let skipped = copy(&fs, "/src/proj", "/dst", true, ConflictPolicy::Skip);
    assert_eq!(skipped.skipped, 1);
    assert_eq!(link_target(&fs, "/dst/proj/bin/rel"), "old");

    let replaced = copy(&fs, "/src/proj", "/dst", true, ConflictPolicy::Overwrite);
    assert!(replaced.is_clean(), "{:?}", replaced.failed);
    assert_eq!(link_target(&fs, "/dst/proj/bin/rel"), "../lib/a.js");
}
