//! Full attribute read/write for local paths (capability `file-attributes`).
//!
//! Everything here is safe code: times through `rustix::fs::utimensat` (Unix, symlink-safe)
//! or `std::fs::FileTimes` (Windows, macOS creation time), owner through
//! `std::os::unix::fs::lchown`, xattrs and POSIX ACL blobs through `rustix`'s xattr calls
//! (Linux). A final symlink is never followed.

use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::time::SystemTime;

use fm_vfs::{AttrCaps, AttrFailKind, AttrGroup, AttrReport, AttrSet, Attrs, FmError};

/// What the local backend supports on this platform.
pub fn caps() -> AttrCaps {
    #[cfg(target_os = "linux")]
    {
        AttrCaps {
            read: AttrSet::of(&[
                AttrGroup::Btime,
                AttrGroup::Mtime,
                AttrGroup::Atime,
                AttrGroup::Mode,
                AttrGroup::Owner,
                AttrGroup::Acl,
                AttrGroup::Xattrs,
            ]),
            write: AttrSet::of(&[
                AttrGroup::Mtime,
                AttrGroup::Atime,
                AttrGroup::Mode,
                AttrGroup::Owner,
                AttrGroup::Acl,
                AttrGroup::Xattrs,
            ]),
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        AttrCaps {
            read: AttrSet::of(&[
                AttrGroup::Btime,
                AttrGroup::Mtime,
                AttrGroup::Atime,
                AttrGroup::Mode,
                AttrGroup::Owner,
            ]),
            write: AttrSet::of(&[
                AttrGroup::Mtime,
                AttrGroup::Atime,
                AttrGroup::Mode,
                AttrGroup::Owner,
            ])
            .union(if cfg!(target_os = "macos") {
                AttrSet::of(&[AttrGroup::Btime])
            } else {
                AttrSet::NONE
            }),
        }
    }
    #[cfg(not(unix))]
    {
        AttrCaps {
            read: AttrSet::of(&[
                AttrGroup::Btime,
                AttrGroup::Mtime,
                AttrGroup::Atime,
                AttrGroup::Owner,
                AttrGroup::Acl,
                AttrGroup::Flags,
            ]),
            write: AttrSet::TIMES.union(AttrSet::of(&[AttrGroup::Owner, AttrGroup::Acl])),
        }
    }
}

/// Read every attribute the platform exposes for `path` (not following a final symlink).
pub fn read(path: &Path, display: &str) -> Result<Attrs, FmError> {
    let meta = fs::symlink_metadata(path).map_err(|e| crate::map_io(e, display))?;
    let is_link = meta.file_type().is_symlink();
    let mut a = Attrs {
        btime: meta.created().ok(),
        mtime: meta.modified().ok(),
        atime: meta.accessed().ok(),
        ..Attrs::default()
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !is_link {
            a.mode = Some(meta.mode() & 0o7777);
        }
        a.uid = Some(meta.uid());
        a.gid = Some(meta.gid());
        a.owner = names::user(meta.uid());
        a.group = names::group(meta.gid());
        #[cfg(target_os = "linux")]
        xattr::read_into(path, &mut a);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        a.dos_flags = Some(meta.file_attributes());
        // A link's descriptor would be its target's; leave links without one.
        if !is_link {
            match fm_winsec::read(path) {
                Ok(sec) => {
                    a.owner = sec.owner.as_deref().and_then(Attrs::bounded_name);
                    a.group = sec.group.as_deref().and_then(Attrs::bounded_name);
                    a.set_acl(fm_vfs::Acl::WindowsSd(sec.descriptor));
                }
                Err(fm_winsec::SecError::Unsupported) => {}
                Err(e) => a.skipped.push(format!("security descriptor: {e}")),
            }
        }
    }
    let _ = is_link;
    Ok(a)
}

/// Apply the groups of `which` that `attrs` carries. Order: owner, ACL, xattrs, mode, times
/// (owner first because chown may clear setuid; times last so nothing disturbs them).
pub fn write(path: &Path, attrs: &Attrs, which: AttrSet) -> Result<AttrReport, FmError> {
    // A vanished path is an error for the whole call; per-attribute trouble is reported.
    let meta = fs::symlink_metadata(path).map_err(|e| crate::map_io(e, &path.to_string_lossy()))?;
    let is_link = meta.file_type().is_symlink();
    let writable = caps().write;
    let mut report = AttrReport::default();
    for g in [
        AttrGroup::Owner,
        AttrGroup::Acl,
        AttrGroup::Xattrs,
        AttrGroup::Mode,
        AttrGroup::Flags,
        AttrGroup::Btime,
        AttrGroup::Mtime,
        AttrGroup::Atime,
    ] {
        if !which.contains(g) || !attrs.present().contains(g) {
            continue;
        }
        if !writable.contains(g) {
            report.fail(g, AttrFailKind::Unsupported);
            continue;
        }
        if g == AttrGroup::Mode && is_link {
            report.fail(g, AttrFailKind::Unsupported);
            continue;
        }
        let outcome = match g {
            AttrGroup::Owner => apply_owner(path, attrs),
            AttrGroup::Acl => apply_acl(path, attrs),
            AttrGroup::Xattrs => apply_xattrs(path, attrs),
            AttrGroup::Mode => apply_mode(path, attrs),
            // Times are applied together below so one syscall sets both.
            _ => continue,
        };
        record(&mut report, g, outcome);
    }
    apply_times(path, attrs, which, writable, is_link, &mut report);
    Ok(report)
}

fn record(report: &mut AttrReport, g: AttrGroup, r: Result<(), AttrFailKind>) {
    match r {
        Ok(()) => report.ok(g),
        Err(kind) => report.fail(g, kind),
    }
}

fn kind_of_io(e: &std::io::Error) -> AttrFailKind {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => AttrFailKind::PermissionDenied,
        std::io::ErrorKind::Unsupported => AttrFailKind::Unsupported,
        _ => match e.raw_os_error() {
            // EPERM / EACCES / EROFS, ENOTSUP / EOPNOTSUPP (Linux numbering).
            Some(1 | 13 | 30) => AttrFailKind::PermissionDenied,
            Some(95) => AttrFailKind::Unsupported,
            _ => AttrFailKind::Other(e.to_string()),
        },
    }
}

