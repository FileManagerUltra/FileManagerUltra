#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::Path;
use std::process::Command;

use fm_git::{Mark, Rev};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?} failed");
}

fn fixture() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let d = t.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(d.join("src/deep")).unwrap();
    std::fs::write(d.join("a.txt"), "one\n").unwrap();
    std::fs::write(d.join("gone.txt"), "x\n").unwrap();
    std::fs::write(d.join("src/deep/m.rs"), "fn m() {}\n").unwrap();
    std::fs::write(d.join(".gitignore"), "*.log\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "init"]);
    t
}

#[test]
fn reports_marks_and_folder_aggregates() {
    let t = fixture();
    let d = t.path();
    std::fs::write(d.join("src/deep/m.rs"), "fn m() { 1 }\n").unwrap();
    std::fs::remove_file(d.join("gone.txt")).unwrap();
    std::fs::write(d.join("new.txt"), "n\n").unwrap();
    std::fs::write(d.join("x.log"), "l\n").unwrap();
    std::fs::write(d.join("staged.txt"), "s\n").unwrap();
    git(d, &["add", "staged.txt"]);
    std::fs::create_dir(d.join("newdir")).unwrap();
    std::fs::write(d.join("newdir/f"), "f").unwrap();

    let root = fm_git::discover(&d.join("src/deep")).unwrap();
    assert_eq!(root, d);
    let s = fm_git::status(&root).unwrap();
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert_eq!(s.mark("src/deep/m.rs"), Some(Mark::Modified));
    assert_eq!(s.mark("src/deep"), Some(Mark::Modified));
    assert_eq!(s.mark("src"), Some(Mark::Modified));
    assert_eq!(s.mark("gone.txt"), Some(Mark::Removed));
    assert_eq!(s.mark("new.txt"), Some(Mark::Untracked));
    assert_eq!(s.mark("x.log"), Some(Mark::Ignored));
    assert_eq!(s.mark("staged.txt"), Some(Mark::Added));
    assert_eq!(s.mark("newdir/f"), Some(Mark::Untracked));
    assert_eq!(s.mark("a.txt"), None);
    assert_eq!(s.mark_abs(&d.join("src")), Some(Mark::Modified));
    assert!(!s.truncated);
}

#[test]
fn blobs_from_head_and_index() {
    let t = fixture();
    let d = t.path();
    std::fs::write(d.join("a.txt"), "two\n").unwrap();
    git(d, &["add", "a.txt"]);
    std::fs::write(d.join("a.txt"), "three\n").unwrap();
    let head = fm_git::blob(d, &Rev::Commit("HEAD".into()), "a.txt").unwrap();
    assert_eq!(head.as_deref(), Some(&b"one\n"[..]));
    let idx = fm_git::blob(d, &Rev::Index, "a.txt").unwrap();
    assert_eq!(idx.as_deref(), Some(&b"two\n"[..]));
    assert_eq!(
        fm_git::blob(d, &Rev::Commit("HEAD".into()), "nope").unwrap(),
        None
    );
}

#[test]
fn hostile_inputs_are_rejected() {
    let t = fixture();
    let d = t.path();
    let h = || Rev::Commit("HEAD".into());
    assert!(fm_git::blob(d, &h(), "../x").is_err());
    assert!(fm_git::blob(d, &h(), "/etc/passwd").is_err());
    assert!(fm_git::blob(d, &Rev::Commit("--output=x".into()), "a.txt").is_err());
    assert!(fm_git::blob(d, &Rev::Commit(String::new()), "a.txt").is_err());
    assert!(fm_git::status(&d.join("missing")).is_err());
}

#[test]
fn fingerprint_changes_with_index() {
    let t = fixture();
    let d = t.path();
    let a = fm_git::fingerprint(d).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(d.join("b.txt"), "b").unwrap();
    git(d, &["add", "b.txt"]);
    assert_ne!(a, fm_git::fingerprint(d).unwrap());
}

