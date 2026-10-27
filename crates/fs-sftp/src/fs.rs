//! [`SftpFs`]: the blocking [`Vfs`] over one live SFTP connection.

use std::io::{self, Read, SeekFrom, Write};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use fm_vfs::{
    Authority, Entry, EntryKind, FmError, MAX_COMPONENT_BYTES, MAX_DIR_ENTRIES, Scheme, SpaceInfo,
    Vfs, VfsUri, WriteOpts,
};
use russh::client;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::fs::File as RemoteFile;
use russh_sftp::client::{RawSftpSession, SftpSession};
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::connect::Handler;
use crate::rt::{self, block_on_timeout, transport_error};

/// Reads and writes move in chunks this size; a reported file size is never trusted
/// for an allocation.
const CHUNK: usize = 32 * 1024;
/// Symlinks resolved (target + dir-ness) per listing; the rest show without a target.
const MAX_LINK_LOOKUPS: usize = 512;

struct Inner {
    handle: client::Handle<Handler>,
    sftp: SftpSession,
    /// A second channel used only for listings, so entry counts can be capped as the
    /// packets arrive instead of after the library has buffered everything.
    dir: RawSftpSession,
    authority: Authority,
    home: String,
    max_entries: usize,
}

/// One connection to one server. Cheap to clone; all clones share the session.
#[derive(Clone)]
pub struct SftpFs {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SftpFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpFs")
            .field("authority", &self.inner.authority.to_string())
            .finish()
    }
}

impl SftpFs {
    pub(crate) fn from_parts(
        handle: client::Handle<Handler>,
        sftp: SftpSession,
        dir: RawSftpSession,
        authority: Authority,
        home: String,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                handle,
                sftp,
                dir,
                authority,
                home,
                max_entries: MAX_DIR_ENTRIES,
            }),
        }
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

    pub fn authority(&self) -> &Authority {
        &self.inner.authority
    }

    /// The directory the connection opened in (absolute).
    pub fn start_uri(&self) -> Result<VfsUri, FmError> {
        self.uri_for(&self.inner.home)
    }

    /// `false` once the link has dropped.
    pub fn is_alive(&self) -> bool {
        !self.inner.handle.is_closed()
    }

    /// Close the session. Safe to call more than once.
    pub fn disconnect(&self) {
        let inner = self.inner.clone();
        let _ = block_on_timeout(async move {
            let _ = inner.sftp.close().await;
            let _ = inner
                .handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        });
    }

    fn uri_for(&self, path: &str) -> Result<VfsUri, FmError> {
        VfsUri::parse(&format!("sftp://{}{}", self.inner.authority, path))
    }

    fn path<'a>(&self, uri: &'a VfsUri) -> Result<&'a str, FmError> {
        if uri.scheme() != Scheme::Sftp || *uri.authority() != self.inner.authority {
            return Err(FmError::backend(
                "sftp",
                format!(
                    "this connection serves {}, not {}",
                    self.inner.authority,
                    uri.authority()
                ),
            ));
        }
        Ok(uri.path_str())
    }

    fn entry(
        &self,
        uri: VfsUri,
        name: String,
        attrs: &FileAttributes,
        link: Option<(String, bool)>,
    ) -> Entry {
        let kind = if attrs.is_dir() {
            EntryKind::Dir
        } else if attrs.is_symlink() {
            EntryKind::Symlink {
                target: link.as_ref().map(|l| l.0.clone()).unwrap_or_default(),
            }
        } else if attrs.is_regular() {
            EntryKind::File
        } else if attrs.permissions.is_none() && !attrs.is_symlink() {
            // A server that reports no mode at all: treat as a plain file.
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let is_file = matches!(kind, EntryKind::File);
        Entry {
            uri,
            name,
            size: if is_file { attrs.size } else { None },
            modified: attrs
                .mtime
                .map(|s| UNIX_EPOCH + Duration::from_secs(u64::from(s))),
            permissions: attrs.permissions.map(|m| m & 0o7777),
            hidden: false,
            link_to_dir: matches!(kind, EntryKind::Symlink { .. })
                && link.as_ref().is_some_and(|l| l.1),
            kind,
        }
    }
}

fn io_to_fm(e: io::Error, path: &str) -> FmError {
    use io::ErrorKind as K;
    match e.kind() {
        K::NotFound => FmError::NotFound(path.to_string()),
        K::PermissionDenied => FmError::PermissionDenied(path.to_string()),
        K::AlreadyExists => FmError::AlreadyExists(path.to_string()),
        K::ConnectionAborted
        | K::BrokenPipe
        | K::UnexpectedEof
        | K::TimedOut
        | K::ConnectionReset => transport_error("the connection to the server was lost"),
        _ => FmError::backend("sftp", format!("{path}: {e}")),
    }
}