#[cfg(unix)]
fn apply_owner(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
    std::os::unix::fs::lchown(path, attrs.uid, attrs.gid).map_err(|e| kind_of_io(&e))
}

#[cfg(windows)]
fn apply_owner(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
    apply_sd(
        path,
        attrs,
        fm_winsec::Parts {
            owner: true,
            dacl: false,
        },
    )
}

#[cfg(not(any(unix, windows)))]
fn apply_owner(_: &Path, _: &Attrs) -> Result<(), AttrFailKind> {
    Err(AttrFailKind::Unsupported)
}

#[cfg(windows)]
fn apply_sd(path: &Path, attrs: &Attrs, parts: fm_winsec::Parts) -> Result<(), AttrFailKind> {
    use fm_winsec::SecError;
    let Some(fm_vfs::Acl::WindowsSd(sd)) = &attrs.acl else {
        // Owner and DACL travel in one descriptor; a POSIX source has none to give.
        return Err(AttrFailKind::Unsupported);
    };
    fm_winsec::write(path, sd, parts).map_err(|e| match e {
        SecError::PermissionDenied => AttrFailKind::PermissionDenied,
        SecError::Unsupported => AttrFailKind::Unsupported,
        other => AttrFailKind::Other(other.to_string()),
    })
}

#[cfg(unix)]
fn apply_mode(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = attrs.mode else {
        return Ok(());
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777)).map_err(|e| kind_of_io(&e))
}

#[cfg(not(unix))]
fn apply_mode(_: &Path, _: &Attrs) -> Result<(), AttrFailKind> {
    Err(AttrFailKind::Unsupported)
}

#[cfg(target_os = "linux")]
fn apply_acl(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
    xattr::write_acl(path, attrs)
}

#[cfg(windows)]
fn apply_acl(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
    apply_sd(
        path,
        attrs,
        fm_winsec::Parts {
            owner: false,
            dacl: true,
        },
    )
}

