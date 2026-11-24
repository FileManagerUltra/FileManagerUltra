//! The Preview pane's card for an archive file (`archive-browsing` "Archive preview"):
//! format, methods, entry counts, packed and unpacked size, encryption, shadowed
//! duplicates and the top-level entries. The archive is opened on its own, outside the
//! session's mounts, so previewing never leaves anything mounted or asks for a password.

use std::sync::Arc;

use fm_archive::NodeKind;
use fm_archive::index::ROOT;
use fm_archive::{FormatId, LazyArchive};
use fm_preview::{MetaDoc, Preview};
use fm_vfs::{Entry, Vfs, VfsUri};

use crate::archive_ops::{group, human};

/// Formats whose entry list comes from an index at the end of the file; the others (tar
/// family, single compressed files) are read through, so they are only opened when small.
fn random_access(format: FormatId) -> bool {
    matches!(format, FormatId::Zip | FormatId::SevenZ | FormatId::Iso)
}

/// Largest compressed stream read through just to preview it.
const SCAN_MAX: u64 = 64 << 20;

/// Top-level entries named on the card.
const TOP_ENTRIES: usize = 12;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", group(n as u64), if n == 1 { one } else { many })
}

/// The card for `entry` when it is an archive file, `None` for anything else. Blocks while
/// the archive is indexed: call it from the preview worker.
pub(crate) fn card(vfs: &Arc<dyn Vfs>, entry: &Entry) -> Option<Preview> {
    if !entry.is_file() || !crate::archive_cmds::is_archive_name(&entry.name) {
        return None;
    }
    let format = fm_archive::by_name(&entry.name)
        .or_else(|| fm_archive::split::numbered_format(&entry.name));
    let mut rows = vec![("Name".to_string(), entry.name.clone())];
    let mut hint = Some("Press Enter to browse inside.".to_string());
    if let Some(f) = format {
        rows.push(("Format".into(), f.label().to_string()));
    }
    let size = entry.size.unwrap_or(0);
    let scan = format.is_none_or(|f| random_access(f) || size <= SCAN_MAX);
    if !scan {
        rows.push(("Size".into(), human(size)));
        hint = Some("Too large to scan for a preview. Press Enter to browse inside.".into());
        return Some(meta(rows, hint));
    }
    match open(vfs, entry) {
        Ok(fs) => describe(&fs, size, &mut rows),
        Err(e) => {
            if size > 0 {
                rows.push(("Size".into(), human(size)));
            }
            hint = Some(if e.wants_password() {
                "Its file names are encrypted. Press Enter and give the password to browse inside."
                    .to_string()
            } else {
                e.to_string()
            });
        }
    }
    Some(meta(rows, hint))
}

fn meta(rows: Vec<(String, String)>, hint: Option<String>) -> Preview {
    Preview::Meta(MetaDoc {
        rows,
        hint,
        poster: None,
        video: None,
        audio: None,
    })
}

fn open(vfs: &Arc<dyn Vfs>, entry: &Entry) -> Result<Arc<fm_archive::ArchiveFs>, fm_vfs::FmError> {
    let handle = fm_archive::handle_for(&entry.uri, 0, None);
    let root = VfsUri::archive_root(&handle)?;
    LazyArchive::new(root, entry.uri.clone(), vfs.clone(), None, 0).mount()
}

fn describe(fs: &fm_archive::ArchiveFs, file_size: u64, rows: &mut Vec<(String, String)>) {
    let index = fs.index();
    let info = fs.info();
    let (mut files, mut dirs, mut links, mut other) = (0usize, 0usize, 0usize, 0usize);
    for node in (1..=index.len()).filter_map(|id| index.node(id)) {
        match node.kind {
            NodeKind::File => files += 1,
            NodeKind::Dir => dirs += 1,
            NodeKind::Symlink(_) => links += 1,
            NodeKind::Other => other += 1,
        }
    }
    if let Some(slot) = rows.iter_mut().find(|(k, _)| k == "Format") {
        slot.1 = fs.format().label().to_string();
    } else {
        rows.push(("Format".into(), fs.format().label().to_string()));
    }
    if !info.methods.is_empty() {
        rows.push(("Methods".into(), info.methods.join(", ")));
    }
    let mut counts = vec![plural(files, "file", "files")];
    if dirs > 0 {
        counts.push(plural(dirs, "folder", "folders"));
    }
    if links > 0 {
        counts.push(plural(links, "link", "links"));
    }
    if other > 0 {
        counts.push(plural(other, "special entry", "special entries"));
    }
    rows.push(("Entries".into(), counts.join(", ")));
    let (unpacked, _) = index.totals();
    let packed = fs.parts().map_or(file_size, |p| p.stamp.0);
    rows.push((
        "Size".into(),
        format!("{} packed, {} unpacked", human(packed), human(unpacked)),
    ));
    if let Some(parts) = fs.parts() {
        rows.push(("Parts".into(), group(parts.names.len() as u64)));
    }
    if info.encrypted {
        let mut text = String::from("Yes");
        let named: Vec<&String> = info
            .methods
            .iter()
            .filter(|m| m.to_ascii_uppercase().contains("AES") || m.contains("ZipCrypto"))
            .collect();
        if !named.is_empty() {
            text.push_str(&format!(
                " ({})",
                named
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if info.names_encrypted {
            text.push_str(", file names too");
        }
        rows.push(("Encrypted".into(), text));
    }
    if index.shadowed > 0 {
        rows.push((
            "Shadowed duplicates".into(),
            format!(
                "{} (hidden by later entries of the same name)",
                index.shadowed
            ),
        ));
    }
    let mut top: Vec<String> = index
        .children(ROOT)
        .iter()
        .filter_map(|id| index.node(*id))
        .map(|n| {
            if n.kind == NodeKind::Dir {
                format!("{}/", n.name)
            } else {
                n.name.clone()
            }
        })
        .collect();
    top.sort();
    if !top.is_empty() {
        let more = top.len().saturating_sub(TOP_ENTRIES);
        let mut text = top
            .iter()
            .take(TOP_ENTRIES)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if more > 0 {
            text.push_str(&format!(" and {} more", group(more as u64)));
        }
        rows.push(("Top level".into(), text));
    }
}
