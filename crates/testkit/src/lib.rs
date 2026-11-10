//! Test helpers shared across crates: the in-memory [`Vfs`] implementation ([`MemFs`]),
//! builders for trees of paths, and (later) adversarial-input generators.
//!
//! `MemFs` covers every trait method so `ops`/`engine` tests need no disk and no
//! network. It is `Send + Sync` and safe to share across job lanes.

pub mod fault;

pub use fault::{FaultFs, Faults};

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use fm_vfs::{Entry, EntryKind, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

#[derive(Clone, Debug)]
enum Node {
    Dir,
    File { data: Vec<u8>, modified: SystemTime },
    Symlink { target: String },
}

#[derive(Debug, Default)]
struct Inner {
    /// Key = `VfsUri::to_string()` (canonical form).
    nodes: BTreeMap<String, Node>,
}

/// In-memory filesystem. The root directory always exists.
#[derive(Clone, Debug, Default)]
pub struct MemFs {
    inner: Arc<Mutex<Inner>>,
    scheme: Option<Scheme>,
}

impl MemFs {
    pub fn new() -> Self {
        let fs = Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            scheme: None,
        };
        if let Ok(root) = VfsUri::parse("/") {
            fs.lock().nodes.insert(root.to_string(), Node::Dir);
        }
        fs
    }

    /// Serve `scheme` (default `file`), to stand in for a remote backend.
    pub fn with_scheme(mut self, scheme: Scheme) -> Self {
        self.scheme = Some(scheme);
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Poison recovery is safe: the store holds plain data, no invariants span calls.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Create `uri` and every missing ancestor directory.
    pub fn mkdirs(&self, uri: &VfsUri) -> Result<(), FmError> {
        let mut chain: Vec<VfsUri> = Vec::new();
        let mut cur = uri.clone();
        loop {
            chain.push(cur.clone());
            match cur.parent() {
                Some(p) => cur = p,
                None => break,
            }
        }
        chain.reverse();
        let mut g = self.lock();
        for u in chain {
            let key = u.to_string();
            match g.nodes.get(&key) {
                Some(Node::Dir) => continue,
                Some(Node::File { .. } | Node::Symlink { .. }) => {
                    return Err(FmError::NotADirectory(key));
                }
                None => {
                    g.nodes.insert(key, Node::Dir);
                }
            }
        }
        Ok(())
    }

    /// Create (or overwrite) a file with `data`, creating parent directories.
    pub fn put_file(&self, uri: &VfsUri, data: &[u8]) -> Result<(), FmError> {
        self.put_file_at(uri, data, SystemTime::now())
    }

    /// Like [`Self::put_file`] but with an explicit mtime (snapshot determinism).
    pub fn put_file_at(
        &self,
        uri: &VfsUri,
        data: &[u8],
        modified: SystemTime,
    ) -> Result<(), FmError> {
        let parent = uri
            .parent()
            .ok_or(FmError::Unsupported("MemFs: cannot put the root"))?;
        self.mkdirs(&parent)?;
        let mut g = self.lock();
        let key = uri.to_string();
        if matches!(g.nodes.get(&key), Some(Node::Dir)) {
            return Err(FmError::NotADirectory(key));
        }
        g.nodes.insert(
            key,
            Node::File {
                data: data.to_vec(),
                modified,
            },
        );
        Ok(())
    }

    /// Create a symlink node holding `target` verbatim (test fixture helper).
    pub fn put_symlink(&self, uri: &VfsUri, target: &str) -> Result<(), FmError> {
        let parent = uri
            .parent()
            .ok_or(FmError::Unsupported("MemFs: cannot put the root"))?;
        self.mkdirs(&parent)?;
        self.create_symlink(target, uri)
    }

    /// Read a file's bytes.
    pub fn get_file(&self, uri: &VfsUri) -> Result<Vec<u8>, FmError> {
        let g = self.lock();
        match g.nodes.get(&uri.to_string()) {
            Some(Node::File { data, .. }) => Ok(data.clone()),
            Some(Node::Dir | Node::Symlink { .. }) => Err(FmError::NotADirectory(uri.to_string())),
            None => Err(FmError::NotFound(uri.to_string())),
        }
    }

    pub fn exists(&self, uri: &VfsUri) -> bool {
        self.lock().nodes.contains_key(&uri.to_string())
    }

    /// Number of nodes (root included) — handy in assertions.
    pub fn len(&self) -> usize {
        self.lock().nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 1
    }

    // String conveniences so tests stay short.

    pub fn mkdirs_str(&self, path: &str) -> Result<(), FmError> {
        let uri = VfsUri::parse(path)?;
        self.mkdirs(&uri)
    }

    pub fn put_str(&self, path: &str, data: &[u8]) -> Result<(), FmError> {
        let uri = VfsUri::parse(path)?;
        self.put_file(&uri, data)
    }

    pub fn get_str(&self, path: &str) -> Result<Vec<u8>, FmError> {
        let uri = VfsUri::parse(path)?;
        self.get_file(&uri)
    }
}

/// Build a hostile in-memory tree under `root` (capability `security`):
/// `count` numbered files, a max-length (4 KiB) name, unicode-heavy names,
/// spaces/quotes, and deep nesting. Control-character and non-UTF-8 names are
/// rejected by `VfsUri` on purpose — those are covered by the `fs-local`
/// integration suite against a real directory.
pub fn hostile_tree(root: &str, count: usize) -> MemFs {
    #[allow(clippy::expect_used)]
    fn build(root: &str, count: usize) -> MemFs {
        let fs = MemFs::new();
        fs.mkdirs_str(root).expect("root");
        for i in 0..count {
            let name = format!("{root}/file-{i:06}.bin");
            fs.put_str(&name, b"x").expect("file");
        }
        let long = "L".repeat(4096);
        fs.put_str(&format!("{root}/{long}"), b"long")
            .expect("long name");
        for (i, name) in [
            "cjk-日本語-文件.txt",
            "emoji-🦀-🔥.txt",
            "spaces and dots ... fine.txt",
            "quote-'\"-percent%20.txt",
            "mixedCase.DOCX",
            "UPPER.txt",
        ]
        .iter()
        .enumerate()
        {
            fs.put_str(&format!("{root}/{name}"), &[i as u8])
                .expect("name");
        }
        let mut deep = root.to_string();
        for level in 0..12 {
            deep = format!("{deep}/lvl{level}");
            fs.mkdirs_str(&deep).expect("deep");
        }
        fs.put_str(&format!("{deep}/leaf.txt"), b"leaf")
            .expect("leaf");
        fs
    }
    build(root, count)
}

fn child_prefix(uri: &VfsUri) -> String {
    let key = uri.to_string();
    if key.ends_with('/') {
        key
    } else {
        format!("{key}/")
    }
}

fn entry_of(uri: VfsUri, node: &Node) -> Entry {
    match node {
        Node::Dir => Entry::new(uri, EntryKind::Dir),
        Node::File { data, modified } => {
            let mut e = Entry::new(uri, EntryKind::File);
            e.size = Some(data.len() as u64);
            e.modified = Some(*modified);
            e
        }
        Node::Symlink { target } => Entry::new(
            uri,
            EntryKind::Symlink {
                target: target.clone(),
            },
        ),
    }
}

struct MemWriter {
    fs: MemFs,
    key: String,
    buf: Vec<u8>,
    committed: bool,
}

impl MemWriter {
    fn commit(&mut self) -> std::io::Result<()> {
        if self.committed {
            return Ok(());
        }
        let mut g = self.fs.lock();
        g.nodes.insert(
            self.key.clone(),
            Node::File {
                data: std::mem::take(&mut self.buf),
                modified: SystemTime::now(),
            },
        );
        self.committed = true;
        Ok(())
    }
}

impl Write for MemWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.commit()
    }
}

