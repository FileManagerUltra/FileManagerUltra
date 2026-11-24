//! [`ArchiveFs`]: one mounted archive served through the `Vfs` trait, so panes, the tree,
//! file jobs, Find and previews work inside it unchanged.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use fm_vfs::{Attrs, Entry, EntryKind, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

use crate::format::{FormatId, Info, Passwords, Reader};
use crate::index::{Index, NodeId, NodeKind};
use crate::overlay::{Change, Data, OvEntry, Overlay, Resolved};

pub struct ArchiveFs {
    root: VfsUri,
    reader: Arc<dyn Reader>,
    passwords: Arc<Passwords>,
    overlay: Arc<Mutex<Overlay>>,
    /// Why changes are refused (`None` for writable archives).
    read_only: Option<&'static str>,
    /// Size and modification time of the archive file when it was opened.
    stamp: Option<(u64, Option<std::time::SystemTime>)>,
    /// A save is writing the archive: changes wait until it is done.
    saving: std::sync::atomic::AtomicBool,
}

impl ArchiveFs {
    /// Serve `reader` under `root` (`archive://<handle>/`). Formats the app can write
    /// accept staged changes; the others are read-only.
    pub fn new(root: VfsUri, reader: Box<dyn Reader>, passwords: Arc<Passwords>) -> Self {
        let read_only = (!crate::write::WRITABLE.contains(&reader.format()))
            .then_some("this kind of archive is read-only");
        Self {
            root: root.root(),
            reader: Arc::from(reader),
            passwords,
            overlay: Arc::new(Mutex::new(Overlay::default())),
            read_only,
            stamp: None,
            saving: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Remember the archive file's size and modification time (checked before Save).
    pub fn with_stamp(mut self, stamp: (u64, Option<std::time::SystemTime>)) -> Self {
        self.stamp = Some(stamp);
        self
    }

    pub fn stamp(&self) -> Option<(u64, Option<std::time::SystemTime>)> {
        self.stamp
    }

    /// The archive member whose bytes `uri` shows unchanged (moved or renamed counts as
    /// unchanged), so Save can copy it as stored.
    pub fn base_member(&self, uri: &VfsUri) -> Option<NodeId> {
        match self.resolve(uri).ok()? {
            Resolved::Base(id)
            | Resolved::Over(OvEntry::File {
                data: Data::Base(id),
                ..
            }) => Some(id),
            Resolved::Over(_) => None,
        }
    }

    /// Refuse changes with `why` (split sets, spanned zips).
    pub fn with_read_only(mut self, why: &'static str) -> Self {
        self.read_only = Some(why);
        self
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

    /// The reader, for writing the archive again on Save.
    pub fn reader(&self) -> &Arc<dyn Reader> {
        &self.reader
    }

    fn ov(&self) -> MutexGuard<'_, Overlay> {
        self.overlay.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Whether changes are staged and not yet saved.
    pub fn is_dirty(&self) -> bool {
        !self.ov().is_empty()
    }

    /// The staged changes (`archive.changes`).
    pub fn changes(&self) -> Vec<Change> {
        self.ov().changes(self.index())
    }

    /// Drop every staged change (Discard).
    pub fn discard(&self) {
        self.ov().clear();
    }

    /// The password change staged for Save, if any.
    pub fn password_change(&self) -> Option<crate::overlay::PasswordChange> {
        self.ov().password_change().cloned()
    }

    /// Stage setting, changing (`Some` new password) or removing (`None`) the password;
    /// zip and 7z only.
    pub fn stage_password(&self, change: crate::overlay::PasswordChange) -> Result<(), FmError> {
        self.writable()?;
        match self.format() {
            FormatId::Zip | FormatId::SevenZ => {}
            _ => {
                return Err(FmError::InvalidParams(
                    "tar has no encryption; compress to zip or 7z to protect it".into(),
                ));
            }
        }
        if change.new.as_ref().is_some_and(crate::Secret::is_empty) {
            return Err(FmError::InvalidParams("the password is empty".into()));
        }
        let info = self.info();
        let encrypted = info.encrypted || info.names_encrypted;
        let mut ov = self.ov();
        // Removing a password the archive never had (and none staged) changes nothing.
        if change.new.is_none() && !encrypted {
            ov.set_password_change(None);
            return Ok(());
        }
        ov.set_password_change(Some(change));
        Ok(())
    }

    /// `Err` with the reason when this archive cannot take changes.
    pub fn writable(&self) -> Result<(), FmError> {
        if let Some(why) = self.read_only {
            return Err(FmError::Unsupported(why));
        }
        if self.saving.load(std::sync::atomic::Ordering::Acquire) {
            return Err(FmError::Unsupported(
                "the archive is being saved; try again when it is done",
            ));
        }
        Ok(())
    }

    pub fn is_saving(&self) -> bool {
        self.saving.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Mark a save as running (`true`) or done; while it runs changes are refused, so
    /// none can slip in between writing the archive and clearing the staged set.
    /// Returns `false` when another save already runs.
    pub fn set_saving(&self, on: bool) -> bool {
        use std::sync::atomic::Ordering;
        if on {
            self.saving
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        } else {
            self.saving.store(false, Ordering::Release);
            true
        }
    }

    /// Whether the entry at `uri` is stored encrypted (the listing's lock badge).
    pub fn is_encrypted(&self, uri: &VfsUri) -> bool {
        let Ok(key) = self.key(uri) else {
            return false;
        };
        let id = match self.ov().resolve(self.index(), &key) {
            Some(Resolved::Base(id)) => id,
            Some(Resolved::Over(OvEntry::File {
                data: Data::Base(id),
                ..
            })) => id,
            _ => return false,
        };
        self.index().node(id).is_some_and(|n| n.meta.encrypted)
    }

    /// The archive path of `uri` (no leading slash), if `uri` is in this mount.
    fn key(&self, uri: &VfsUri) -> Result<String, FmError> {
        if uri.scheme() != Scheme::Archive || uri.authority() != self.root.authority() {
            return Err(FmError::NotFound(uri.to_string()));
        }
        Ok(uri.path_str().trim_start_matches('/').to_string())
    }

    fn resolve(&self, uri: &VfsUri) -> Result<Resolved, FmError> {
        let key = self.key(uri)?;
        self.ov()
            .resolve(self.index(), &key)
            .ok_or_else(|| FmError::NotFound(uri.to_string()))
    }

    fn entry_of(&self, uri: VfsUri, r: &Resolved) -> Entry {
        match r {
            Resolved::Base(id) => self.entry(uri, *id),
            Resolved::Over(OvEntry::File { size, modified, .. }) => {
                let mut e = Entry::new(uri, EntryKind::File);
                e.size = Some(*size);
                e.modified = *modified;
                e
            }
            Resolved::Over(OvEntry::Dir { modified, .. }) => {
                let mut e = Entry::new(uri, EntryKind::Dir);
                e.modified = *modified;
                e
            }
            Resolved::Over(OvEntry::Deleted) => Entry::new(uri, EntryKind::Other),
        }
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

    fn open_base(&self, id: NodeId, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
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
}

/// A new member being written into the overlay's private folder; `flush` stages it.
struct StageWriter {
    file: Option<std::fs::File>,
    path: std::path::PathBuf,
    key: String,
    size: u64,
    overlay: Arc<Mutex<Overlay>>,
    reader: Arc<dyn Reader>,
    committed: bool,
}

impl StageWriter {
    fn ov(&self) -> MutexGuard<'_, Overlay> {
        self.overlay.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Write for StageWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.ov()
            .reserve(buf.len() as u64)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::StorageFull, e.to_string()))?;
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("already saved"))?;
        file.write_all(buf)?;
        self.size += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.committed {
            return Ok(());
        }
        if let Some(mut file) = self.file.take() {
            file.flush()?;
        }
        let (path, key, size) = (self.path.clone(), self.key.clone(), self.size);
        let reader = self.reader.clone();
        self.ov()
            .put_file(reader.index(), &key, path, size)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for StageWriter {
    fn drop(&mut self) {
        // Dropping without flush is best-effort (Vfs contract): stage what was written.
        if !self.committed && self.flush().is_err() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Vfs for ArchiveFs {
    fn scheme(&self) -> Scheme {
        Scheme::Archive
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        let key = self.key(uri)?;
        let children = self
            .ov()
            .children(self.index(), &key)
            .map_err(|e| match e {
                FmError::NotFound(_) => FmError::NotFound(uri.to_string()),
                FmError::NotADirectory(_) => FmError::NotADirectory(uri.to_string()),
                other => other,
            })?;
        let mut out = Vec::with_capacity(children.len());
        for (name, r) in children {
            let Ok(child_uri) = uri.join(&name) else {
                continue;
            };
            out.push(self.entry_of(child_uri, &r));
        }
        Ok(out)
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        let r = self.resolve(uri)?;
        Ok(self.entry_of(uri.clone(), &r))
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        match self.resolve(uri)? {
            Resolved::Base(id)
            | Resolved::Over(OvEntry::File {
                data: Data::Base(id),
                ..
            }) => self.open_base(id, uri),
            Resolved::Over(OvEntry::File {
                data: Data::Spool(path),
                ..
            }) => Ok(Box::new(std::fs::File::open(path).map_err(FmError::Io)?)),
            Resolved::Over(_) => Err(FmError::NotADirectory(uri.to_string())),
        }
    }

    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        self.writable()?;
        if opts.append || !opts.truncate {
            return Err(FmError::Unsupported(
                "files inside archives can only be written whole",
            ));
        }
        let key = self.key(uri)?;
        let path = {
            let mut ov = self.ov();
            ov.can_write(self.index(), &key)?;
            if !opts.create && ov.resolve(self.index(), &key).is_none() {
                return Err(FmError::NotFound(uri.to_string()));
            }
            ov.spool_file()?
        };
        let file = std::fs::File::create(&path).map_err(FmError::Io)?;
        Ok(Box::new(StageWriter {
            file: Some(file),
            path,
            key,
            size: 0,
            overlay: self.overlay.clone(),
            reader: self.reader.clone(),
            committed: false,
        }))
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        self.writable()?;
        let key = self.key(uri)?;
        self.ov().create_dir(self.index(), &key)
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        self.writable()?;
        let key = self.key(uri)?;
        self.ov().remove(self.index(), &key)
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        self.writable()?;
        let (a, b) = (self.key(from)?, self.key(to)?);
        self.ov().rename(self.index(), &a, &b)
    }

    fn stat_full(&self, uri: &VfsUri) -> Result<Attrs, FmError> {
        let id = match self.resolve(uri)? {
            Resolved::Base(id)
            | Resolved::Over(OvEntry::File {
                data: Data::Base(id),
                ..
            }) => id,
            Resolved::Over(OvEntry::File { modified, .. } | OvEntry::Dir { modified, .. }) => {
                return Ok(Attrs {
                    mtime: modified,
                    ..Attrs::default()
                });
            }
            Resolved::Over(OvEntry::Deleted) => return Err(FmError::NotFound(uri.to_string())),
        };
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
        assert!(!fs.is_dirty());
        fs.create_dir(&root.join("new").expect("new"))
            .expect("staged");
        assert!(fs.is_dirty(), "a zip takes staged changes");
        let link = www.join("link").expect("link");
        let e = fs.stat(&link).expect("link");
        assert!(e.is_symlink() && !e.link_to_dir);
    }
}
