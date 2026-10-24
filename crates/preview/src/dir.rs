use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often a running recursive scan reports its totals so far.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

use fm_vfs::{Entry, FmError, Vfs};

use crate::{DirDoc, Limits};

/// Counts the direct children only; `recursive` also walks every subfolder, bounded by the
/// scan caps in [`Limits`] and never following symlinks.
pub fn summarize(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    recursive: bool,
) -> Result<DirDoc, FmError> {
    summarize_cancellable(vfs, entry, limits, recursive, None, None)
}

/// [`summarize`] that stops early (returning the partial document) once `cancel` is set;
/// the caller discards that result.
pub fn summarize_cancellable(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    recursive: bool,
    cancel: Option<&AtomicBool>,
    progress: Option<&dyn Fn(DirDoc)>,
) -> Result<DirDoc, FmError> {
    let children = vfs.read_dir(&entry.uri)?;
    let mut doc = DirDoc {
        files: 0,
        dirs: 0,
        other: 0,
        bytes: 0,
        newest: None,
        truncated: children.len() > limits.dir_entries,
        items: Vec::new(),
        items_truncated: false,
        recursive,
        unreadable: 0,
        oldest: None,
        largest: None,
        hidden: 0,
        subdirs: Default::default(),
        scanning: false,
    };
    if !recursive {
        for c in children.iter().take(limits.dir_entries) {
            count(&mut doc, c, &c.name);
        }
        let mut shown: Vec<&Entry> = children.iter().filter(|c| !c.hidden).collect();
        sort_entries(&mut shown);
        doc.items_truncated = shown.len() > limits.dir_items;
        doc.items = shown.into_iter().take(limits.dir_items).cloned().collect();
        return Ok(doc);
    }
    Ok(walk(vfs, children, limits, doc, cancel, progress))
}

/// One pass over a recursive tree, depth-first so a folder's own files come before its
/// subfolders. Iterative (no recursion: hostile trees must not overflow the stack).
fn walk(
    vfs: &dyn Vfs,
    root: Vec<Entry>,
    limits: &Limits,
    mut doc: DirDoc,
    cancel: Option<&AtomicBool>,
    progress: Option<&dyn Fn(DirDoc)>,
) -> DirDoc {
    let deadline = Instant::now() + Duration::from_millis(limits.dir_scan_millis);
    let mut visited = 0usize;
    let mut last_report = Instant::now();
    let mut opened = 1usize;
    let mut stack: Vec<(Vec<Entry>, String, usize)> = vec![(root, String::new(), 1)];
    'walk: while let Some((list, prefix, depth)) = stack.pop() {
        let mut list = list;
        list.sort_by(entry_order);
        let mut below: Vec<(Vec<Entry>, String, usize)> = Vec::new();
        for child in list {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                break 'walk;
            }
            if visited >= limits.dir_scan_entries || Instant::now() >= deadline {
                doc.truncated = true;
                doc.items_truncated |= child_is_shown(&child);
                break 'walk;
            }
            visited += 1;
            let shown = format!("{prefix}{}", child.name);
            count(&mut doc, &child, &shown);
            tally_subdir(&mut doc, &prefix, &child);
            if let Some(report) = progress
                && last_report.elapsed() >= PROGRESS_EVERY
            {
                last_report = Instant::now();
                report(DirDoc {
                    scanning: true,
                    ..doc.clone()
                });
            }
            if !child.hidden {
                if doc.items.len() < limits.dir_items {
                    let mut item = child.clone();
                    item.name = shown.clone();
                    doc.items.push(item);
                } else {
                    doc.items_truncated = true;
                }
            }
            if child.is_dir() {
                if depth >= limits.dir_scan_depth || opened >= limits.dir_scan_dirs {
                    doc.truncated = true;
                    continue;
                }
                match vfs.read_dir(&child.uri) {
                    Ok(children) => {
                        opened += 1;
                        below.push((children, format!("{shown}/"), depth + 1));
                    }
                    Err(_) => doc.unreadable += 1,
                }
            }
        }
        // Pop order reverses again: push the last folder first so the first one is walked next.
        for item in below.into_iter().rev() {
            stack.push(item);
        }
    }
    doc
}

