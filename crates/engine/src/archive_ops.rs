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

use fm_archive::write::{CompressOptions, Hooks, VfsFor, left_out_links, walk};
use fm_ops::{OpCtx, OpReport, PlanLimits, Progress, TransferProgress};
use fm_vfs::{EntryKind, FmError, Vfs, VfsUri, WriteOpts};

use crate::backends::VfsRegistry;

/// Smallest part size for split output.
pub const MIN_PART: u64 = 64 * 1024;

/// Most numbered names tried for a free name.
const MAX_TRIES: u32 = 1000;

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
    for n in 1..MAX_TRIES {
        let name = if n == 1 {
            format!("{stem}{suffix}")
        } else {
            format!("{stem} ({n}){suffix}")
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

fn human(bytes: u64) -> String {
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

fn group(n: u64) -> String {
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

fn copy_cancellable(
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
    let (stem, _) = fm_archive::format::split_suffix(archive_name);
    if stem.is_empty() {
        "extracted".to_string()
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
