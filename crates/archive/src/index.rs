//! The directory tree of one archive, built once per mount from the format's entry list.
//!
//! Names are hostile input: they are normalised here (backslashes, absolute paths, drive
//! letters and `..` cannot leave the archive root; control characters become visible
//! control pictures) and every limit in [`crate::limits`] is enforced while building.

use std::collections::HashMap;
use std::time::SystemTime;

use fm_vfs::FmError;

use crate::limits::{MAX_DEPTH, MAX_ENTRIES, MAX_LINK_TARGET, MAX_NAME_BYTES};

pub type NodeId = usize;

/// The root folder's id in every index.
pub const ROOT: NodeId = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    /// A link stored in the archive; the target is shown, never followed.
    Symlink(String),
    /// Devices, FIFOs, hard links and other special entries: listed, never opened.
    Other,
}

/// Metadata one format reports for an entry.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub size: Option<u64>,
    pub packed: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Unix mode bits (`& 0o7777`), when the format stores them.
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub encrypted: bool,
    /// Where the format finds the data (zip index, tar offset, 7z entry index, …).
    pub locator: u64,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub parent: NodeId,
    pub kind: NodeKind,
    pub meta: Meta,
    /// `true` for folders that exist only because a deeper path named them.
    pub implicit: bool,
    children: Vec<NodeId>,
}

/// One archive's tree. Paths are `/`-separated and relative to the archive root, without
/// a leading slash (`""` is the root).
#[derive(Clone, Debug)]
pub struct Index {
    nodes: Vec<Node>,
    by_path: HashMap<String, NodeId>,
    /// Entries replaced by a later entry with the same path.
    pub shadowed: usize,
    /// Entries skipped because their name normalised to nothing (e.g. `./`).
    pub skipped: usize,
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

impl Index {
    pub fn new() -> Self {
        let root = Node {
            name: String::new(),
            parent: ROOT,
            kind: NodeKind::Dir,
            meta: Meta::default(),
            implicit: true,
            children: Vec::new(),
        };
        let mut by_path = HashMap::new();
        by_path.insert(String::new(), ROOT);
        Self {
            nodes: vec![root],
            by_path,
            shadowed: 0,
            skipped: 0,
        }
    }

    /// Number of entries (folders and files), without the root.
    pub fn len(&self) -> usize {
        self.nodes.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes.get(id).map_or(&[], |n| n.children.as_slice())
    }

    /// Look up a normalised path (`"a/b"`, `""` for the root; leading `/` tolerated).
    pub fn lookup(&self, path: &str) -> Option<NodeId> {
        self.by_path.get(path.trim_start_matches('/')).copied()
    }

    /// The path of `id` relative to the root (`""` for the root).
    pub fn path_of(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut at = id;
        let mut guard = 0;
        while at != ROOT && guard <= MAX_DEPTH {
            let Some(node) = self.nodes.get(at) else {
                break;
            };
            parts.push(node.name.as_str());
            at = node.parent;
            guard += 1;
        }
        parts.reverse();
        parts.join("/")
    }