fn child_is_shown(child: &Entry) -> bool {
    !child.hidden
}

/// Attribute `child` to the direct subfolder of the scanned folder that contains it.
fn tally_subdir(doc: &mut DirDoc, prefix: &str, child: &Entry) {
    let top = match prefix.split('/').next() {
        Some("") | None if child.is_dir() => child.name.as_str(),
        Some(t) if !t.is_empty() => t,
        _ => return,
    };
    if !doc.subdirs.contains_key(top) && doc.subdirs.len() >= crate::SUBDIRS_MAX {
        return;
    }
    let slot = doc.subdirs.entry(top.to_string()).or_default();
    if !prefix.is_empty() && (child.is_file() || child.is_symlink()) {
        slot.0 += 1;
        slot.1 = slot.1.saturating_add(child.size.unwrap_or(0));
    }
}

fn count(doc: &mut DirDoc, entry: &Entry, shown: &str) {
    if entry.is_dir_like() {
        doc.dirs += 1;
    } else if entry.is_file() || entry.is_symlink() {
        doc.files += 1;
        let size = entry.size.unwrap_or(0);
        doc.bytes = doc.bytes.saturating_add(size);
        if doc.largest.as_ref().is_none_or(|(_, s)| size > *s) {
            doc.largest = Some((shown.to_string(), size));
        }
    } else {
        doc.other += 1;
    }
    if entry.hidden {
        doc.hidden += 1;
    }
    if let Some(m) = entry.modified
        && doc.oldest.as_ref().is_none_or(|(_, t)| m < *t)
    {
        doc.oldest = Some((shown.to_string(), m));
    }
    if let Some(m) = entry.modified
        && doc.newest.as_ref().is_none_or(|(_, t)| m > *t)
    {
        doc.newest = Some((shown.to_string(), m));
    }
}

fn entry_order(a: &Entry, b: &Entry) -> std::cmp::Ordering {
    b.is_dir_like()
        .cmp(&a.is_dir_like())
        .then_with(|| natural_cmp(&a.name, &b.name))
}

fn sort_entries(list: &mut [&Entry]) {
    list.sort_by(|a, b| entry_order(a, b));
}

