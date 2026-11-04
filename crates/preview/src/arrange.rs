//! Display order for a folder's thumbnail grid: sort, optional grouping by subfolder.

use std::cmp::Ordering;

use fm_vfs::Entry;

use crate::DirDoc;
use crate::dir::natural_cmp;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThumbSort {
    #[default]
    Name,
    /// Newest first.
    Modified,
    Type,
}

impl ThumbSort {
    pub const ALL: [(ThumbSort, &'static str); 3] = [
        (ThumbSort::Name, "name"),
        (ThumbSort::Modified, "modified"),
        (ThumbSort::Type, "type"),
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(s, _)| *s)
    }

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(s, _)| *s == self)
            .map_or("name", |(_, n)| n)
    }
}

/// A run of tiles under one subfolder heading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Index of the group's first tile in [`Arranged::items`].
    pub start: usize,
    /// Folder path relative to the previewed folder; empty for its own files.
    pub folder: String,
}

#[derive(Clone, Debug, Default)]
pub struct Arranged {
    pub items: Vec<Entry>,
    /// Empty unless the grid is grouped.
    pub groups: Vec<Group>,
}

/// The folder part of a recursive item name (`a/b/c.txt` → `a/b`).
pub fn folder_of(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// The file part of a recursive item name.
pub fn base_name(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(_, base)| base)
}

fn extension(name: &str) -> String {
    match base_name(name).rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_lowercase(),
        _ => String::new(),
    }
}

fn compare(sort: ThumbSort, a: &Entry, b: &Entry) -> Ordering {
    let by_name = || {
        natural_cmp(base_name(&a.name), base_name(&b.name))
            .then_with(|| natural_cmp(&a.name, &b.name))
    };
    match sort {
        ThumbSort::Name => by_name(),
        ThumbSort::Modified => b.modified.cmp(&a.modified).then_with(by_name),
        ThumbSort::Type => extension(&a.name)
            .cmp(&extension(&b.name))
            .then_with(by_name),
    }
}

/// Order `doc.items` for display. A recursive grid lists files only (folders are
/// represented by group headings); a plain folder keeps its subfolders first.
pub fn arrange(doc: &DirDoc, sort: ThumbSort, group: bool) -> Arranged {
    let mut items: Vec<Entry> = doc
        .items
        .iter()
        .filter(|e| !(doc.recursive && e.is_dir_like()))
        .cloned()
        .collect();
    let grouped = doc.recursive && group;
    items.sort_by(|a, b| {
        let folders = b.is_dir_like().cmp(&a.is_dir_like());
        let place = if grouped {
            natural_cmp(folder_of(&a.name), folder_of(&b.name))
        } else {
            Ordering::Equal
        };
        folders.then(place).then_with(|| compare(sort, a, b))
    });
    let mut groups: Vec<Group> = Vec::new();
    if grouped {
        for (i, e) in items.iter().enumerate() {
            let folder = folder_of(&e.name);
            if groups.last().is_none_or(|g| g.folder != folder) {
                groups.push(Group {
                    start: i,
                    folder: folder.to_string(),
                });
            }
        }
    }
    Arranged { items, groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DirDoc, Limits};
    use fm_testkit::MemFs;

    fn doc(recursive: bool) -> DirDoc {
        let fs = MemFs::new();
        fs.mkdirs_str("/r/sub/deep").expect("dirs");
        for (path, body) in [
            ("/r/b.png", &b"1"[..]),
            ("/r/a.txt", b"1"),
            ("/r/sub/c.png", b"1"),
            ("/r/sub/A.png", b"1"),
            ("/r/sub/deep/z.txt", b"1"),
        ] {
            fs.put_str(path, body).expect("file");
        }
        let root =
            fm_vfs::Vfs::stat(&fs, &fm_vfs::VfsUri::parse("/r").expect("uri")).expect("stat");
        crate::dir::summarize(&fs, &root, &Limits::default(), recursive).expect("doc")
    }

    fn names(a: &Arranged) -> Vec<&str> {
        a.items.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn recursive_grids_drop_folder_tiles_and_group_by_folder() {
        let a = arrange(&doc(true), ThumbSort::Name, true);
        assert_eq!(
            names(&a),
            ["a.txt", "b.png", "sub/A.png", "sub/c.png", "sub/deep/z.txt"]
        );
        let starts: Vec<(usize, &str)> = a
            .groups
            .iter()
            .map(|g| (g.start, g.folder.as_str()))
            .collect();
        assert_eq!(starts, [(0, ""), (2, "sub"), (4, "sub/deep")]);
    }

    #[test]
    fn flat_mode_sorts_every_file_by_name_regardless_of_folder() {
        let a = arrange(&doc(true), ThumbSort::Name, false);
        assert_eq!(
            names(&a),
            ["sub/A.png", "a.txt", "b.png", "sub/c.png", "sub/deep/z.txt"]
        );
        assert!(a.groups.is_empty());
    }

    #[test]
    fn type_sort_groups_extensions_and_plain_folders_keep_subfolders_first() {
        let a = arrange(&doc(true), ThumbSort::Type, false);
        assert_eq!(
            names(&a),
            ["sub/A.png", "b.png", "sub/c.png", "a.txt", "sub/deep/z.txt"]
        );
        let plain = arrange(&doc(false), ThumbSort::Name, true);
        assert_eq!(names(&plain), ["sub", "a.txt", "b.png"]);
        assert!(plain.groups.is_empty());
    }

    #[test]
    fn sort_names_round_trip() {
        for (sort, name) in ThumbSort::ALL {
            assert_eq!(ThumbSort::parse(name), Some(sort));
            assert_eq!(sort.name(), name);
        }
        assert_eq!(ThumbSort::parse("nope"), None);
    }
}