impl Drop for MemWriter {
    fn drop(&mut self) {
        // Best-effort commit (Drop cannot report errors); tests that care call flush.
        let _ = self.commit();
    }
}

impl Vfs for MemFs {
    fn scheme(&self) -> Scheme {
        self.scheme.unwrap_or(Scheme::File)
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        let g = self.lock();
        match g.nodes.get(&uri.to_string()) {
            None => Err(FmError::NotFound(uri.to_string())),
            Some(Node::File { .. } | Node::Symlink { .. }) => {
                Err(FmError::NotADirectory(uri.to_string()))
            }
            Some(Node::Dir) => {
                let prefix = child_prefix(uri);
                let mut out = Vec::new();
                for (key, node) in &g.nodes {
                    if let Some(rest) = key.strip_prefix(&prefix)
                        && !rest.is_empty()
                        && !rest.contains('/')
                        && let Ok(child) = VfsUri::parse(key)
                    {
                        out.push(entry_of(child, node));
                    }
                }
                Ok(out)
            }
        }
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        let g = self.lock();
        match g.nodes.get(&uri.to_string()) {
            Some(node) => Ok(entry_of(uri.clone(), node)),
            None => Err(FmError::NotFound(uri.to_string())),
        }
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn std::io::Read + Send>, FmError> {
        let g = self.lock();
        match g.nodes.get(&uri.to_string()) {
            Some(Node::File { data, .. }) => Ok(Box::new(std::io::Cursor::new(data.clone()))),
            Some(Node::Dir) => Err(FmError::NotADirectory(uri.to_string())),
            Some(Node::Symlink { .. }) => {
                Err(FmError::Unsupported("MemFs: symlinks are never followed"))
            }
            None => Err(FmError::NotFound(uri.to_string())),
        }
    }

    fn open_write(
        &self,
        uri: &VfsUri,
        opts: WriteOpts,
    ) -> Result<Box<dyn std::io::Write + Send>, FmError> {
        let g = self.lock();
        let parent = uri
            .parent()
            .ok_or(FmError::Unsupported("MemFs: cannot write the root"))?;
        match g.nodes.get(&parent.to_string()) {
            Some(Node::Dir) => {}
            Some(Node::File { .. } | Node::Symlink { .. }) => {
                return Err(FmError::NotADirectory(parent.to_string()));
            }
            None => return Err(FmError::NotFound(parent.to_string())),
        }
        let key = uri.to_string();
        let existing = g.nodes.get(&key).cloned();
        match &existing {
            Some(Node::Dir) => return Err(FmError::NotADirectory(key)),
            Some(Node::File { .. }) if !opts.create && !opts.truncate && !opts.append => {
                // Existing file, no create/truncate/append: opening for write is fine.
            }
            None if !opts.create => return Err(FmError::NotFound(key)),
            _ => {}
        }
        let buf = match (&existing, opts.append) {
            (Some(Node::File { data, .. }), true) => data.clone(),
            (Some(Node::File { data, .. }), false) if !opts.truncate => data.clone(),
            _ => Vec::new(),
        };
        drop(g);
        Ok(Box::new(MemWriter {
            fs: self.clone(),
            key,
            buf,
            committed: false,
        }))
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        let mut g = self.lock();
        let key = uri.to_string();
        if g.nodes.contains_key(&key) {
            return Err(FmError::AlreadyExists(key));
        }
        let parent = uri
            .parent()
            .ok_or(FmError::Unsupported("MemFs: root always exists"))?;
        match g.nodes.get(&parent.to_string()) {
            Some(Node::Dir) => {}
            Some(Node::File { .. } | Node::Symlink { .. }) => {
                return Err(FmError::NotADirectory(parent.to_string()));
            }
            None => return Err(FmError::NotFound(parent.to_string())),
        }
        g.nodes.insert(key, Node::Dir);
        Ok(())
    }

    fn create_symlink(&self, target: &str, link: &VfsUri) -> Result<(), FmError> {
        let mut g = self.lock();
        let key = link.to_string();
        if g.nodes.contains_key(&key) {
            return Err(FmError::AlreadyExists(key));
        }
        let parent = link
            .parent()
            .ok_or(FmError::Unsupported("MemFs: cannot link the root"))?;
        match g.nodes.get(&parent.to_string()) {
            Some(Node::Dir) => {}
            Some(_) => return Err(FmError::NotADirectory(parent.to_string())),
            None => return Err(FmError::NotFound(parent.to_string())),
        }
        g.nodes.insert(
            key,
            Node::Symlink {
                target: target.to_string(),
            },
        );
        Ok(())
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        if uri.path_str() == "/" {
            return Err(FmError::Unsupported("MemFs: cannot remove the root"));
        }
        let mut g = self.lock();
        let key = uri.to_string();
        match g.nodes.get(&key) {
            None => return Err(FmError::NotFound(key)),
            Some(Node::File { .. } | Node::Symlink { .. }) => {}
            Some(Node::Dir) => {
                let prefix = child_prefix(uri);
                if g.nodes.keys().any(|k| k.starts_with(&prefix)) {
                    return Err(FmError::NotEmpty(key));
                }
            }
        }
        g.nodes.remove(&key);
        Ok(())
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        if from.path_str() == "/" {
            return Err(FmError::Unsupported("MemFs: cannot rename the root"));
        }
        let mut g = self.lock();
        let from_key = from.to_string();
        if !g.nodes.contains_key(&from_key) {
            return Err(FmError::NotFound(from_key));
        }
        let to_key = to.to_string();
        if g.nodes.contains_key(&to_key) {
            return Err(FmError::AlreadyExists(to_key));
        }
        let to_parent = to
            .parent()
            .ok_or_else(|| FmError::NotADirectory(to_key.clone()))?;
        match g.nodes.get(&to_parent.to_string()) {
            Some(Node::Dir) => {}
            Some(Node::File { .. } | Node::Symlink { .. }) => {
                return Err(FmError::NotADirectory(to_parent.to_string()));
            }
            None => return Err(FmError::NotFound(to_parent.to_string())),
        }
        let from_prefix = child_prefix(from);
        let moving: Vec<(String, String)> = g
            .nodes
            .keys()
            .filter_map(|k| {
                if k == &from_key {
                    Some((k.clone(), to_key.clone()))
                } else {
                    k.strip_prefix(&from_prefix)
                        .map(|rest| (k.clone(), format!("{to_key}/{rest}")))
                }
            })
            .collect();
        let nodes: Vec<(String, Node)> = moving
            .iter()
            .filter_map(|(old, _)| g.nodes.remove(old).map(|n| (old.clone(), n)))
            .collect();
        for (old, node) in nodes {
            let new = moving
                .iter()
                .find(|(o, _)| *o == old)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| old.clone());
            g.nodes.insert(new, node);
        }
        Ok(())
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        a.scheme() == b.scheme() && a.authority() == b.authority()
    }

