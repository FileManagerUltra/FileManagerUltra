//! Extract and compress as file jobs (`archive-extract-compress`).
//!
//! Extraction is the normal copy pipeline reading from an archive: the archive's top
//! items are copied into a new folder named after it (`site (2)` when `site` exists), so
//! conflicts, progress, cancel and the link rules are the ones every copy has.
//! Compression walks the selection once, writes the archive to a temporary file and then
//! puts it in place (renamed next to the target, uploaded, or cut into numbered parts).

use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use fm_archive::split::PartSet;
use fm_archive::write::{CompressOptions, Hooks, VfsFor, left_out_links, walk};
use fm_ops::{OpCtx, OpReport, PlanLimits, Progress, TransferProgress};
use fm_vfs::{EntryKind, FmError, Vfs, VfsUri, WriteOpts};

use crate::archive_parts::{self, BackupAction};
use crate::backends::VfsRegistry;

/// Smallest part size for split output.
pub const MIN_PART: u64 = 64 * 1024;

/// Most numbered names tried for a free name.
const MAX_TRIES: u32 = 1000;

/// `a (2)` → `a`; names without a trailing ` (N)` stay as they are.
fn without_number(stem: &str) -> &str {
    stem.strip_suffix(')')
        .and_then(|s| s.rsplit_once(" ("))
        .filter(|(base, n)| {
            !base.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
        })
        .map_or(stem, |(base, _)| base)
}

/// `dir/stem+suffix`, or `dir/stem (N)+suffix` from N = 2 when taken. With `parts`, the
/// `.001` part must be free too.
fn free(
    vfs: &dyn Vfs,
    dir: &VfsUri,
    stem: &str,
    suffix: &str,
    parts: bool,
) -> Result<VfsUri, FmError> {
    let free_at = |uri: &VfsUri| match vfs.stat(uri) {
        Err(FmError::NotFound(_)) => Ok(true),
        Err(e) => Err(e),
        Ok(_) => Ok(false),
    };
    // `a (2)` taken: the next is `a (3)`, not `a (2) (2)`.
    let base = without_number(stem);
    for n in 1..MAX_TRIES {
        let name = if n == 1 {
            format!("{stem}{suffix}")
        } else {
            format!("{base} ({n}){suffix}")
        };
        let candidate = dir.join(&name)?;
        let part = dir.join(&format!("{name}.001"))?;
        if free_at(&candidate)? && (!parts || free_at(&part)?) {
            return Ok(candidate);
        }
    }
    Err(FmError::AlreadyExists(format!("{stem}{suffix}")))
}

/// Extraction of more than this many bytes...
pub const BOMB_BYTES: u64 = 1 << 30;
/// ...at more than this ratio to the archive's own size asks first (decompression bomb).
pub const BOMB_RATIO: u64 = 1000;

/// Why a device, FIFO or socket in an archive is not extracted.
const SPECIAL: &str = "special file (device, FIFO, socket or unresolved hard link), not extracted";

/// The question to ask before extracting `total` declared bytes from an archive of
/// `packed` bytes, or `None` when it is unremarkable (more than [`BOMB_BYTES`] at a ratio
/// above [`BOMB_RATIO`]: a decompression bomb, or a header that lies).
pub fn bomb_question(name: &str, total: u64, packed: u64) -> Option<String> {
    if total <= BOMB_BYTES || total / packed.max(1) <= BOMB_RATIO {
        return None;
    }
    Some(format!(
        "\"{name}\" says it unpacks to {} from only {} ({}:1). \
         This can be a decompression bomb meant to fill the disk. Extract anyway?",
        human(total),
        human(packed),
        group(total / packed.max(1)),
    ))
}

pub(crate) fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for u in UNITS {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = u;
    }
    format!("{value:.1} {unit}")
}