    /// Every file node id with its path, depth-first (for whole-archive walks).
    pub fn files(&self) -> Vec<(NodeId, String)> {
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, String::new())];
        while let Some((id, path)) = stack.pop() {
            for &child in self.children(id).iter().rev() {
                let Some(node) = self.nodes.get(child) else {
                    continue;
                };
                let p = if path.is_empty() {
                    node.name.clone()
                } else {
                    format!("{path}/{}", node.name)
                };
                if node.kind == NodeKind::Dir {
                    stack.push((child, p));
                } else {
                    out.push((child, p));
                }
            }
        }
        out
    }

    /// Total unpacked and packed size of all files (sizes the format did not report count 0).
    pub fn totals(&self) -> (u64, u64) {
        self.nodes.iter().fold((0u64, 0u64), |(u, p), n| {
            (
                u.saturating_add(n.meta.size.unwrap_or(0)),
                p.saturating_add(n.meta.packed.unwrap_or(0)),
            )
        })
    }

    /// Add one entry by its raw name from the archive. Folders implied by the path are
    /// created; a later entry with the same path replaces the earlier one (counted in
    /// [`Self::shadowed`]). Returns `None` when the name normalised to nothing.
    pub fn add(
        &mut self,
        raw_name: &str,
        kind: NodeKind,
        meta: Meta,
    ) -> Result<Option<NodeId>, FmError> {
        if self.nodes.len() > MAX_ENTRIES {
            return Err(too_many());
        }
        let parts = normalize(raw_name)?;
        let Some((leaf, dirs)) = parts.split_last() else {
            self.skipped += 1;
            return Ok(None);
        };
        let mut parent = ROOT;
        let mut path = String::new();
        for dir in dirs {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(dir);
            parent = match self.by_path.get(&path) {
                Some(&id) if self.is_dir(id) => id,
                Some(&id) => {
                    // A file is in the way of a folder: the folder wins, the file is shadowed.
                    self.shadowed += 1;
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.kind = NodeKind::Dir;
                        node.meta = Meta::default();
                        node.implicit = true;
                    }
                    id
                }
                None => self.push(
                    parent,
                    dir,
                    path.clone(),
                    NodeKind::Dir,
                    Meta::default(),
                    true,
                )?,
            };
        }
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(leaf);
        let kind = match kind {
            NodeKind::Symlink(target) => NodeKind::Symlink(cap_target(target)),
            other => other,
        };
        if let Some(&id) = self.by_path.get(&path) {
            let Some(node) = self.nodes.get_mut(id) else {
                return Ok(None);
            };
            let both_dirs = node.kind == NodeKind::Dir && kind == NodeKind::Dir;
            if both_dirs {
                // An explicit folder entry after an implied one only brings metadata.
                node.meta = meta;
                node.implicit = false;
                return Ok(Some(id));
            }
            if node.kind == NodeKind::Dir && !node.children.is_empty() {
                // Never replace a folder that has contents by a file.
                self.shadowed += 1;
                return Ok(Some(id));
            }
            if !node.implicit {
                self.shadowed += 1;
            }
            node.kind = kind;
            node.meta = meta;
            node.implicit = false;
            return Ok(Some(id));
        }
        self.push(parent, leaf, path, kind, meta, false).map(Some)
    }

    pub(crate) fn set_kind(&mut self, id: NodeId, kind: NodeKind) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.kind = kind;
        }
    }

    fn is_dir(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|n| n.kind == NodeKind::Dir)
    }

    fn push(
        &mut self,
        parent: NodeId,
        name: &str,
        path: String,
        kind: NodeKind,
        meta: Meta,
        implicit: bool,
    ) -> Result<NodeId, FmError> {
        if self.nodes.len() > MAX_ENTRIES {
            return Err(too_many());
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            name: name.to_string(),
            parent,
            kind,
            meta,
            implicit,
            children: Vec::new(),
        });
        if let Some(p) = self.nodes.get_mut(parent) {
            p.children.push(id);
        }
        self.by_path.insert(path, id);
        Ok(id)
    }
}

fn too_many() -> FmError {
    FmError::backend(
        "archive",
        format!("too many entries (limit {})", group(MAX_ENTRIES)),
    )
}

fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn cap_target(mut target: String) -> String {
    if target.len() > MAX_LINK_TARGET {
        let mut cut = MAX_LINK_TARGET;
        while !target.is_char_boundary(cut) {
            cut -= 1;
        }
        target.truncate(cut);
    }
    target.chars().map(visible).collect()
}

/// A control character as its Unicode control picture (␀…␟, ␡), so hostile names stay
/// visible and can never act on a terminal or a path parser.
fn visible(c: char) -> char {
    match c {
        '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'),
        '\u{7f}' => '\u{2421}',
        c if c.is_control() => '\u{fffd}',
        c => c,
    }
}

