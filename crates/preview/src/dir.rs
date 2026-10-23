use fm_vfs::{Entry, FmError, Vfs};

use crate::{DirDoc, Limits};

/// Counts direct children only; never recurses.
pub fn summarize(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<DirDoc, FmError> {
    let children = vfs.read_dir(&entry.uri)?;
    let mut doc = DirDoc {
        files: 0,
        dirs: 0,
        other: 0,
        bytes: 0,
        newest: None,
        truncated: children.len() > limits.dir_entries,
    };
    for c in children.iter().take(limits.dir_entries) {
        if c.is_dir_like() {
            doc.dirs += 1;
        } else if c.is_file() || c.is_symlink() {
            doc.files += 1;
            doc.bytes = doc.bytes.saturating_add(c.size.unwrap_or(0));
        } else {
            doc.other += 1;
        }
        if let Some(m) = c.modified
            && doc.newest.as_ref().is_none_or(|(_, t)| m > *t)
        {
            doc.newest = Some((c.name.clone(), m));
        }
    }
    Ok(doc)
}
