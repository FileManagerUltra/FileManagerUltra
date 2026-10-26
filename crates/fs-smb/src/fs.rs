//! [`SmbFs`]: the blocking [`Vfs`] over one live SMB session and one share (tree).
//!
//! Every `smb` type stays inside this module; the rest of the workspace sees only
//! `fm-vfs` types.

use std::io::{self, Read, Write};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use fm_vfs::{
    Authority, Entry, EntryKind, FmError, MAX_COMPONENT_BYTES, MAX_DIR_ENTRIES, Scheme, SpaceInfo,
    Vfs, VfsUri, WriteOpts,
};
use secrecy::{ExposeSecret, SecretString};
use smb::binrw_util::sized_string::SizedWideString;
use smb::{
    Client, CreateDisposition, CreateOptions, DirAccessMask, FileAccessMask, FileAttributes,
    FileCreateArgs, FileDispositionInformation, FileFsFullSizeInformation,
    FileFullDirectoryInformation, FileNetworkOpenInformation, FileRenameInformation, Resource,
    UncPath,
};

use crate::errors::map_err;

/// Reads and writes move in chunks this size; a reported file size is never trusted
/// for an allocation.
const CHUNK: usize = 1024 * 1024;
/// Seconds between 1601-01-01 (FILETIME) and 1970-01-01.
const FILETIME_UNIX_OFFSET_SECS: u64 = 11_644_473_600;

pub(crate) struct Inner {
    pub(crate) client: Client,
    /// `host` or `host:port`, as used in UNC paths.
    pub(crate) server: String,
    pub(crate) authority: Authority,
    pub(crate) account: String,
    pub(crate) password: SecretString,
    /// Shares with a live tree connection, in connection order.
    pub(crate) shares: Mutex<Vec<String>>,
    pub(crate) max_entries: usize,
    pub(crate) dead: AtomicBool,
    /// The pipes share (`IPC$`, used to list shares) is connected.
    pub(crate) ipc: AtomicBool,
    pub(crate) encrypted: bool,
}

/// One session to one server, serving every share the user opens. Cheap to clone; all
/// clones share the session. `smb://host/` lists the shares connected so far.
#[derive(Clone)]
pub struct SmbFs {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for SmbFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbFs")
            .field("authority", &self.inner.authority.to_string())
            .finish()
    }
}