#[test]
fn fingerprint_changes_when_only_the_branch_moves() {
    let t = fixture();
    let d = t.path();
    git(d, &["commit", "-q", "--allow-empty", "-m", "second"]);
    let a = fm_git::fingerprint(d).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    // What a commit that leaves the index alone does: move the branch, nothing else.
    git(d, &["update-ref", "refs/heads/main", "HEAD~1"]);
    let b = fm_git::fingerprint(d).unwrap();
    assert_eq!((a.0, a.1), (b.0, b.1), "index and HEAD untouched");
    assert_ne!(a, b);
    // A packed branch is followed through packed-refs.
    git(d, &["pack-refs", "--all"]);
    let packed = fm_git::fingerprint(d).unwrap();
    assert!(packed.2.is_some());
    std::thread::sleep(std::time::Duration::from_millis(30));
    git(d, &["commit", "-q", "--allow-empty", "-m", "third"]);
    assert_ne!(packed, fm_git::fingerprint(d).unwrap());
}

#[test]
fn fingerprint_ignores_a_head_pointing_outside_refs() {
    let t = fixture();
    let d = t.path();
    for head in [
        "ref: ../../../etc/passwd\n",
        "ref: /etc/passwd\n",
        "ref: refs/../../x\n",
    ] {
        std::fs::write(d.join(".git/HEAD"), head).unwrap();
        let stamp = fm_git::fingerprint(d).unwrap();
        assert_eq!(stamp.2, None, "{head}");
    }
}

#[test]
fn locate_splits_repo_and_relative_path() {
    let t = fixture();
    let d = t.path();
    let (root, rel) = fm_git::locate(&d.join("src/deep/m.rs")).unwrap();
    assert_eq!((root.as_path(), rel.as_str()), (d, "src/deep/m.rs"));
    assert!(fm_git::locate(Path::new("/")).is_none());
}

#[test]
fn hostile_repo_config_is_never_executed() {
    let t = fixture();
    let d = t.path();
    let marker = d.join("pwned");
    let cmd = format!("touch {}", marker.display());
    git(d, &["config", "core.fsmonitor", &cmd]);
    git(d, &["config", "core.hooksPath", "/nonexistent"]);
    git(d, &["config", "filter.x.clean", &cmd]);
    std::fs::write(d.join(".gitattributes"), "*.txt filter=x\n").unwrap();
    std::fs::write(d.join("a.txt"), "changed\n").unwrap();
    let _ = fm_git::status(d);
    assert!(!marker.exists(), "status executed repo config");
    let _ = fm_git::blob(d, &Rev::Index, "a.txt");
    assert!(!marker.exists(), "blob executed repo config");
}

#[cfg(unix)]
#[test]
fn symlink_loops_do_not_hang_or_escape() {
    let t = fixture();
    let d = t.path();
    std::os::unix::fs::symlink(d, d.join("loop")).unwrap();
    std::os::unix::fs::symlink("/etc", d.join("etc-link")).unwrap();
    let st = fm_git::status(d).unwrap();
    assert!(st.len() < 100);
}

#[test]
fn blob_rejects_traversal_and_option_like_revisions() {
    let t = fixture();
    let d = t.path();
    for rel in ["../x", "/etc/passwd", "a\0b"] {
        assert!(fm_git::blob(d, &Rev::Index, rel).is_err(), "{rel:?}");
    }
    for rev in ["--output=/tmp/x", "-1"] {
        assert!(
            fm_git::blob(d, &Rev::Commit(rev.to_string()), "a.txt").is_err(),
            "{rev}"
        );
    }
    let long = "a".repeat(300);
    assert!(fm_git::blob(d, &Rev::Commit(long), "a.txt").is_err());
}

#[test]
fn repo_autocrlf_checkout_is_not_reported_modified() {
    let t = fixture();
    let d = t.path();
    git(d, &["config", "core.autocrlf", "true"]);
    std::fs::write(d.join("a.txt"), "one\r\n").unwrap();
    let st = fm_git::status(d).unwrap();
    assert_eq!(st.mark_abs(&d.join("a.txt")), None);
}

#[test]
fn refs_lists_branches_tags_and_recent_checkouts() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a"), "1\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "i"]);
    git(d, &["tag", "v1"]);
    git(d, &["checkout", "-q", "-b", "feature/x"]);
    git(d, &["checkout", "-q", "main"]);
    let refs = fm_git::refs::list(d).unwrap();
    assert_eq!(refs.current.as_deref(), Some("main"));
    assert_eq!(refs.local, ["feature/x", "main"]);
    assert_eq!(refs.tags, ["v1"]);
    assert_eq!(refs.recent.first().map(String::as_str), Some("main"));
    assert!(refs.recent.iter().any(|r| r == "feature/x"));
}