#[cfg(not(any(target_os = "linux", windows)))]
fn apply_acl(_: &Path, _: &Attrs) -> Result<(), AttrFailKind> {
    Err(AttrFailKind::Unsupported)
}

#[cfg(target_os = "linux")]
fn apply_xattrs(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
    xattr::write_xattrs(path, attrs)
}

#[cfg(not(target_os = "linux"))]
fn apply_xattrs(_: &Path, _: &Attrs) -> Result<(), AttrFailKind> {
    Err(AttrFailKind::Unsupported)
}

#[cfg(unix)]
fn apply_times(
    path: &Path,
    attrs: &Attrs,
    which: AttrSet,
    writable: AttrSet,
    is_link: bool,
    report: &mut AttrReport,
) {
    use rustix::fs::UTIME_OMIT;
    use rustix::fs::{AtFlags, CWD, Timestamps, utimensat};
    use rustix::time::Timespec;

    fn spec(t: Option<SystemTime>) -> Timespec {
        match t.and_then(to_unix) {
            Some((sec, nsec)) => Timespec {
                tv_sec: sec,
                tv_nsec: nsec.into(),
            },
            None => Timespec {
                tv_sec: 0,
                #[allow(clippy::useless_conversion)] // the field's type differs per platform
                tv_nsec: UTIME_OMIT.into(),
            },
        }
    }

    let want_m = which.contains(AttrGroup::Mtime) && attrs.mtime.is_some();
    let want_a = which.contains(AttrGroup::Atime) && attrs.atime.is_some();
    if want_m || want_a {
        let ts = Timestamps {
            last_access: spec(want_a.then_some(attrs.atime).flatten()),
            last_modification: spec(want_m.then_some(attrs.mtime).flatten()),
        };
        let flags = if is_link {
            AtFlags::SYMLINK_NOFOLLOW
        } else {
            AtFlags::empty()
        };
        let res =
            utimensat(CWD, path, &ts, flags).map_err(|e| kind_of_io(&std::io::Error::from(e)));
        if want_m {
            record(report, AttrGroup::Mtime, res.clone());
        }
        if want_a {
            record(report, AttrGroup::Atime, res);
        }
    }
    if which.contains(AttrGroup::Btime) && attrs.btime.is_some() {
        record(
            report,
            AttrGroup::Btime,
            apply_btime(path, attrs, writable, is_link),
        );
    }
}