/// Case-insensitive name order that sorts digit runs by value (`img2` before `img10`).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits.trim_start_matches('0').to_string()
                };
                let (da, db) = (run(&mut a), run(&mut b));
                let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(&db));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{natural_cmp, summarize};
    use crate::Limits;
    use fm_testkit::{FaultFs, MemFs};
    use fm_vfs::{Entry, Vfs, VfsUri};
    use std::cmp::Ordering;

    fn uri(path: &str) -> VfsUri {
        VfsUri::parse(path).expect("uri")
    }

    fn entry(fs: &dyn Vfs, path: &str) -> Entry {
        fs.stat(&uri(path)).expect("stat")
    }

    fn tree() -> MemFs {
        let fs = MemFs::new();
        fs.mkdirs_str("/r/docs/deep").expect("dirs");
        fs.mkdirs_str("/r/.hidden").expect("dirs");
        fs.put_str("/r/a.txt", b"abc").expect("a");
        fs.put_str("/r/z.bin", b"12345").expect("z");
        fs.put_str("/r/docs/b.txt", b"1234567").expect("b");
        fs.put_str("/r/docs/deep/c.txt", b"123456789").expect("c");
        fs.put_str("/r/.hidden/h.txt", b"hid").expect("h");
        fs
    }

    #[test]
    fn shallow_summary_counts_direct_children_only() {
        let fs = tree();
        let doc = summarize(&fs, &entry(&fs, "/r"), &Limits::default(), false).expect("summary");
        assert!(!doc.recursive && doc.unreadable == 0);
        assert_eq!((doc.files, doc.dirs), (2, 2));
        assert_eq!(doc.bytes, 8);
        let names: Vec<&str> = doc.items.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".hidden", "docs", "a.txt", "z.bin"]);
    }

    #[test]
    fn recursive_summary_reports_subfolder_sizes_and_the_largest_file() {
        let fs = tree();
        let doc = summarize(&fs, &entry(&fs, "/r"), &Limits::default(), true).expect("summary");
        assert_eq!(doc.largest.as_ref().map(|l| l.1), Some(9));
        assert_eq!(doc.subdirs.get("docs"), Some(&(2, 16)));
        assert_eq!(doc.subdirs.get(".hidden"), Some(&(1, 3)));
        assert_eq!(doc.subdirs.len(), 2);
        assert!(doc.oldest.is_some() || doc.newest.is_none());
    }

    #[test]
    fn recursive_summary_counts_the_whole_tree() {
        let fs = tree();
        let doc = summarize(&fs, &entry(&fs, "/r"), &Limits::default(), true).expect("summary");
        assert!(doc.recursive);
        assert_eq!((doc.files, doc.dirs), (5, 3));
        assert_eq!(doc.bytes, 27);
        assert!(!doc.truncated && !doc.items_truncated);
        let names: Vec<&str> = doc.items.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                ".hidden",
                "docs",
                "a.txt",
                "z.bin",
                ".hidden/h.txt",
                "docs/deep",
                "docs/b.txt",
                "docs/deep/c.txt",
            ]
        );
        // Names in the grid are relative paths, so every tile has a unique name.
        assert_eq!(doc.items[7].uri, uri("/r/docs/deep/c.txt"));
        assert!(doc.newest.is_some());
    }

    #[test]
    fn recursive_scan_respects_the_entry_and_depth_caps() {
        let fs = tree();
        let caps = Limits {
            dir_scan_entries: 4,
            ..Limits::default()
        };
        let doc = summarize(&fs, &entry(&fs, "/r"), &caps, true).expect("summary");
        assert!(doc.truncated && doc.items_truncated);
        assert_eq!(doc.files + doc.dirs, 4);

        let shallow_depth = Limits {
            dir_scan_depth: 1,
            ..Limits::default()
        };
        let doc = summarize(&fs, &entry(&fs, "/r"), &shallow_depth, true).expect("summary");
        assert!(doc.truncated);
        assert_eq!(doc.files, 2, "only the root's own files were counted");
        assert_eq!(doc.dirs, 2, "subfolders are counted but not entered");
    }

    #[test]
    fn recursive_scan_marks_unreadable_subfolders_and_keeps_going() {
        let fs = FaultFs::new(tree());
        fs.faults().unreadable_dirs.push(uri("/r/docs"));
        let doc = summarize(&fs, &entry(&fs, "/r"), &Limits::default(), true).expect("summary");
        assert_eq!(doc.unreadable, 1);
        assert!(!doc.truncated);
        assert_eq!((doc.files, doc.dirs), (3, 2));
        assert_eq!(doc.bytes, 11);
        assert!(doc.items.iter().all(|e| !e.name.starts_with("docs/")));
    }

    #[test]
    fn a_zero_time_budget_stops_immediately() {
        let fs = tree();
        let limits = Limits {
            dir_scan_millis: 0,
            ..Limits::default()
        };
        let doc = summarize(&fs, &entry(&fs, "/r"), &limits, true).expect("summary");
        assert!(doc.truncated && doc.items_truncated);
    }

    #[test]
    fn natural_order_compares_numbers_by_value() {
        assert_eq!(natural_cmp("img2.jpg", "img10.jpg"), Ordering::Less);
        assert_eq!(natural_cmp("IMG_01", "img_1"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_eq!(natural_cmp("a", "a1"), Ordering::Less);
        assert_eq!(natural_cmp("", ""), Ordering::Equal);
    }
}