/// Map a client error to `FmError`; transport failures become `Io(ConnectionAborted)`.
fn map_err(e: SftpError, path: &str) -> FmError {
    match e {
        SftpError::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => FmError::NotFound(path.to_string()),
            StatusCode::PermissionDenied => FmError::PermissionDenied(path.to_string()),
            StatusCode::NoConnection | StatusCode::ConnectionLost => {
                transport_error("the connection to the server was lost")
            }
            StatusCode::OpUnsupported => {
                FmError::Unsupported("the server does not support this operation")
            }
            _ => FmError::backend("sftp", format!("{path}: {}", s.error_message)),
        },
        SftpError::Timeout => transport_error("the server stopped responding (timeout)"),
        SftpError::IO(_) => transport_error("the connection to the server was lost"),
        SftpError::UnexpectedBehavior(m) if is_closed_msg(&m) => {
            transport_error("the connection to the server was lost")
        }
        other => FmError::backend("sftp", format!("{path}: {other}")),
    }
}

fn is_closed_msg(m: &str) -> bool {
    m.contains("session closed") || m.contains("SendError") || m.contains("RecvError")
}

/// Errors surfaced by `AsyncRead`/`AsyncWrite` on a remote file: keep the kind, but
/// recognise the client library's "link is gone" messages as a lost connection.
fn lift_io(e: io::Error) -> io::Error {
    if is_closed_msg(&e.to_string()) {
        io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "the connection to the server was lost",
        )
    } else {
        e
    }
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

fn fm_to_io(e: FmError) -> io::Error {
    match e {
        FmError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    }
}

impl Vfs for SftpFs {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        let path = self.path(uri)?;
        let inner = self.inner.clone();
        let this = self.clone();
        let dir = uri.clone();
        let p = path.to_string();
        block_on_timeout(async move {
            let handle = inner
                .dir
                .opendir(p.clone())
                .await
                .map_err(|e| map_err(e, &p))?
                .handle;
            let mut out = Vec::new();
            let mut link_lookups = 0usize;
            let result: Result<(), FmError> = async {
                loop {
                    let batch = match inner.dir.readdir(handle.clone()).await {
                        Ok(b) => b,
                        Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                        Err(e) => return Err(map_err(e, &p)),
                    };
                    for file in batch.files {
                        if !valid_name(&file.filename) {
                            continue;
                        }
                        let Ok(child) = dir.join(&file.filename) else {
                            continue;
                        };
                        let link = if file.attrs.is_symlink() && link_lookups < MAX_LINK_LOOKUPS {
                            link_lookups += 1;
                            Some(resolve_link(&inner.sftp, child.path_str()).await)
                        } else {
                            None
                        };
                        out.push(this.entry(child, file.filename, &file.attrs, link));
                        if out.len() > inner.max_entries {
                            return Err(FmError::backend(
                                "sftp",
                                format!("directory too large (> {} entries)", inner.max_entries),
                            ));
                        }
                    }
                }
                Ok(())
            }
            .await;
            let _ = inner.dir.close(handle).await;
            result.map(|()| out)
        })?
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        let path = self.path(uri)?.to_string();
        let inner = self.inner.clone();
        let this = self.clone();
        let uri = uri.clone();
        block_on_timeout(async move {
            let attrs = inner
                .sftp
                .symlink_metadata(path.clone())
                .await
                .map_err(|e| map_err(e, &path))?;
            let link = if attrs.is_symlink() {
                Some(resolve_link(&inner.sftp, &path).await)
            } else {
                None
            };
            let name = uri
                .file_name()
                .map(str::to_string)
                .unwrap_or_else(|| path.clone());
            Ok(this.entry(uri, name, &attrs, link))
        })?
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        self.open_read_at(uri, 0)
    }

    fn open_read_at(&self, uri: &VfsUri, offset: u64) -> Result<Box<dyn Read + Send>, FmError> {
        let path = self.path(uri)?.to_string();
        let inner = self.inner.clone();
        let file = block_on_timeout(async move {
            let mut f = inner
                .sftp
                .open(path.clone())
                .await
                .map_err(|e| map_err(e, &path))?;
            if offset > 0 {
                f.seek(SeekFrom::Start(offset))
                    .await
                    .map_err(|e| io_to_fm(e, &path))?;
            }
            Ok::<_, FmError>(f)
        })??;
        Ok(Box::new(RemoteReader { file }))
    }

    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        let path = self.path(uri)?.to_string();
        let mut flags = OpenFlags::WRITE;
        if opts.create {
            flags |= OpenFlags::CREATE;
        }
        if opts.truncate && !opts.append {
            flags |= OpenFlags::TRUNCATE;
        }
        if opts.append {
            flags |= OpenFlags::APPEND;
        }
        let inner = self.inner.clone();
        let file = block_on_timeout(async move {
            inner
                .sftp
                .open_with_flags(path.clone(), flags)
                .await
                .map_err(|e| map_err(e, &path))
        })??;
        Ok(Box::new(RemoteWriter { file: Some(file) }))
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        let path = self.path(uri)?.to_string();
        let inner = self.inner.clone();
        block_on_timeout(async move {
            match inner.sftp.create_dir(path.clone()).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Servers answer "Failure" for an existing directory.
                    if inner.sftp.try_exists(path.clone()).await.unwrap_or(false) {
                        Err(FmError::AlreadyExists(path))
                    } else {
                        Err(map_err(e, &path))
                    }
                }
            }
        })?
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        let path = self.path(uri)?.to_string();
        let inner = self.inner.clone();
        block_on_timeout(async move {
            let attrs = inner
                .sftp
                .symlink_metadata(path.clone())
                .await
                .map_err(|e| map_err(e, &path))?;
            if attrs.is_dir() {
                inner.sftp.remove_dir(path.clone()).await.map_err(|e| {
                    let err = map_err(e, &path);
                    match err {
                        FmError::Backend { .. } => FmError::NotEmpty(path.clone()),
                        other => other,
                    }
                })
            } else {
                inner
                    .sftp
                    .remove_file(path.clone())
                    .await
                    .map_err(|e| map_err(e, &path))
            }
        })?
    }

    fn create_symlink(&self, target: &str, link: &VfsUri) -> Result<(), FmError> {
        let path = self.path(link)?.to_string();
        let target = target.to_string();
        let inner = self.inner.clone();
        block_on_timeout(async move {
            // russh-sftp's argument order matches OpenSSH's (reversed) SYMLINK packet.
            inner
                .sftp
                .symlink(path.clone(), target)
                .await
                .map_err(|e| map_err(e, &path))
        })?
    }

    fn set_permissions(&self, uri: &VfsUri, mode: u32) -> Result<(), FmError> {
        let path = self.path(uri)?.to_string();
        let inner = self.inner.clone();
        block_on_timeout(async move {
            let attrs = FileAttributes {
                permissions: Some(mode & 0o7777),
                ..FileAttributes::empty()
            };
            inner
                .sftp
                .set_metadata(path.clone(), attrs)
                .await
                .map_err(|e| map_err(e, &path))
        })?
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        let a = self.path(from)?.to_string();
        let b = self.path(to)?.to_string();
        let inner = self.inner.clone();
        block_on_timeout(async move {
            if inner.sftp.try_exists(b.clone()).await.unwrap_or(false) {
                return Err(FmError::AlreadyExists(b));
            }
            inner
                .sftp
                .rename(a.clone(), b)
                .await
                .map_err(|e| map_err(e, &a))
        })?
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        a.scheme() == Scheme::Sftp
            && b.scheme() == Scheme::Sftp
            && a.authority() == b.authority()
            && *a.authority() == self.inner.authority
    }

    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        let path = self.path(uri)?.to_string();
        let inner = self.inner.clone();
        block_on_timeout(async move {
            match inner.sftp.fs_info(path.clone()).await {
                Ok(Some(s)) => {
                    let unit = s.fragment_size.max(1);
                    Ok(Some(SpaceInfo {
                        total: s.blocks.saturating_mul(unit),
                        available: s.blocks_avail.saturating_mul(unit),
                    }))
                }
                Ok(None) => Ok(None),
                Err(SftpError::Status(s)) if s.status_code == StatusCode::OpUnsupported => Ok(None),
                Err(e) => Err(map_err(e, &path)),
            }
        })?
    }
}