    fn space(&self, _uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        Ok(Some(SpaceInfo {
            total: 1 << 40,
            available: 1 << 39,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> MemFs {
        let fs = MemFs::new();
        fs.mkdirs_str("/home/alice").expect("mkdirs");
        fs.put_str("/home/alice/a.txt", b"hello").expect("put");
        fs.put_str("/home/alice/b.txt", b"world").expect("put");
        fs.mkdirs_str("/home/alice/Docs").expect("mkdirs");
        fs.put_str("/home/alice/Docs/c.txt", b"deep").expect("put");
        fs
    }

    #[test]
    fn put_get_round_trip() {
        let fs = fs();
        assert_eq!(fs.get_str("/home/alice/a.txt").expect("get"), b"hello");
        assert_eq!(fs.get_str("/home/alice/Docs/c.txt").expect("get"), b"deep");
    }

    #[test]
    fn read_dir_lists_only_direct_children() {
        let fs = fs();
        let home = VfsUri::parse("/home/alice").expect("uri");
        let mut names: Vec<String> = fs
            .read_dir(&home)
            .expect("read_dir")
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["Docs", "a.txt", "b.txt"]);
        let docs = VfsUri::parse("/home/alice/Docs").expect("uri");
        let entries = fs.read_dir(&docs).expect("read_dir");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_file());
    }

    #[test]
    fn stat_reports_kind_and_size() {
        let fs = fs();
        let f = VfsUri::parse("/home/alice/a.txt").expect("uri");
        let e = fs.stat(&f).expect("stat");
        assert!(e.is_file());
        assert_eq!(e.size, Some(5));
        let d = VfsUri::parse("/home/alice/Docs").expect("uri");
        assert!(fs.stat(&d).expect("stat").is_dir());
        let missing = VfsUri::parse("/nope").expect("uri");
        assert!(matches!(fs.stat(&missing), Err(FmError::NotFound(_))));
    }

    #[test]
    fn open_write_truncates_by_default_and_appends_on_request() {
        let fs = fs();
        let uri = VfsUri::parse("/home/alice/a.txt").expect("uri");
        let mut w = fs
            .open_write(&uri, WriteOpts::default())
            .expect("open_write");
        w.write_all(b"new").expect("write");
        w.flush().expect("flush");
        assert_eq!(fs.get_file(&uri).expect("get"), b"new");

        let mut w = fs
            .open_write(
                &uri,
                WriteOpts {
                    create: true,
                    truncate: false,
                    append: true,
                },
            )
            .expect("open_write");
        w.write_all(b"+more").expect("write");
        w.flush().expect("flush");
        assert_eq!(fs.get_file(&uri).expect("get"), b"new+more");
    }

    #[test]
    fn write_missing_parent_fails() {
        let fs = MemFs::new();
        let uri = VfsUri::parse("/ghost/file.txt").expect("uri");
        assert!(matches!(
            fs.open_write(&uri, WriteOpts::default()),
            Err(FmError::NotFound(_))
        ));
    }

    #[test]
    fn create_dir_conflicts() {
        let fs = fs();
        let docs = VfsUri::parse("/home/alice/Docs").expect("uri");
        assert!(matches!(
            fs.create_dir(&docs),
            Err(FmError::AlreadyExists(_))
        ));
        let file = VfsUri::parse("/home/alice/a.txt").expect("uri");
        assert!(matches!(
            fs.create_dir(&file),
            Err(FmError::AlreadyExists(_))
        ));
    }

    #[test]
    fn remove_rules() {
        let fs = fs();
        let docs = VfsUri::parse("/home/alice/Docs").expect("uri");
        assert!(matches!(fs.remove(&docs), Err(FmError::NotEmpty(_))));
        let c = VfsUri::parse("/home/alice/Docs/c.txt").expect("uri");
        fs.remove(&c).expect("remove file");
        fs.remove(&docs).expect("remove empty dir");
        assert!(!fs.exists(&docs));
        let root = VfsUri::parse("/").expect("uri");
        assert!(matches!(fs.remove(&root), Err(FmError::Unsupported(_))));
    }

    #[test]
    fn rename_moves_subtree() {
        let fs = fs();
        let from = VfsUri::parse("/home/alice/Docs").expect("uri");
        let to = VfsUri::parse("/home/alice/Archive").expect("uri");
        fs.rename(&from, &to).expect("rename");
        assert!(!fs.exists(&from));
        assert_eq!(
            fs.get_str("/home/alice/Archive/c.txt").expect("get"),
            b"deep"
        );
        // sibling with a shared prefix is untouched
        assert!(fs.exists(&VfsUri::parse("/home/alice/a.txt").expect("uri")));
    }

    #[test]
    fn rename_conflicts_and_missing() {
        let fs = fs();
        let a = VfsUri::parse("/home/alice/a.txt").expect("uri");
        let b = VfsUri::parse("/home/alice/b.txt").expect("uri");
        assert!(matches!(fs.rename(&a, &b), Err(FmError::AlreadyExists(_))));
        let ghost = VfsUri::parse("/ghost").expect("uri");
        assert!(matches!(fs.rename(&ghost, &b), Err(FmError::NotFound(_))));
    }

    #[test]
    fn read_dir_on_file_is_not_a_directory() {
        let fs = fs();
        let a = VfsUri::parse("/home/alice/a.txt").expect("uri");
        assert!(matches!(fs.read_dir(&a), Err(FmError::NotADirectory(_))));
    }

    #[test]
    fn root_is_special() {
        let fs = MemFs::new();
        let root = VfsUri::parse("/").expect("uri");
        assert!(fs.stat(&root).expect("stat").is_dir());
        assert!(matches!(
            fs.open_write(&root, WriteOpts::default()),
            Err(FmError::Unsupported(_))
        ));
        assert!(matches!(
            fs.rename(&root, &VfsUri::parse("/x").expect("uri")),
            Err(FmError::Unsupported(_))
        ));
    }

    #[test]
    fn same_device_and_space() {
        let fs = fs();
        let a = VfsUri::parse("/a").expect("uri");
        let b = VfsUri::parse("/b").expect("uri");
        assert!(fs.same_device(&a, &b));
        let remote = VfsUri::parse("smb://nas/share").expect("uri");
        assert!(!fs.same_device(&a, &remote));
        assert!(fs.space(&a).expect("space").is_some());
    }

    #[test]
    fn concurrent_writes_to_different_files() {
        use std::thread;
        let fs = fs();
        let mut handles = Vec::new();
        for i in 0..4 {
            let fs = fs.clone();
            handles.push(thread::spawn(move || {
                let path = format!("/home/alice/t{i}.txt");
                fs.put_str(&path, format!("data{i}").as_bytes())
                    .expect("put");
                assert_eq!(
                    fs.get_str(&path).expect("get"),
                    format!("data{i}").as_bytes()
                );
            }));
        }
        for h in handles {
            h.join().expect("thread");
        }
    }
}