/// Split an archive entry name into safe path components: `\` is a separator, a leading
/// drive letter or `/` is dropped, `.` is skipped, `..` is clamped at the archive root,
/// and control characters become control pictures.
pub fn normalize(raw: &str) -> Result<Vec<String>, FmError> {
    if raw.len() > MAX_NAME_BYTES {
        return Err(FmError::backend(
            "archive",
            format!("entry name longer than {MAX_NAME_BYTES} bytes"),
        ));
    }
    let unified = raw.replace('\\', "/");
    let mut rest = unified.as_str();
    if let [d, b':', ..] = rest.as_bytes()
        && d.is_ascii_alphabetic()
    {
        rest = rest.get(2..).unwrap_or("");
    }
    let mut out: Vec<String> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => {
                if out.len() >= MAX_DEPTH {
                    return Err(FmError::backend(
                        "archive",
                        format!("entry nested deeper than {MAX_DEPTH} folders"),
                    ));
                }
                out.push(s.chars().map(visible).collect());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64) -> Meta {
        Meta {
            size: Some(size),
            ..Meta::default()
        }
    }

    #[test]
    fn hostile_names_stay_inside_the_root() {
        for (raw, want) in [
            ("../../.bashrc", vec![".bashrc"]),
            ("/etc/passwd", vec!["etc", "passwd"]),
            ("C:\\Windows\\win.ini", vec!["Windows", "win.ini"]),
            ("a/./b/../c", vec!["a", "c"]),
            ("a\u{7}b", vec!["a\u{2407}b"]),
            ("./", vec![]),
        ] {
            assert_eq!(normalize(raw).expect(raw), want, "{raw:?}");
        }
        assert!(normalize(&"x".repeat(MAX_NAME_BYTES + 1)).is_err());
        assert!(normalize(&"d/".repeat(MAX_DEPTH + 1)).is_err());
    }

    #[test]
    fn implicit_folders_duplicates_and_lookup() {
        let mut idx = Index::new();
        idx.add("www/css/site.css", NodeKind::File, file(10))
            .expect("add");
        idx.add("www/index.html", NodeKind::File, file(5))
            .expect("add");
        idx.add("www/", NodeKind::Dir, Meta::default())
            .expect("add");
        idx.add("www/index.html", NodeKind::File, file(7))
            .expect("dup");
        idx.add("./", NodeKind::Dir, Meta::default()).expect("dot");
        let www = idx.lookup("www").expect("www");
        assert_eq!(idx.children(www).len(), 2);
        let page = idx.lookup("/www/index.html").expect("page");
        assert_eq!(idx.node(page).and_then(|n| n.meta.size), Some(7));
        assert_eq!(idx.path_of(page), "www/index.html");
        assert_eq!(idx.shadowed, 1);
        assert_eq!(idx.skipped, 1);
        assert_eq!(idx.len(), 4);
        assert_eq!(idx.totals().0, 17);
        let mut files: Vec<String> = idx.files().into_iter().map(|(_, p)| p).collect();
        files.sort();
        assert_eq!(files, vec!["www/css/site.css", "www/index.html"]);
    }

    #[test]
    fn a_file_never_replaces_a_folder_with_contents() {
        let mut idx = Index::new();
        idx.add("a/b.txt", NodeKind::File, file(1)).expect("add");
        idx.add("a", NodeKind::File, file(2)).expect("add");
        let a = idx.lookup("a").expect("a");
        assert_eq!(idx.node(a).map(|n| n.kind.clone()), Some(NodeKind::Dir));
        assert!(idx.lookup("a/b.txt").is_some());
        assert_eq!(idx.shadowed, 1);
    }

    #[test]
    fn symlink_targets_are_capped_and_visible() {
        let mut idx = Index::new();
        let long = format!("{}\u{1b}", "x".repeat(MAX_LINK_TARGET + 10));
        let id = idx
            .add("l", NodeKind::Symlink(long), Meta::default())
            .expect("add")
            .expect("id");
        let NodeKind::Symlink(t) = &idx.node(id).expect("node").kind else {
            panic!("kind");
        };
        assert!(t.len() <= MAX_LINK_TARGET);
        assert!(!t.chars().any(char::is_control));
    }

    #[test]
    fn grouping_reads_well() {
        assert_eq!(group(1_000_000), "1 000 000");
        assert_eq!(group(999), "999");
    }
}
