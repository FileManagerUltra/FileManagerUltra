//! [`ArchiveFs`]: one mounted archive served through the `Vfs` trait, so panes, the tree,
//! file jobs, Find and previews work inside it unchanged.

use std::io::{Read, Write};
use std::sync::Arc;

use fm_vfs::{Attrs, Entry, EntryKind, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

use crate::format::{FormatId, Info, Passwords, Reader};
use crate::index::{Index, NodeId, NodeKind};

pub struct ArchiveFs {
    root: VfsUri,
    reader: Box<dyn Reader>,
    passwords: Arc<Passwords>,
}

impl ArchiveFs {
    /// Serve `reader` under `root` (`archive://<handle>/`).
    pub fn new(root: VfsUri, reader: Box<dyn Reader>, passwords: Arc<Passwords>) -> Self {
        Self {
            root: root.root(),
            reader,
            passwords,
        }
    }

    pub fn root(&self) -> &VfsUri {
        &self.root
    }

    pub fn format(&self) -> FormatId {
        self.reader.format()
    }

    pub fn info(&self) -> Info {
        self.reader.info()
    }

    pub fn index(&self) -> &Index {
        self.reader.index()
    }

    pub fn passwords(&self) -> &Arc<Passwords> {
        &self.passwords
    }

    fn node_of(&self, uri: &VfsUri) -> Result<NodeId, FmError> {
        if uri.scheme() != Scheme::Archive || uri.authority() != self.root.authority() {
            return Err(FmError::NotFound(uri.to_string()));
        }
        self.index()
            .lookup(uri.path_str())
            .ok_or_else(|| FmError::NotFound(uri.to_string()))
    }

    fn entry(&self, uri: VfsUri, id: NodeId) -> Entry {
        let Some(node) = self.index().node(id) else {
            return Entry::new(uri, EntryKind::Other);
        };
        let kind = match &node.kind {
            NodeKind::File => EntryKind::File,
            NodeKind::Dir => EntryKind::Dir,
            NodeKind::Symlink(target) => EntryKind::Symlink {
                target: target.clone(),
            },
            NodeKind::Other => EntryKind::Other,
        };
        let link_to_dir = matches!(&node.kind, NodeKind::Symlink(t) if self.link_is_dir(id, t));
        let mut e = Entry::new(uri, kind);
        e.size = match node.kind {
            NodeKind::Dir => None,
            _ => node.meta.size,
        };
        e.modified = node.meta.modified;
        e.permissions = node.meta.mode;
        e.link_to_dir = link_to_dir;
        e
    }

    /// A link that resolves, inside the archive, to a folder (never outside it).
    fn link_is_dir(&self, id: NodeId, target: &str) -> bool {
        let Some(node) = self.index().node(id) else {
            return false;
        };
        let base = if target.starts_with('/') {
            String::new()
        } else {
            self.index().path_of(node.parent)
        };
        let joined = format!("{base}/{target}");
        crate::index::normalize(&joined)
            .ok()
            .and_then(|parts| self.index().lookup(&parts.join("/")))
            .and_then(|t| self.index().node(t))
            .is_some_and(|n| n.kind == NodeKind::Dir)
    }

    fn read_only(&self) -> FmError {
        FmError::Unsupported("changes inside archives are not supported yet")
    }
}

impl Vfs for ArchiveFs {
    fn scheme(&self) -> Scheme {
        Scheme::Archive
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        let id = self.node_of(uri)?;
        let node = self
            .index()
            .node(id)
            .ok_or_else(|| FmError::NotFound(uri.to_string()))?;
        if node.kind != NodeKind::Dir {
            return Err(FmError::NotADirectory(uri.to_string()));
        }
        let mut out = Vec::with_capacity(self.index().children(id).len());
        for &child in self.index().children(id) {
            let Some(c) = self.index().node(child) else {
                continue;
            };
            let Ok(child_uri) = uri.join(&c.name) else {
                continue;
            };
            out.push(self.entry(child_uri, child));
        }
        Ok(out)
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        let id = self.node_of(uri)?;
        Ok(self.entry(uri.clone(), id))
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        let id = self.node_of(uri)?;
        match self.index().node(id).map(|n| &n.kind) {
            Some(NodeKind::File) => {
                let r = crate::format::guarded(|| self.reader.open(id, &self.passwords))?;
                Ok(Box::new(crate::format::CatchRead(r)))
            }
            Some(NodeKind::Dir) => Err(FmError::NotADirectory(uri.to_string())),
            _ => Err(FmError::Unsupported(
                "special archive entries cannot be opened",
            )),
        }
    }

    fn open_write(&self, _: &VfsUri, _: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        Err(self.read_only())
    }

    fn create_dir(&self, _: &VfsUri) -> Result<(), FmError> {
        Err(self.read_only())
    }

    fn remove(&self, _: &VfsUri) -> Result<(), FmError> {
        Err(self.read_only())
    }

    fn rename(&self, _: &VfsUri, _: &VfsUri) -> Result<(), FmError> {
        Err(self.read_only())
    }

    fn stat_full(&self, uri: &VfsUri) -> Result<Attrs, FmError> {
        let id = self.node_of(uri)?;
        let node = self
            .index()
            .node(id)
            .ok_or_else(|| FmError::NotFound(uri.to_string()))?;
        Ok(Attrs {
            mtime: node.meta.modified,
            mode: node.meta.mode,
            uid: node.meta.uid,
            gid: node.meta.gid,
            ..Attrs::default()
        })
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        a.archive_handle().is_some() && a.archive_handle() == b.archive_handle()
    }

    fn space(&self, _: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Progress, open};
    use crate::source::MemSource;

    fn mounted() -> ArchiveFs {
        let src = Arc::new(MemSource::new(crate::format::zip::tests::sample_zip()));
        let passwords = Arc::new(Passwords::default());
        let reader = open(
            src,
            FormatId::Zip,
            "site.zip",
            &passwords,
            &Progress::none(),
        )
        .expect("open");
        let root = VfsUri::archive_root("0123456789abcdef").expect("root");
        ArchiveFs::new(root, reader, passwords)
    }

    #[test]
    fn lists_stats_and_reads_like_a_folder() {
        let fs = mounted();
        let root = fs.root().clone();
        let mut names: Vec<String> = fs
            .read_dir(&root)
            .expect("root")
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["big.bin", "evil.txt", "www"]);
        let www = root.join("www").expect("www");
        assert!(fs.stat(&www).expect("stat").is_dir());
        let page = www.join("index.html").expect("page");
        let mut s = String::new();
        fs.open_read(&page)
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        assert_eq!(s, "<h1>hi</h1>");
        assert!(matches!(fs.read_dir(&page), Err(FmError::NotADirectory(_))));
        assert!(fs.open_read(&www).is_err());
        let other = VfsUri::archive_root("ffffffffffffffff").expect("other");
        assert!(fs.stat(&other).is_err(), "another mount's uri is not ours");
        assert!(fs.create_dir(&root.join("new").expect("new")).is_err());
        let link = www.join("link").expect("link");
        let e = fs.stat(&link).expect("link");
        assert!(e.is_symlink() && !e.link_to_dir);
    }
}
