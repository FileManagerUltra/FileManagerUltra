//! Attribute carry-over during copy: groups are selected, directory times survive their
//! children, and a refused attribute never costs the content.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use fm_ops::{Backends, Choice, ConflictPolicy, Decision, OpCtx, OpReport, Progress};
use fm_testkit::MemFs;
use fm_vfs::{AttrGroup, AttrSet, Attrs, FmError, Scheme, Vfs, VfsUri};

fn uri(s: &str) -> VfsUri {
    VfsUri::parse(s).expect("uri")
}

struct Two {
    local: Arc<MemFs>,
    remote: Arc<MemFs>,
}

impl Backends for Two {
    fn vfs(&self, u: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        Ok(if u.scheme() == Scheme::Sftp {
            self.remote.clone()
        } else {
            self.local.clone()
        })
    }
}

fn copy(b: &Two, src: &str, dst: &str, attrs: AttrSet) -> OpReport {
    let mut progress = |_: &Progress| {};
    let mut conflict = |_: &fm_ops::Conflict| Decision {
        choice: Choice::Cancel,
        apply_to_all: false,
    };
    let mut ctx = OpCtx::new(ConflictPolicy::Skip, &mut progress, &mut conflict).with_attrs(attrs);
    fm_ops::copy(b, &[uri(src)], &uri(dst), &mut ctx).expect("copy")
}

fn fixture() -> Two {
    let local = MemFs::new();
    local.put_str("/src/d/f.txt", b"data").expect("put");
    let t = |s| Some(UNIX_EPOCH + Duration::from_secs(s));
    let attrs = |m, mode| Attrs {
        mtime: t(m),
        atime: t(m + 1),
        mode: Some(mode),
        ..Attrs::default()
    };
    local
        .set_attrs(&uri("/src/d/f.txt"), &attrs(1_000, 0o640), AttrSet::ALL)
        .expect("attrs");
    local
        .set_attrs(&uri("/src/d"), &attrs(500, 0o750), AttrSet::ALL)
        .expect("attrs");
    let remote = MemFs::new().with_scheme(Scheme::Sftp);
    remote.mkdirs(&uri("sftp://h/dst")).expect("dst");
    Two {
        local: Arc::new(local),
        remote: Arc::new(remote),
    }
}

#[test]
fn directory_mtime_survives_its_children() {
    let b = fixture();
    let r = copy(&b, "/src/d", "sftp://h/dst", AttrSet::ALL);
    assert!(r.is_clean());
    let d = b.remote.stat_full(&uri("sftp://h/dst/d")).expect("dir");
    assert_eq!(d.mtime, Some(UNIX_EPOCH + Duration::from_secs(500)));
    assert_eq!(d.mode, Some(0o750));
    let f = b
        .remote
        .stat_full(&uri("sftp://h/dst/d/f.txt"))
        .expect("file");
    assert_eq!(f.mtime, Some(UNIX_EPOCH + Duration::from_secs(1_000)));
    assert_eq!(f.mode, Some(0o640));
}

#[test]
fn only_the_selected_groups_are_carried() {
    let b = fixture();
    let r = copy(&b, "/src/d", "sftp://h/dst", AttrSet::TIMES);
    assert!(r.is_clean() && r.attr_incomplete.is_empty());
    let f = b
        .remote
        .stat_full(&uri("sftp://h/dst/d/f.txt"))
        .expect("file");
    assert_eq!(f.mtime, Some(UNIX_EPOCH + Duration::from_secs(1_000)));
    assert_eq!(f.mode, None);
}

#[test]
fn plain_copy_touches_no_attributes() {
    let b = fixture();
    copy(&b, "/src/d", "sftp://h/dst", AttrSet::NONE);
    let f = b
        .remote
        .stat_full(&uri("sftp://h/dst/d/f.txt"))
        .expect("file");
    assert_ne!(f.mtime, Some(UNIX_EPOCH + Duration::from_secs(1_000)));
}

#[test]
fn a_refused_group_is_reported_and_the_content_stays() {
    let mut b = fixture();
    b.remote = Arc::new(
        MemFs::new()
            .with_scheme(Scheme::Sftp)
            .with_attr_caps(AttrSet::TIMES),
    );
    b.remote.mkdirs(&uri("sftp://h/dst")).expect("dst");
    let r = copy(&b, "/src/d", "sftp://h/dst", AttrSet::ALL);
    assert!(r.is_clean(), "attribute refusal is not an item failure");
    assert_eq!(
        b.remote
            .get_file(&uri("sftp://h/dst/d/f.txt"))
            .expect("content"),
        b"data"
    );
    let (at, report) = r
        .attr_incomplete
        .iter()
        .find(|(u, _)| u.path_str().ends_with("f.txt"))
        .expect("file reported");
    assert!(at.path_str().ends_with("f.txt"));
    assert!(report.failed_group(AttrGroup::Mode).is_some());
    assert!(report.failed_group(AttrGroup::Mtime).is_none());
}