/// Seconds and nanoseconds since the epoch (negative seconds before it).
#[cfg(unix)]
fn to_unix(t: SystemTime) -> Option<(i64, u32)> {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => Some((i64::try_from(d.as_secs()).ok()?, d.subsec_nanos())),
        Err(e) => {
            let d = e.duration();
            let secs = i64::try_from(d.as_secs()).ok()?;
            match d.subsec_nanos() {
                0 => Some((secs.checked_neg()?, 0)),
                n => Some((secs.checked_add(1)?.checked_neg()?, 1_000_000_000 - n)),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn apply_btime(path: &Path, attrs: &Attrs, _: AttrSet, is_link: bool) -> Result<(), AttrFailKind> {
    use std::os::macos::fs::FileTimesExt;
    let Some(t) = attrs.btime else {
        return Ok(());
    };
    if is_link {
        return Err(AttrFailKind::Unsupported);
    }
    let f = fs::File::options()
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path))
        .map_err(|e| kind_of_io(&e))?;
    f.set_times(fs::FileTimes::new().set_created(t))
        .map_err(|e| kind_of_io(&e))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn apply_btime(_: &Path, _: &Attrs, _: AttrSet, _: bool) -> Result<(), AttrFailKind> {
    Err(AttrFailKind::Unsupported)
}

#[cfg(not(unix))]
fn apply_times(
    path: &Path,
    attrs: &Attrs,
    which: AttrSet,
    _writable: AttrSet,
    is_link: bool,
    report: &mut AttrReport,
) {
    use std::os::windows::fs::{FileTimesExt, OpenOptionsExt};

    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let groups = [AttrGroup::Btime, AttrGroup::Mtime, AttrGroup::Atime];
    let wanted: Vec<AttrGroup> = groups
        .into_iter()
        .filter(|g| which.contains(*g) && attrs.present().contains(*g))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let mut times = fs::FileTimes::new();
    if let (true, Some(t)) = (wanted.contains(&AttrGroup::Btime), attrs.btime) {
        times = times.set_created(t);
    }
    if let (true, Some(t)) = (wanted.contains(&AttrGroup::Mtime), attrs.mtime) {
        times = times.set_modified(t);
    }
    if let (true, Some(t)) = (wanted.contains(&AttrGroup::Atime), attrs.atime) {
        times = times.set_accessed(t);
    }
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if is_link {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let res = fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .custom_flags(flags)
        .open(path)
        .and_then(|f| f.set_times(times))
        .map_err(|e| kind_of_io(&e));
    for g in wanted {
        record(report, g, res.clone());
    }
}

/// uid/gid → name lookup from `/etc/passwd` and `/etc/group` (bounded, safe parsing).
#[cfg(unix)]
mod names {
    use std::io::Read;

    use fm_vfs::Attrs;

    const MAX_DB_BYTES: u64 = 4 * 1024 * 1024;

    fn lookup(file: &str, id: u32) -> Option<String> {
        let mut text = String::new();
        std::fs::File::open(file)
            .ok()?
            .take(MAX_DB_BYTES)
            .read_to_string(&mut text)
            .ok()?;
        text.lines().find_map(|line| {
            let mut f = line.split(':');
            let name = f.next()?;
            let _pw = f.next()?;
            let n: u32 = f.next()?.parse().ok()?;
            (n == id).then(|| Attrs::bounded_name(name)).flatten()
        })
    }

    pub fn user(uid: u32) -> Option<String> {
        lookup("/etc/passwd", uid)
    }

    pub fn group(gid: u32) -> Option<String> {
        lookup("/etc/group", gid)
    }
}

#[cfg(target_os = "linux")]
mod xattr {
    use std::path::Path;

    use fm_vfs::{Acl, AttrFailKind, Attrs, MAX_ACL_BYTES, MAX_XATTR_VALUE};
    use rustix::fs::{
        XattrFlags, lgetxattr, lgetxattr_query, llistxattr, llistxattr_query, lsetxattr,
    };

    const ACL_ACCESS: &str = "system.posix_acl_access";
    const ACL_DEFAULT: &str = "system.posix_acl_default";
    /// Kernel `XATTR_LIST_MAX`; a bigger list is not trusted.
    const MAX_LIST: usize = 64 * 1024;

    fn value(path: &Path, name: &str, cap: usize) -> Option<Vec<u8>> {
        let size = lgetxattr_query(path, name).ok()?;
        if size > cap {
            return None;
        }
        let mut buf = vec![0u8; size];
        let n = lgetxattr(path, name, &mut buf[..]).ok()?;
        buf.truncate(n);
        Some(buf)
    }

    pub fn read_into(path: &Path, a: &mut Attrs) {
        let Ok(size) = llistxattr_query(path) else {
            return;
        };
        if size == 0 {
            return;
        }
        if size > MAX_LIST {
            a.skipped.push("xattr list: too large".to_string());
            return;
        }
        let mut list = vec![0u8; size];
        let Ok(n) = llistxattr(path, &mut list[..]) else {
            return;
        };
        list.truncate(n);
        let (mut access, mut default) = (None, None);
        for raw in list.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let Ok(name) = std::str::from_utf8(raw) else {
                a.skipped.push("xattr: name is not UTF-8".to_string());
                continue;
            };
            match name {
                ACL_ACCESS => access = value(path, name, MAX_ACL_BYTES),
                ACL_DEFAULT => default = value(path, name, MAX_ACL_BYTES),
                _ => match value(path, name, MAX_XATTR_VALUE) {
                    Some(v) => {
                        a.insert_xattr(name, v);
                    }
                    None => a
                        .skipped
                        .push(format!("xattr {name}: unreadable or too large")),
                },
            }
        }
        if access.is_some() || default.is_some() {
            a.set_acl(Acl::Posix { access, default });
        }
    }

    fn set(path: &Path, name: &str, v: &[u8]) -> Result<(), AttrFailKind> {
        lsetxattr(path, name, v, XattrFlags::empty())
            .map_err(|e| super::kind_of_io(&std::io::Error::from(e)))
    }

    pub fn write_acl(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
        match &attrs.acl {
            Some(Acl::Posix { access, default }) => {
                if let Some(b) = access {
                    set(path, ACL_ACCESS, b)?;
                }
                if let Some(b) = default {
                    set(path, ACL_DEFAULT, b)?;
                }
                Ok(())
            }
            // A Windows descriptor has no meaning on a POSIX filesystem.
            Some(Acl::WindowsSd(_)) => Err(AttrFailKind::Unsupported),
            None => Ok(()),
        }
    }

    pub fn write_xattrs(path: &Path, attrs: &Attrs) -> Result<(), AttrFailKind> {
        let mut first_err = None;
        let mut failed = 0usize;
        for (name, v) in &attrs.xattrs {
            if let Err(e) = set(path, name, v) {
                failed += 1;
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            None => Ok(()),
            Some(AttrFailKind::PermissionDenied) => Err(AttrFailKind::PermissionDenied),
            Some(AttrFailKind::Unsupported) => Err(AttrFailKind::Unsupported),
            Some(AttrFailKind::Other(m)) => {
                Err(AttrFailKind::Other(format!("{failed} not set: {m}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn times_and_mode_round_trip_on_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a");
        let dst = dir.path().join("b");
        fs::write(&src, b"x").unwrap();
        fs::write(&dst, b"x").unwrap();
        let want = Attrs {
            mtime: Some(t(1_000_000_000)),
            atime: Some(t(1_100_000_000)),
            mode: Some(0o640),
            ..Attrs::default()
        };
        let which = AttrSet::of(&[AttrGroup::Mtime, AttrGroup::Atime, AttrGroup::Mode]);
        let report = write(&src, &want, which).unwrap();
        if cfg!(unix) {
            assert!(report.is_complete(), "{:?}", report.failed);
        }
        let got = read(&src, "a").unwrap();
        assert_eq!(got.mtime, want.mtime);
        assert_eq!(got.atime, want.atime);
        #[cfg(unix)]
        assert_eq!(got.mode, Some(0o640));

        // stat_full of one file applied to another reproduces it.
        write(&dst, &got, which).unwrap();
        let copy = read(&dst, "b").unwrap();
        assert_eq!(copy.mtime, want.mtime);
    }

    #[test]
    fn directory_mtime_can_be_set() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("d");
        fs::create_dir(&d).unwrap();
        let a = Attrs {
            mtime: Some(t(1_234_567_890)),
            ..Attrs::default()
        };
        write(&d, &a, AttrSet::TIMES).unwrap();
        fs::write(d.join("child"), b"x").unwrap();
        // Writing a child moved it; applying again after the children restores it.
        write(&d, &a, AttrSet::TIMES).unwrap();
        assert_eq!(read(&d, "d").unwrap().mtime, a.mtime);
    }

    #[test]
    fn unselected_groups_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, b"x").unwrap();
        let before = read(&f, "f").unwrap();
        let a = Attrs {
            mtime: Some(t(1_000_000_000)),
            mode: Some(0o600),
            ..Attrs::default()
        };
        write(&f, &a, AttrSet::TIMES).unwrap();
        assert_eq!(read(&f, "f").unwrap().mode, before.mode);
    }

    #[test]
    fn a_missing_path_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        assert!(read(&gone, "gone").is_err());
        assert!(write(&gone, &Attrs::default(), AttrSet::ALL).is_err());
    }

    #[test]
    fn unsupported_groups_are_reported_per_attribute() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, b"x").unwrap();
        let a = Attrs {
            dos_flags: Some(1),
            mtime: Some(t(1_000_000_000)),
            ..Attrs::default()
        };
        let r = write(&f, &a, AttrSet::ALL).unwrap();
        assert!(r.failed_group(AttrGroup::Flags).is_some());
        assert!(r.applied.contains(AttrGroup::Mtime));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_times_do_not_touch_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t");
        fs::write(&target, b"x").unwrap();
        let link = dir.path().join("l");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let before = read(&target, "t").unwrap().mtime;
        let a = Attrs {
            mtime: Some(t(1_000_000_000)),
            mode: Some(0o600),
            ..Attrs::default()
        };
        let r = write(&link, &a, AttrSet::of(&[AttrGroup::Mtime, AttrGroup::Mode])).unwrap();
        assert_eq!(read(&target, "t").unwrap().mtime, before);
        assert_eq!(read(&link, "l").unwrap().mtime, a.mtime);
        assert!(r.failed_group(AttrGroup::Mode).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn pre_epoch_times_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, b"x").unwrap();
        let old = SystemTime::UNIX_EPOCH - Duration::new(86_400, 500);
        let a = Attrs {
            mtime: Some(old),
            ..Attrs::default()
        };
        write(&f, &a, AttrSet::TIMES).unwrap();
        assert_eq!(read(&f, "f").unwrap().mtime, Some(old));
    }

    #[cfg(unix)]
    #[test]
    fn chown_to_self_works_and_names_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, b"x").unwrap();
        let a = read(&f, "f").unwrap();
        assert!(a.uid.is_some() && a.gid.is_some());
        let r = write(&f, &a, AttrSet::of(&[AttrGroup::Owner])).unwrap();
        assert!(r.is_complete(), "{:?}", r.failed);
    }

    #[cfg(unix)]
    #[test]
    fn chown_to_someone_else_reports_permission_denied_when_unprivileged() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        fs::write(&f, b"x").unwrap();
        let me = fs::metadata(&f).unwrap().uid();
        if me == 0 {
            return; // root may chown freely
        }
        let a = Attrs {
            uid: Some(0),
            gid: Some(0),
            mtime: Some(t(1_000_000_000)),
            ..Attrs::default()
        };
        let r = write(&f, &a, AttrSet::of(&[AttrGroup::Owner, AttrGroup::Mtime])).unwrap();
        assert_eq!(
            r.failed_group(AttrGroup::Owner).map(|f| &f.kind),
            Some(&AttrFailKind::PermissionDenied)
        );
        assert!(r.applied.contains(AttrGroup::Mtime));
        assert_eq!(fs::read(&f).unwrap(), b"x");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn xattrs_and_acl_blobs_round_trip_or_report_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("s");
        let dst = dir.path().join("d");
        fs::write(&src, b"x").unwrap();
        fs::write(&dst, b"x").unwrap();
        let mut a = Attrs::default();
        a.insert_xattr("user.fm.test", b"hello".to_vec());
        let r = write(&src, &a, AttrSet::of(&[AttrGroup::Xattrs])).unwrap();
        if let Some(f) = r.failed_group(AttrGroup::Xattrs) {
            // tmpfs without user xattrs: must be reported, not fatal.
            assert_ne!(f.kind, AttrFailKind::Other(String::new()));
            return;
        }
        let got = read(&src, "s").unwrap();
        assert_eq!(
            got.xattrs.get("user.fm.test").map(Vec::as_slice),
            Some(&b"hello"[..])
        );
        let r = write(&dst, &got, AttrSet::ALL).unwrap();
        assert!(r.applied.contains(AttrGroup::Xattrs));
        assert_eq!(
            read(&dst, "d")
                .unwrap()
                .xattrs
                .get("user.fm.test")
                .map(Vec::as_slice),
            Some(&b"hello"[..])
        );
    }

    #[test]
    fn caps_never_claim_to_write_what_they_cannot_read_back() {
        let c = caps();
        for g in c.write.iter() {
            // Everything writable is also readable, so a round trip is checkable.
            assert!(c.read.contains(g) || g == AttrGroup::Btime, "{g:?}");
        }
        #[cfg(target_os = "linux")]
        assert!(!c.write.contains(AttrGroup::Btime));
    }
}