/// `(target, resolves_to_dir)`; failures degrade to an empty target, never an error.
async fn resolve_link(sftp: &SftpSession, path: &str) -> (String, bool) {
    let target = sftp.read_link(path.to_string()).await.unwrap_or_default();
    let is_dir = sftp
        .metadata(path.to_string())
        .await
        .is_ok_and(|m| m.is_dir());
    (target, is_dir)
}

struct RemoteReader {
    file: RemoteFile,
}

impl Read for RemoteReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(CHUNK);
        let file = &mut self.file;
        match block_on_timeout(async { file.read(&mut buf[..n]).await }) {
            Ok(r) => r.map_err(lift_io),
            Err(e) => Err(fm_to_io(e)),
        }
    }
}

struct RemoteWriter {
    file: Option<RemoteFile>,
}

impl Write for RemoteWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(file) = self.file.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "file already closed",
            ));
        };
        if buf.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(CHUNK);
        match block_on_timeout(async { file.write(&buf[..n]).await }) {
            Ok(r) => r.map_err(lift_io),
            Err(e) => Err(fm_to_io(e)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        match block_on_timeout(async { file.flush().await }) {
            Ok(r) => r.map_err(lift_io),
            Err(e) => Err(fm_to_io(e)),
        }
    }
}

impl Drop for RemoteWriter {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Waits for pending writes; skipped (the file closes itself) if we are
            // somehow on a runtime thread, where blocking would panic.
            let _ = rt::block_on_timeout(async move { file.close().await });
        }
    }
}