pub(crate) fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Extract the archive mounted at `root` (the file `container`) into a new folder `name`
/// (or `name (N)`) in `dest_dir`.
///
/// Containment (security): names were normalised when the archive was indexed, so no
/// entry can name a path outside the archive root; the copy pipeline never writes
/// through a link and skips everything below a folder it could not create; devices,
/// FIFOs, sockets and hard links that leave the archive are left out and listed; no
/// modes are carried, so setuid/setgid bits never survive. A large extraction at a
/// bomb-like ratio calls `ask` first; `false` removes the new folder and cancels.
pub(crate) fn extract(
    backends: &VfsRegistry,
    root: &VfsUri,
    container: &VfsUri,
    dest_dir: &VfsUri,
    name: &str,
    octx: &mut OpCtx<'_>,
    ask: &mut dyn FnMut(&str) -> bool,
) -> Result<OpReport, FmError> {
    let archive = backends.require(root)?;
    let entries = archive.read_dir(root)?;
    probe_password(archive.as_ref(), root)?;
    let dest_vfs = backends.require(dest_dir)?;
    let target = free(dest_vfs.as_ref(), dest_dir, name, "", false)?;
    dest_vfs.create_dir(&target)?;
    let (specials, items): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|e| matches!(e.kind, EntryKind::Other));
    let sources: Vec<VfsUri> = items.into_iter().map(|e| e.uri).collect();
    let plan = match fm_ops::plan_transfer(backends, &sources, &target, PlanLimits::default()) {
        Ok(plan) => plan,
        Err(e) => {
            let _ = dest_vfs.remove(&target);
            return Err(e);
        }
    };
    let packed = backends
        .require(container)
        .and_then(|v| v.stat(container))
        .ok()
        .and_then(|e| e.size)
        .unwrap_or(0);
    if let Some(question) = bomb_question(name, plan.total_bytes, packed)
        && !ask(&question)
    {
        let _ = dest_vfs.remove(&target);
        return Ok(OpReport {
            cancelled: true,
            ..OpReport::default()
        });
    }
    let mut report = if plan.is_empty() {
        OpReport::default()
    } else {
        fm_ops::transfer::run_copy(backends, &plan, octx)
    };
    report.skipped += specials.len() as u64;
    let room = fm_ops::plan::MAX_LEFT_OUT.saturating_sub(report.left_out.len());
    report.left_out.extend(
        specials
            .into_iter()
            .take(room)
            .map(|e| (e.uri, SPECIAL.to_string())),
    );
    // Undo removes the whole new folder.
    report.created = vec![target];
    Ok(report)
}