struct Loc {
    share: String,
    rel: String,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn valid_share(share: &str) -> bool {
    !share.is_empty()
        && share.len() <= 80
        && !share
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        && share != "."
        && share != ".."
}

fn close_res(res: Resource) {
    match res {
        Resource::File(f) => {
            let _ = f.close();
        }
        Resource::Directory(d) => {
            let _ = d.close();
        }
        Resource::Pipe(_) => {}
    }
}

fn filetime_to_system(ft: u64) -> Option<std::time::SystemTime> {
    let secs = (ft / 10_000_000).checked_sub(FILETIME_UNIX_OFFSET_SECS)?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

fn valid_name(name: &str) -> bool {
    !(name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(|c| c.is_control())
        || name.len() > MAX_COMPONENT_BYTES)
}

fn read_access() -> FileAccessMask {
    FileAccessMask::new()
        .with_file_read_attributes(true)
        .with_synchronize(true)
}

impl SmbFs {
    pub fn authority(&self) -> &Authority {
        &self.inner.authority
    }

    /// Names of the shares connected so far.
    pub fn shares(&self) -> Vec<String> {
        lock(&self.inner.shares).clone()
    }

    /// The share root (`smb://host/share`).
    pub fn start_uri(&self) -> Result<VfsUri, FmError> {
        let first = lock(&self.inner.shares).first().cloned();
        VfsUri::parse(&format!(
            "smb://{}/{}",
            self.inner.authority,
            first.unwrap_or_default()
        ))
    }

    /// Whether the session negotiated encryption (otherwise it is signed).
    pub fn is_encrypted(&self) -> bool {
        self.inner.encrypted
    }

    /// `false` once a transport error was seen.
    pub fn is_alive(&self) -> bool {
        !self.inner.dead.load(Ordering::Relaxed)
    }

    /// Close the session. Safe to call more than once.
    pub fn disconnect(&self) {
        self.inner.dead.store(true, Ordering::Relaxed);
        let _ = self.inner.client.close();
    }

    /// Test hook: fail listings past `max` entries instead of the production cap.
    #[must_use]
    pub fn with_max_entries(self, max: usize) -> Self {
        match Arc::try_unwrap(self.inner) {
            Ok(mut inner) => {
                inner.max_entries = max;
                Self {
                    inner: Arc::new(inner),
                }
            }
            Err(shared) => Self { inner: shared },
        }
    }

    fn note_io(&self, e: &io::Error) {
        if e.kind() == io::ErrorKind::ConnectionAborted {
            self.inner.dead.store(true, Ordering::Relaxed);
        }
    }

    fn need(&self, uri: &VfsUri) -> Result<Loc, FmError> {
        self.locate(uri)?.ok_or_else(|| {
            FmError::backend(
                "smb",
                format!(
                    "open a share first, e.g. smb://{}/<share>",
                    self.inner.authority
                ),
            )
        })
    }

    fn mark<T>(&self, r: Result<T, FmError>) -> Result<T, FmError> {
        if let Err(FmError::Io(e)) = &r
            && e.kind() == io::ErrorKind::ConnectionAborted
        {
            self.inner.dead.store(true, Ordering::Relaxed);
        }
        r
    }

    /// `None` for the server root (`smb://host/`); otherwise the share and the path inside
    /// it, with `\` separators and no leading slash.
    fn locate(&self, uri: &VfsUri) -> Result<Option<Loc>, FmError> {
        if uri.scheme() != Scheme::Smb || *uri.authority() != self.inner.authority {
            return Err(FmError::backend(
                "smb",
                format!(
                    "this connection serves {}, not {}",
                    self.inner.authority,
                    uri.authority()
                ),
            ));
        }
        let Some(share) = uri.share() else {
            return Ok(None);
        };
        let rest = uri
            .path_str()
            .strip_prefix('/')
            .and_then(|p| p.get(share.len()..))
            .unwrap_or_default()
            .trim_start_matches('/');
        if rest.split('/').any(|c| c == ".." || c == ".") {
            return Err(FmError::invalid_uri("relative component in smb path"));
        }
        Ok(Some(Loc {
            share: share.to_string(),
            rel: rest.replace('/', "\\"),
        }))
    }

    /// Tree-connect `share` with the session's credentials if that has not happened yet.
    fn ensure_share(&self, share: &str) -> Result<(), FmError> {
        if !valid_share(share) {
            return Err(FmError::backend("smb", "invalid share name"));
        }
        let mut shares = lock(&self.inner.shares);
        if shares.iter().any(|s| s.eq_ignore_ascii_case(share)) {
            return Ok(());
        }
        let unc = UncPath::from_str(&format!("\\\\{}\\{share}", self.inner.server))
            .map_err(|e| map_err(&e, share))?;
        let r = self
            .inner
            .client
            .share_connect(
                &unc,
                &self.inner.account,
                self.inner.password.expose_secret().to_string(),
            )
            .map_err(|e| map_err(&e, share));
        self.mark(r)?;
        shares.push(share.to_string());
        Ok(())
    }

    fn unc(&self, loc: &Loc) -> Result<UncPath, FmError> {
        let root = UncPath::from_str(&format!("\\\\{}\\{}", self.inner.server, loc.share))
            .map_err(|e| map_err(&e, &loc.share))?;
        Ok(if loc.rel.is_empty() {
            root
        } else {
            root.with_path(&loc.rel)
        })
    }

    fn open(&self, loc: &Loc, args: &FileCreateArgs, display: &str) -> Result<Resource, FmError> {
        self.ensure_share(&loc.share)?;
        let r = self
            .inner
            .client
            .create_file(&self.unc(loc)?, args)
            .map_err(|e| map_err(&e, display));
        self.mark(r)
    }

    /// Names of the server's disk shares (no `$` admin shares), asked of the server.
    fn server_shares(&self) -> Result<Vec<String>, FmError> {
        if !self.inner.ipc.load(Ordering::Relaxed) {
            let r = self
                .inner
                .client
                .ipc_connect(
                    &self.inner.server,
                    &self.inner.account,
                    self.inner.password.expose_secret().to_string(),
                )
                .map_err(|e| map_err(&e, &self.inner.server));
            self.mark(r)?;
            self.inner.ipc.store(true, Ordering::Relaxed);
        }
        let pipe = self
            .inner
            .client
            .open_pipe(&self.inner.server, "srvsvc")
            .map_err(|e| map_err(&e, &self.inner.server));
        let pipe = self.mark(pipe)?;
        let listed = crate::shares::list(&pipe, &self.inner.server);
        let _ = pipe.close();
        let mut names: Vec<String> = listed?
            .into_iter()
            .filter(|n| valid_share(n) && !n.ends_with('$'))
            .take(self.inner.max_entries)
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    /// The virtual server root: the server's shares as folders. When the server will not
    /// enumerate them (some NAS firmwares), the shares connected so far stand in.
    fn root_entries(&self, uri: &VfsUri) -> Vec<Entry> {
        let names = self
            .server_shares()
            .unwrap_or_else(|_| lock(&self.inner.shares).clone());
        names
            .iter()
            .filter_map(|name| {
                let child = uri.join(name).ok()?;
                Some(Entry {
                    uri: child,
                    name: name.clone(),
                    kind: EntryKind::Dir,
                    size: None,
                    modified: None,
                    permissions: None,
                    hidden: false,
                    link_to_dir: false,
                })
            })
            .collect()
    }

    fn open_dir(&self, loc: &Loc, display: &str) -> Result<smb::Directory, FmError> {
        let mut options = CreateOptions::new();
        options.set_directory_file(true);
        let args = FileCreateArgs {
            disposition: CreateDisposition::Open,
            attributes: FileAttributes::new(),
            options,
            desired_access: DirAccessMask::new()
                .with_list_directory(true)
                .with_synchronize(true)
                .into(),
        };
        match self.open(loc, &args, display)? {
            Resource::Directory(d) => Ok(d),
            other => {
                close_res(other);
                Err(FmError::NotADirectory(display.to_string()))
            }
        }
    }

    fn list(
        &self,
        uri: &VfsUri,
        dir: &smb::Directory,
        display: &str,
    ) -> Result<Vec<Entry>, FmError> {
        let iter = smb::Directory::query::<FileFullDirectoryInformation>(dir, "*")
            .map_err(|e| map_err(&e, display));
        let iter = self.mark(iter)?;
        let mut out = Vec::new();
        for item in iter {
            let info = match item {
                Ok(i) => i,
                Err(e) => return self.mark(Err(map_err(&e, display))),
            };
            let name = info.file_name.to_string();
            if !valid_name(&name) {
                continue;
            }
            let Ok(child) = uri.join(&name) else {
                continue;
            };
            out.push(self.entry_from(
                child,
                name,
                info.file_attributes,
                info.end_of_file,
                *info.last_write_time,
            ));
            if out.len() > self.inner.max_entries.min(MAX_DIR_ENTRIES) {
                return Err(FmError::backend(
                    "smb",
                    format!("directory too large (> {} entries)", self.inner.max_entries),
                ));
            }
        }
        Ok(out)
    }

    fn entry_from(
        &self,
        uri: VfsUri,
        name: String,
        attrs: FileAttributes,
        size: u64,
        ft: u64,
    ) -> Entry {
        let kind = if attrs.reparse_point() {
            EntryKind::Symlink {
                target: String::new(),
            }
        } else if attrs.directory() {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        let is_file = matches!(kind, EntryKind::File);
        Entry {
            uri,
            name,
            size: is_file.then_some(size),
            modified: filetime_to_system(ft),
            permissions: None,
            hidden: attrs.hidden(),
            link_to_dir: attrs.reparse_point() && attrs.directory(),
            kind,
        }
    }
}

impl Vfs for SmbFs {
    fn scheme(&self) -> Scheme {
        Scheme::Smb
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        let Some(loc) = self.locate(uri)? else {
            return Ok(self.root_entries(uri));
        };
        let display = uri.path_str();
        // A listing blocks other queries on its handle, so each listing opens its own.
        let dir = self.open_dir(&loc, display)?;
        let result = self.list(uri, &dir, display);
        let _ = dir.close();
        result
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        let Some(loc) = self.locate(uri)? else {
            return Ok(Entry {
                uri: uri.clone(),
                name: self.inner.authority.host.clone(),
                kind: EntryKind::Dir,
                size: None,
                modified: None,
                permissions: None,
                hidden: false,
                link_to_dir: false,
            });
        };
        let display = uri.path_str();
        let args = FileCreateArgs::make_open_existing(read_access());
        let res = self.open(&loc, &args, display)?;
        let info = match &res {
            Resource::File(f) => f.query_info::<FileNetworkOpenInformation>(),
            Resource::Directory(d) => d.query_info::<FileNetworkOpenInformation>(),
            Resource::Pipe(_) => {
                return Err(FmError::Unsupported("pipes are not browsable"));
            }
        }
        .map_err(|e| map_err(&e, display));
        close_res(res);
        let info = self.mark(info)?;
        let name = uri
            .file_name()
            .map_or_else(|| loc.share.clone(), str::to_string);
        Ok(self.entry_from(
            uri.clone(),
            name,
            info.file_attributes,
            info.end_of_file,
            *info.last_write_time,
        ))
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        self.open_read_at(uri, 0)
    }

    fn open_read_at(&self, uri: &VfsUri, offset: u64) -> Result<Box<dyn Read + Send>, FmError> {
        let loc = self.need(uri)?;
        let display = uri.path_str();
        let mut options = CreateOptions::new();
        options.set_non_directory_file(true);
        let args = FileCreateArgs {
            disposition: CreateDisposition::Open,
            attributes: FileAttributes::new(),
            options,
            desired_access: FileAccessMask::new()
                .with_generic_read(true)
                .with_synchronize(true),
        };
        match self.open(&loc, &args, display)? {
            Resource::File(file) => Ok(Box::new(SmbReader {
                file,
                pos: offset,
                fs: self.clone(),
            })),
            other => {
                close_res(other);
                Err(FmError::backend("smb", format!("{display} is a directory")))
            }
        }
    }

    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        let loc = self.need(uri)?;
        let display = uri.path_str();
        let disposition = match (opts.create, opts.truncate && !opts.append) {
            (true, true) => CreateDisposition::OverwriteIf,
            (true, false) => CreateDisposition::OpenIf,
            (false, true) => CreateDisposition::Overwrite,
            (false, false) => CreateDisposition::Open,
        };
        let mut options = CreateOptions::new();
        options.set_non_directory_file(true);
        let args = FileCreateArgs {
            disposition,
            attributes: FileAttributes::new(),
            options,
            desired_access: FileAccessMask::new()
                .with_generic_write(true)
                .with_file_read_attributes(true)
                .with_synchronize(true),
        };
        let file = match self.open(&loc, &args, display)? {
            Resource::File(f) => f,
            other => {
                close_res(other);
                return Err(FmError::backend("smb", format!("{display} is a directory")));
            }
        };
        let pos = if opts.append {
            let info = file
                .query_info::<FileNetworkOpenInformation>()
                .map_err(|e| map_err(&e, display));
            match self.mark(info) {
                Ok(i) => i.end_of_file,
                Err(e) => {
                    let _ = file.close();
                    return Err(e);
                }
            }
        } else {
            0
        };
        Ok(Box::new(SmbWriter {
            file: Some(file),
            pos,
            fs: self.clone(),
        }))
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        let loc = self.need(uri)?;
        if loc.rel.is_empty() {
            return Err(FmError::AlreadyExists(uri.path_str().to_string()));
        }
        let mut options = CreateOptions::new();
        options.set_directory_file(true);
        let args =
            FileCreateArgs::make_create_new(FileAttributes::new().with_directory(true), options);
        match self.open(&loc, &args, uri.path_str())? {
            Resource::Directory(d) => {
                let _ = d.close();
            }
            Resource::File(f) => {
                let _ = f.close();
            }
            Resource::Pipe(_) => {}
        }
        Ok(())
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        let loc = self.need(uri)?;
        let display = uri.path_str();
        if loc.rel.is_empty() {
            return Err(FmError::PermissionDenied(display.to_string()));
        }
        let args = FileCreateArgs::make_open_existing(
            FileAccessMask::new()
                .with_delete(true)
                .with_file_read_attributes(true)
                .with_synchronize(true),
        );
        let res = self.open(&loc, &args, display)?;
        let (is_dir, result) = match &res {
            Resource::File(f) => (false, f.set_info(FileDispositionInformation::default())),
            Resource::Directory(d) => (true, d.set_info(FileDispositionInformation::default())),
            Resource::Pipe(_) => return Err(FmError::Unsupported("pipes are not browsable")),
        };
        let result = result.map_err(|e| map_err(&e, display));
        let result = self.mark(result);
        let closed = match res {
            Resource::File(f) => f.close(),
            Resource::Directory(d) => d.close(),
            Resource::Pipe(_) => Ok(()),
        }
        .map_err(|e| map_err(&e, display));
        let result = result.and(closed);
        match result {
            Err(FmError::Backend { .. }) if is_dir => Err(FmError::NotEmpty(display.to_string())),
            other => other,
        }
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        let a = self.need(from)?;
        let b = self.need(to)?;
        let display = from.path_str();
        if a.rel.is_empty() || b.rel.is_empty() {
            return Err(FmError::PermissionDenied(display.to_string()));
        }
        if !a.share.eq_ignore_ascii_case(&b.share) {
            return Err(FmError::Unsupported("cannot rename across shares"));
        }
        let args = FileCreateArgs::make_open_existing(
            FileAccessMask::new()
                .with_delete(true)
                .with_file_read_attributes(true)
                .with_synchronize(true),
        );
        let res = self.open(&a, &args, display)?;
        let info = FileRenameInformation {
            replace_if_exists: false.into(),
            root_directory: 0,
            file_name: SizedWideString::from(b.rel.as_str()),
        };
        let result = match &res {
            Resource::File(f) => f.set_info(info),
            Resource::Directory(d) => d.set_info(info),
            Resource::Pipe(_) => return Err(FmError::Unsupported("pipes are not browsable")),
        }
        .map_err(|e| map_err(&e, to.path_str()));
        match res {
            Resource::File(f) => {
                let _ = f.close();
            }
            Resource::Directory(d) => {
                let _ = d.close();
            }
            Resource::Pipe(_) => {}
        }
        self.mark(result)
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        a.scheme() == Scheme::Smb
            && b.scheme() == Scheme::Smb
            && a.authority() == b.authority()
            && *a.authority() == self.inner.authority
            && match (a.share(), b.share()) {
                (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
                _ => false,
            }
    }

    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        let Some(loc) = self.locate(uri)? else {
            return Ok(None);
        };
        let display = uri.path_str();
        let dir = self.open_dir(&loc, display)?;
        let info = dir
            .query_fs_info::<FileFsFullSizeInformation>()
            .map_err(|e| map_err(&e, display));
        let _ = dir.close();
        let info = self.mark(info)?;
        let unit = u64::from(info.sectors_per_allocation_unit)
            .saturating_mul(u64::from(info.bytes_per_sector));
        Ok(Some(SpaceInfo {
            total: info.total_allocation_units.saturating_mul(unit),
            available: info.caller_available_allocation_units.saturating_mul(unit),
        }))
    }
}

struct SmbReader {
    file: smb::File,
    pos: u64,
    fs: SmbFs,
}

impl Read for SmbReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(CHUNK);
        let Some(buf) = buf.get_mut(..n) else {
            return Ok(0);
        };
        let got = self
            .file
            .read_block(buf, self.pos, None, false)
            .inspect_err(|e| self.fs.note_io(e))?;
        self.pos = self.pos.saturating_add(got as u64);
        Ok(got)
    }
}

impl Drop for SmbReader {
    fn drop(&mut self) {
        // The `smb` crate does not close handles on drop in blocking mode.
        let _ = self.file.close();
    }
}

struct SmbWriter {
    file: Option<smb::File>,
    pos: u64,
    fs: SmbFs,
}

impl Write for SmbWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(file) = self.file.as_ref() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "file already closed",
            ));
        };
        let n = buf.len().min(CHUNK);
        let Some(buf) = buf.get(..n) else {
            return Ok(0);
        };
        if buf.is_empty() {
            return Ok(0);
        }
        let wrote = file
            .write_block(buf, self.pos, None)
            .inspect_err(|e| self.fs.note_io(e))?;
        self.pos = self.pos.saturating_add(wrote as u64);
        Ok(wrote)
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(file) = self.file.as_ref() else {
            return Ok(());
        };
        file.flush().inspect_err(|e| self.fs.note_io(e))
    }
}

impl Drop for SmbWriter {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.close();
        }
    }
}