/// Open the first file of the archive before anything is written, so an encrypted
/// archive without its password stops here (and asks) instead of failing every file.
fn probe_password(archive: &dyn Vfs, root: &VfsUri) -> Result<(), FmError> {
    let mut dirs = vec![root.clone()];
    let mut seen = 0usize;
    while let Some(dir) = dirs.pop() {
        for e in archive.read_dir(&dir)? {
            seen += 1;
            if seen > PROBE_ENTRIES {
                return Ok(());
            }
            match e.kind {
                EntryKind::Dir => dirs.push(e.uri),
                EntryKind::File => {
                    return match archive.open_read(&e.uri) {
                        Err(err)
                            if err.wants_password() || fm_archive::format::is_gave_up(&err) =>
                        {
                            Err(err)
                        }
                        // Other errors belong to that one file and show up in the report.
                        _ => Ok(()),
                    };
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Entries looked at to find a first file for [`probe_password`].
const PROBE_ENTRIES: usize = 10_000;

/// Where the archive is written before it is put in place.
struct Temp {
    path: PathBuf,
    /// A private folder to remove afterwards (remote or split targets).
    dir: Option<PathBuf>,
}

impl Temp {
    fn cleanup(&self) {
        match &self.dir {
            Some(dir) => {
                let _ = std::fs::remove_dir_all(dir);
            }
            None => {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

/// Create a new archive from `sources` at `target` (numbered when taken), optionally cut
/// into parts of `split` bytes (`name.ext.001`, …).
pub(crate) fn compress(
    backends: &VfsRegistry,
    sources: &[VfsUri],
    target: &VfsUri,
    options: &CompressOptions,
    split: Option<u64>,
    cancelled: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(&Progress),
) -> Result<OpReport, FmError> {
    options.validate()?;
    if split.is_some_and(|n| n < MIN_PART) {
        return Err(FmError::InvalidParams(
            "parts must be at least 64 KiB".into(),
        ));
    }
    let registry = backends.clone();
    let vfs_for: VfsFor = Arc::new(move |u: &VfsUri| registry.require(u));
    let w = walk(sources, &vfs_for, cancelled)?;
    let dir = target
        .parent()
        .ok_or_else(|| FmError::InvalidParams("the target needs a folder".into()))?;
    let dest_vfs = backends.require(&dir)?;
    let name = target
        .file_name()
        .ok_or_else(|| FmError::InvalidParams("the target needs a name".into()))?;
    let (stem, suffix) = fm_archive::format::split_suffix(name);
    let final_uri = free(dest_vfs.as_ref(), &dir, &stem, &suffix, split.is_some())?;
    let final_name = final_uri.file_name().unwrap_or("archive").to_string();

    let local_dir = dir.local_path().filter(|_| dest_vfs.is_native_local());
    let temp = match (&local_dir, split) {
        (Some(d), None) => Temp {
            path: d.join(format!(".{final_name}.fmtmp-{}", std::process::id())),
            dir: None,
        },
        _ => {
            let d = crate::open_cmds::private_temp_dir()?;
            Temp {
                path: d.join(&final_name),
                dir: Some(d),
            }
        }
    };

    let total = w.bytes;
    let items = (w.files + w.dirs) as u64;
    let mut report_progress = |done: u64, current: &str| {
        progress(&Progress {
            bytes: TransferProgress { done, total },
            items: TransferProgress {
                done: 0,
                total: items,
            },
            current: current.to_string(),
        });
    };
    let written = (|| -> Result<(), FmError> {
        let file = std::fs::File::create(&temp.path)?;
        let mut hooks = Hooks {
            progress: &mut report_progress,
            cancelled,
        };
        let out =
            fm_archive::write::compress(BufWriter::new(file), &w, &vfs_for, options, &mut hooks)?;
        let file = out.into_inner().map_err(|e| FmError::Io(e.into_error()))?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        temp.cleanup();
        return Err(e);
    }

    let placed = match (split, &local_dir) {
        (Some(size), _) => write_parts(
            dest_vfs.as_ref(),
            &dir,
            &final_name,
            &temp.path,
            size,
            cancelled,
        ),
        (None, Some(d)) => std::fs::rename(&temp.path, d.join(&final_name))
            .map(|()| vec![final_uri.clone()])
            .map_err(FmError::Io),
        (None, None) => upload(dest_vfs.as_ref(), &temp.path, &final_uri, cancelled)
            .map(|()| vec![final_uri.clone()]),
    };
    temp.cleanup();
    let created = placed?;
    let left_out = w.skipped.len() + left_out_links(&w, options.format).len();
    Ok(OpReport {
        done: items,
        skipped: left_out as u64,
        created,
        ..OpReport::default()
    })
}

fn upload(
    vfs: &dyn Vfs,
    from: &std::path::Path,
    to: &VfsUri,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), FmError> {
    let mut src = std::fs::File::open(from)?;
    let mut dst = vfs.open_write(to, WriteOpts::default())?;
    let copied = copy_cancellable(&mut src, &mut dst, u64::MAX, cancelled).and_then(|_| {
        dst.flush()?;
        Ok(())
    });
    if copied.is_err() {
        drop(dst);
        let _ = vfs.remove(to);
    }
    copied
}

/// Counts what passes through to report byte progress while a part is copied.
struct Counting<'a> {
    inner: &'a mut dyn Write,
    done: &'a mut u64,
    on: &'a mut dyn FnMut(u64),
}

impl Write for Counting<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        *self.done += n as u64;
        (self.on)(*self.done);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Concatenate the numbered parts `first` belongs to into one file (`name.7z` for
/// `name.7z.001`…), next to the parts unless `target` says otherwise. A missing part is
/// named and nothing is written. The bytes go to a temporary file that replaces an
/// existing target only after `ask` agreed and the sizes add up; the parts are kept.
pub(crate) fn join(
    backends: &VfsRegistry,
    first: &VfsUri,
    target: Option<&VfsUri>,
    ask: &mut dyn FnMut(&str) -> bool,
    cancelled: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(&Progress),
) -> Result<OpReport, FmError> {
    if first.archive_handle().is_some() || target.is_some_and(|t| t.archive_handle().is_some()) {
        return Err(FmError::Unsupported(
            "join parts that are in a folder, not inside an archive",
        ));
    }
    let dir = first
        .parent()
        .ok_or_else(|| FmError::InvalidParams("the part needs a folder".into()))?;
    let name = first
        .file_name()
        .ok_or_else(|| FmError::InvalidParams("the part needs a name".into()))?;
    let vfs = backends.require(&dir)?;
    let entries = vfs.read_dir(&dir)?;
    let set = PartSet::read(name, &entries)?;
    if !set.backups.is_empty() {
        return Err(FmError::backend(
            "archive",
            format!(
                "An earlier save of {} was interrupted; restore or delete its backup parts first",
                set.base
            ),
        ));
    }
    let target = match target {
        Some(t) => t.clone(),
        None => dir.join(&set.base)?,
    };
    let target_dir = target
        .parent()
        .ok_or_else(|| FmError::InvalidParams("the target needs a folder".into()))?;
    let target_name = target
        .file_name()
        .ok_or_else(|| FmError::InvalidParams("the target needs a name".into()))?
        .to_string();
    if target_dir == dir && set.names.contains(&target_name) {
        return Err(FmError::InvalidParams(
            "the target is one of the parts".into(),
        ));
    }
    let dest_vfs = backends.require(&target_dir)?;
    match dest_vfs.stat(&target) {
        Ok(_) => {
            if !ask(&format!("{target_name} already exists. Replace it?")) {
                return Ok(OpReport {
                    cancelled: true,
                    ..OpReport::default()
                });
            }
        }
        Err(FmError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }

    let sizes: Vec<u64> = set
        .names
        .iter()
        .map(|n| {
            entries
                .iter()
                .find(|e| &e.name == n)
                .and_then(|e| e.size)
                .unwrap_or(0)
        })
        .collect();
    let total: u64 = sizes.iter().sum();
    let tmp = target_dir.join(&format!(".{target_name}.fmtmp-{}", std::process::id()))?;
    let written = (|| -> Result<(), FmError> {
        let mut dst = dest_vfs.open_write(&tmp, WriteOpts::default())?;
        let mut done = 0u64;
        for (i, part) in set.names.iter().enumerate() {
            let mut src = vfs.open_read(&dir.join(part)?)?;
            let before = done;
            let mut on = |now: u64| {
                progress(&Progress {
                    bytes: TransferProgress {
                        done: now.min(total),
                        total,
                    },
                    items: TransferProgress {
                        done: i as u64,
                        total: set.names.len() as u64,
                    },
                    current: part.clone(),
                });
            };
            let mut counted = Counting {
                inner: &mut dst,
                done: &mut done,
                on: &mut on,
            };
            copy_cancellable(&mut src, &mut counted, u64::MAX, cancelled)?;
            if done < before.saturating_add(sizes.get(i).copied().unwrap_or(0)) {
                return Err(FmError::backend(
                    "archive",
                    format!("{part} is shorter than it was listed; try again"),
                ));
            }
        }
        dst.flush()?;
        Ok(())
    })();
    let placed = written.and_then(|()| swap_in(dest_vfs.as_ref(), &tmp, &target, &target_dir));
    if let Err(e) = placed {
        let _ = dest_vfs.remove(&tmp);
        return Err(e);
    }
    Ok(OpReport {
        done: set.names.len() as u64,
        created: vec![target],
        ..OpReport::default()
    })
}

fn write_parts(
    vfs: &dyn Vfs,
    dir: &VfsUri,
    name: &str,
    from: &std::path::Path,
    size: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<VfsUri>, FmError> {
    let mut src = std::fs::File::open(from)?;
    let total = src.metadata()?.len();
    let count = total.div_ceil(size).max(1);
    if count > 9999 {
        return Err(FmError::InvalidParams(
            "that part size would make more than 9 999 parts".into(),
        ));
    }
    let width = if count > 999 { 4 } else { 3 };
    let mut parts = Vec::new();
    for i in 1..=count {
        let part = dir.join(&format!("{name}.{i:0width$}"))?;
        let mut dst = vfs.open_write(&part, WriteOpts::default())?;
        parts.push(part.clone());
        let result = copy_cancellable(&mut src, &mut dst, size, cancelled).and_then(|_| {
            dst.flush()?;
            Ok(())
        });
        if let Err(e) = result {
            drop(dst);
            for p in &parts {
                let _ = vfs.remove(p);
            }
            return Err(e);
        }
    }
    Ok(parts)
}

pub(crate) fn copy_cancellable(
    src: &mut dyn Read,
    dst: &mut dyn Write,
    limit: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<u64, FmError> {
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    while done < limit {
        if cancelled() {
            return Err(FmError::Cancelled);
        }
        let want = usize::try_from(limit - done)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let Some(chunk) = buf.get_mut(..want) else {
            break;
        };
        let n = src.read(chunk)?;
        if n == 0 {
            break;
        }
        dst.write_all(chunk.get(..n).unwrap_or(&[]))?;
        done += n as u64;
    }
    Ok(done)
}

/// The folder name Extract here uses for an archive file (`site.tar.gz` → `site`).
pub fn extract_name(archive_name: &str) -> String {
    // A part of a split archive extracts the whole set, named after the archive.
    let base = fm_archive::split::parse(archive_name).map(|p| p.base);
    let (stem, _) = fm_archive::format::split_suffix(base.as_deref().unwrap_or(archive_name));
    if stem.is_empty() {
        "extracted".to_string()
    } else {
        stem
    }
}

/// What [`save`] writes: the archive file itself, or a copy at a new place.
#[derive(Clone)]
pub struct SaveTarget {
    pub mount: Arc<fm_archive::mount::LazyArchive>,
    /// `None`: replace the archive file; `Some(dir/name)`: Save as copy (numbered when
    /// taken).
    pub copy_to: Option<VfsUri>,
    /// Not a save: restore or delete the backup parts an interrupted save left.
    pub backup: Option<BackupAction>,
}

impl SaveTarget {
    pub fn root(&self) -> VfsUri {
        self.mount.root().clone()
    }

    /// The folder the written file lands in.
    pub fn dest_dir(&self) -> Option<VfsUri> {
        match &self.copy_to {
            Some(to) => to.parent(),
            None => self.mount.container().parent(),
        }
    }
}

impl std::fmt::Debug for SaveTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveTarget")
            .field("archive", self.mount.container())
            .field("copy_to", &self.copy_to)
            .field("backup", &self.backup)
            .finish()
    }
}

impl PartialEq for SaveTarget {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mount, &other.mount)
            && self.copy_to == other.copy_to
            && self.backup == other.backup
    }
}

impl Eq for SaveTarget {}

/// Replace `dst` with `tmp` (both in one folder on `vfs`), keeping the old file as a
/// backup until the new one is in place; on failure the old file is restored.
fn swap_in(vfs: &dyn Vfs, tmp: &VfsUri, dst: &VfsUri, dir: &VfsUri) -> Result<(), FmError> {
    let exists = match vfs.stat(dst) {
        Ok(_) => true,
        Err(FmError::NotFound(_)) => false,
        Err(e) => return Err(e),
    };
    if !exists {
        return vfs.rename(tmp, dst);
    }
    let name = dst.file_name().unwrap_or("archive");
    let backup = dir.join(&format!(".{name}.fmbak-{}", std::process::id()))?;
    vfs.rename(dst, &backup)?;
    if let Err(e) = vfs.rename(tmp, dst) {
        let _ = vfs.rename(&backup, dst);
        return Err(e);
    }
    let _ = vfs.remove(&backup);
    Ok(())
}

/// Cut the saved archive at `from` into parts of the original size next to the old ones
/// and swap them in (add-archive-vfs D16).
fn replace_parts(
    vfs: &dyn Vfs,
    dir: &VfsUri,
    old: &PartSet,
    from: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), FmError> {
    let total = std::fs::metadata(from)?.len();
    let names = old.part_names(archive_parts::part_count(total, old.part_size))?;
    let temps = archive_parts::write_temp_parts(vfs, dir, &names, from, old.part_size, cancelled)?;
    // Past this point nothing is cancelled: a half-done swap is the one bad state.
    if cancelled() {
        for t in &temps {
            let _ = vfs.remove(t);
        }
        return Err(FmError::Cancelled);
    }
    archive_parts::swap_parts(vfs, dir, &old.names, &names, &temps).inspect_err(|_| {
        for t in &temps {
            let _ = vfs.remove(t);
        }
    })
}

/// Restore or delete the backup parts of an interrupted save (no archive is written).
fn recover_backups(
    mount: &fm_archive::mount::LazyArchive,
    action: BackupAction,
) -> Result<OpReport, FmError> {
    if mount.is_dirty() {
        return Err(FmError::InvalidParams(
            "discard the staged changes first".into(),
        ));
    }
    let container = mount.container();
    let dir = container
        .parent()
        .ok_or_else(|| FmError::InvalidParams("the archive has no folder".into()))?;
    let first = container.file_name().unwrap_or_default();
    let done = archive_parts::recover(mount.container_vfs().as_ref(), &dir, first, action)?;
    mount.reopen();
    Ok(OpReport {
        done,
        ..OpReport::default()
    })
}

/// Write an archive with its staged changes (add-archive-vfs D7): check the file did not
/// change since it was opened, write the new archive to a temporary file, read it back,
/// then put it in place. The original stays untouched, and the staged changes stay, until
/// the new file is complete; afterwards the archive opens again from the saved file.
pub(crate) fn save(
    backends: &VfsRegistry,
    target: &SaveTarget,
    cancelled: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(&Progress),
) -> Result<OpReport, FmError> {
    let mount = &target.mount;
    if let Some(action) = target.backup {
        return recover_backups(mount, action);
    }
    let fs = mount
        .opened()
        .ok_or_else(|| FmError::InvalidParams("the archive is not open".into()))?;
    fs.writable()?;
    if target.copy_to.is_none() && !fs.is_dirty() {
        return Err(FmError::InvalidParams(
            "the archive has no unsaved changes".into(),
        ));
    }
    let container = mount.container().clone();
    let cvfs = mount.container_vfs().clone();
    // A numbered set is written back as numbered parts, replacing the originals.
    let parts = fs.parts().filter(|_| target.copy_to.is_none()).cloned();
    // A copy of a numbered set is cut into parts of the same size (`copy.7z.001`, …).
    let copy_parts = target
        .copy_to
        .as_ref()
        .and(fs.parts())
        .and_then(|set| set.part_size);
    let changed = || {
        FmError::backend(
            "archive",
            "The archive changed on disk since it was opened: save it as a copy, or \
             discard the changes",
        )
    };
    if let Some(set) = &parts {
        let dir = container
            .parent()
            .ok_or_else(|| FmError::InvalidParams("the archive has no folder".into()))?;
        let first = container.file_name().unwrap_or_default();
        let now = PartSet::read(first, &cvfs.read_dir(&dir)?)?;
        if !now.backups.is_empty() {
            return Err(FmError::backend(
                "archive",
                "Backup parts of an earlier save are in the folder: restore or delete them first",
            ));
        }
        if now.names != set.names || now.stamp != set.stamp {
            return Err(changed());
        }
    } else if target.copy_to.is_none() {
        let now = cvfs.stat(&container)?;
        if fs.stamp() != Some((now.size.unwrap_or(0), now.modified)) {
            return Err(changed());
        }
    }
    let (dir, dvfs, final_uri) = match &target.copy_to {
        None => {
            let dir = container
                .parent()
                .ok_or_else(|| FmError::InvalidParams("the archive has no folder".into()))?;
            (dir, cvfs.clone(), container)
        }
        Some(to) => {
            let dir = to
                .parent()
                .ok_or_else(|| FmError::InvalidParams("the copy needs a folder".into()))?;
            let dvfs = backends.require(&dir)?;
            let name = to
                .file_name()
                .ok_or_else(|| FmError::InvalidParams("the copy needs a name".into()))?;
            let (stem, suffix) = fm_archive::format::split_suffix(name);
            let free_uri = free(dvfs.as_ref(), &dir, &stem, &suffix, copy_parts.is_some())?;
            (dir, dvfs, free_uri)
        }
    };
    let final_name = final_uri.file_name().unwrap_or("archive").to_string();
    if !fs.set_saving(true) {
        return Err(FmError::InvalidParams(
            "this archive is already being saved".into(),
        ));
    }
    let result = (|| -> Result<OpReport, FmError> {
        let local_dir = dir
            .local_path()
            .filter(|_| dvfs.is_native_local() && parts.is_none() && copy_parts.is_none());
        let temp = match &local_dir {
            Some(d) => Temp {
                path: d.join(format!(".{final_name}.fmtmp-{}", std::process::id())),
                dir: None,
            },
            None => {
                let d = crate::open_cmds::private_temp_dir()?;
                Temp {
                    path: d.join(&final_name),
                    dir: Some(d),
                }
            }
        };
        let written = (|| -> Result<(), FmError> {
            let total = fm_archive::save::plan(&fs, cancelled)?.bytes;
            let mut report_progress = |done: u64, current: &str| {
                progress(&Progress {
                    bytes: TransferProgress { done, total },
                    items: TransferProgress::default(),
                    current: current.to_string(),
                });
            };
            let file = std::fs::File::create(&temp.path)?;
            let mut hooks = Hooks {
                progress: &mut report_progress,
                cancelled,
            };
            let (out, expect) = fm_archive::save::save(&fs, BufWriter::new(file), &mut hooks)?;
            let file = out.into_inner().map_err(|e| FmError::Io(e.into_error()))?;
            file.sync_all()?;
            drop(file);
            let tmp_uri = VfsUri::parse(&temp.path.to_string_lossy())?;
            // The temporary file is always on this machine's disk.
            let local: Arc<dyn Vfs> = Arc::new(fm_fs_local::LocalFs::new());
            let src = Arc::new(fm_archive::source::VfsSource::open(local, tmp_uri)?);
            let check = fm_archive::save::saved_passwords(&fs);
            fm_archive::save::verify(src, fs.format(), &check, &expect)
        })();
        if let Err(e) = written {
            temp.cleanup();
            return Err(e);
        }
        if cancelled() {
            temp.cleanup();
            return Err(FmError::Cancelled);
        }
        let placed = match (&local_dir, &parts, copy_parts) {
            (_, Some(set), _) => replace_parts(dvfs.as_ref(), &dir, set, &temp.path, cancelled)
                .map(|()| final_uri.clone()),
            (_, None, Some(size)) => write_parts(
                dvfs.as_ref(),
                &dir,
                &final_name,
                &temp.path,
                size,
                cancelled,
            )
            .and_then(|written| {
                written
                    .into_iter()
                    .next()
                    .ok_or_else(|| FmError::backend("archive", "no part was written"))
            }),
            (Some(d), None, None) => std::fs::rename(&temp.path, d.join(&final_name))
                .map_err(FmError::Io)
                .map(|()| final_uri.clone()),
            (None, None, None) => {
                let tmp_uri = dir.join(&format!(".{final_name}.fmtmp-{}", std::process::id()))?;
                upload(dvfs.as_ref(), &temp.path, &tmp_uri, cancelled)
                    .and_then(|()| {
                        swap_in(dvfs.as_ref(), &tmp_uri, &final_uri, &dir).inspect_err(|_| {
                            let _ = dvfs.remove(&tmp_uri);
                        })
                    })
                    .map(|()| final_uri.clone())
            }
        };
        temp.cleanup();
        let created = placed?;
        Ok(OpReport {
            done: 1,
            created: vec![created],
            ..OpReport::default()
        })
    })();
    fs.set_saving(false);
    let report = result?;
    // Saved over the archive, or saved as a copy: either way the staged set is done
    // with. A replaced file opens again from disk; after a copy the original's index
    // still fits it.
    if target.copy_to.is_none() {
        fm_archive::save::adopt_password(&fs);
    }
    fs.discard();
    if target.copy_to.is_none() {
        mount.reopen();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_names_count_on_instead_of_nesting() {
        assert_eq!(without_number("a (2)"), "a");
        assert_eq!(without_number("a b (12)"), "a b");
        for same in ["a", "a (x)", "a (2) b", "(2)", " (2)", "a ()"] {
            assert_eq!(without_number(same), same);
        }
        let tmp = tempfile::tempdir().expect("tmp");
        for name in ["a.zip", "a (2).zip"] {
            std::fs::write(tmp.path().join(name), b"x").expect("w");
        }
        let vfs = fm_fs_local::LocalFs::new();
        let dir = VfsUri::parse(&tmp.path().to_string_lossy()).expect("dir");
        let next = |stem: &str| {
            free(&vfs, &dir, stem, ".zip", false)
                .expect("free")
                .file_name()
                .map(str::to_string)
        };
        assert_eq!(next("a").as_deref(), Some("a (3).zip"));
        assert_eq!(next("a (2)").as_deref(), Some("a (3).zip"));
        assert_eq!(next("b (2)").as_deref(), Some("b (2).zip"));
    }

    #[test]
    fn only_large_extractions_at_a_bomb_ratio_ask() {
        const MB: u64 = 1_000_000;
        assert_eq!(bomb_question("a.zip", BOMB_BYTES, 1), None, "1 GiB is fine");
        assert_eq!(
            bomb_question("a.zip", 2 * BOMB_BYTES, 4 * MB),
            None,
            "537:1"
        );
        let q = bomb_question("site.zip", 2 * BOMB_BYTES, MB).expect("2147:1 asks");
        assert!(
            q.contains("site.zip") && q.contains("2.1 GB") && q.contains("1.0 MB"),
            "{q}"
        );
        assert!(q.contains("2,147:1"), "{q}");
        assert!(
            bomb_question("empty.zip", 2 * BOMB_BYTES, 0).is_some(),
            "no size: ask"
        );
        let peta = bomb_question("42.zip", 4_500_000_000_000_000, 42_000).expect("asks");
        assert!(
            peta.contains("4.5 PB") && peta.contains("42.0 KB"),
            "{peta}"
        );
    }

    fn zip_with_readme() -> Vec<u8> {
        use std::io::Write as _;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.start_file("readme", zip::write::SimpleFileOptions::default())
            .expect("f");
        w.write_all(b"hi").expect("w");
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn a_cancelled_save_keeps_everything_and_a_remote_save_swaps_whole() {
        let mem = Arc::new(fm_testkit::MemFs::new());
        mem.mkdirs_str("/srv").expect("dir");
        let original = zip_with_readme();
        mem.put_str("/srv/site.zip", &original).expect("zip");
        let vfs: Arc<dyn Vfs> = mem.clone();
        let mut backends = VfsRegistry::default();
        backends.register(vfs.clone());
        let container = VfsUri::parse("/srv/site.zip").expect("uri");
        let root =
            VfsUri::archive_root(&fm_archive::handle_for(&container, 0, None)).expect("root");
        let mount = Arc::new(fm_archive::mount::LazyArchive::new(
            root.clone(),
            container,
            vfs,
            None,
            0,
        ));
        mount
            .create_dir(&root.join("new").expect("n"))
            .expect("staged");
        let target = SaveTarget {
            mount: mount.clone(),
            copy_to: None,
            backup: None,
        };
        let listing = |mem: &fm_testkit::MemFs| {
            let dir = VfsUri::parse("/srv").expect("dir");
            let mut v: Vec<String> = mem
                .read_dir(&dir)
                .expect("ls")
                .into_iter()
                .map(|e| e.name)
                .collect();
            v.sort();
            v
        };
        let r = save(&backends, &target, &|| true, &mut |_| {});
        assert!(matches!(r, Err(FmError::Cancelled)), "{r:?}");
        assert_eq!(mem.get_str("/srv/site.zip").expect("read"), original);
        assert_eq!(listing(&mem), vec!["site.zip"], "no temporary file left");
        assert!(mount.is_dirty(), "the staged change is kept");

        save(&backends, &target, &|| false, &mut |_| {}).expect("save");
        assert_eq!(
            listing(&mem),
            vec!["site.zip"],
            "no backup or temporary file left"
        );
        let saved = mem.get_str("/srv/site.zip").expect("read");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved)).expect("zip");
        assert!(z.by_name("new/").is_ok() && z.by_name("readme").is_ok());
        assert!(!mount.is_dirty());
        assert_eq!(
            mount.read_dir(&root).expect("reopened").len(),
            2,
            "opens from the saved file"
        );
    }
}
