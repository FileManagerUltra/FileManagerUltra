//! In-process SFTP server over a directory, for tests (feature `test-server`, unix only).
//!
//! Real `russh` server + `russh-sftp` server handler on `127.0.0.1:0`: password and key
//! auth, a fixed host key, and a [`Faults`] block to inject stalls, hostile listings and
//! dropped connections.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, FileMode, Handle, Name, OpenFlags, Status, StatusCode,
    Version,
};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use crate::rt;

/// Fault injection, shared with the running server. All off by default.
#[derive(Default)]
pub struct Faults {
    /// Delay every `read` reply by this many milliseconds.
    pub read_delay_ms: AtomicU64,
    /// Report this many extra bogus entries (plus one name containing `/` and one entry
    /// with a `u64::MAX` size) in every directory listing.
    pub hostile_entries: AtomicUsize,
    /// Refuse every file open with "permission denied".
    pub deny_open: AtomicBool,
    kill: Notify,
}

impl Faults {
    /// Drop every live connection (the sockets close without a goodbye).
    pub fn kill_connections(&self) {
        self.kill.notify_waiters();
    }
}

/// Deterministic keys: the host key and the one authorised client key.
pub fn host_key() -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[7u8; 32]))
}

pub fn client_key() -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[9u8; 32]))
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub user: String,
    pub password: String,
    pub faults: Arc<Faults>,
    pub authorized_key: PublicKey,
    pub host_public: PublicKey,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Serve `root` as `/` for `user` / `password` (or the fixed [`client_key`]).
    pub fn start(root: &Path, user: &str, password: &str) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        let faults = Arc::new(Faults::default());
        let host = host_key();
        let host_public = host.public_key().clone();
        let authorized_key = client_key().public_key().clone();
        let config = Arc::new(russh::server::Config {
            keys: vec![host],
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::from_millis(0)),
            ..Default::default()
        });
        let ctx = Arc::new(Ctx {
            root,
            user: user.to_string(),
            password: password.to_string(),
            authorized: authorized_key.clone(),
            faults: faults.clone(),
        });
        let handle = rt::handle().map_err(|e| std::io::Error::other(e.to_string()))?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let task = handle.spawn(async move {
            let Ok(listener) = TcpListener::from_std(listener) else {
                return;
            };
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let config = config.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let kill = ctx.faults.kill.notified();
                    tokio::pin!(kill);
                    // Registered before the handshake so a kill during it still lands.
                    kill.as_mut().enable();
                    let handler = SshSession::new(ctx.clone());
                    let Ok(running) = russh::server::run_stream(config, socket, handler).await
                    else {
                        return;
                    };
                    let session = running.handle();
                    tokio::select! {
                        _ = running => {}
                        _ = &mut kill => {
                            let _ = session
                                .disconnect(
                                    russh::Disconnect::ByApplication,
                                    String::new(),
                                    "en".to_string(),
                                )
                                .await;
                        }
                    }
                });
            }
        });
        Ok(Self {
            addr,
            user: user.to_string(),
            password: password.to_string(),
            faults,
            authorized_key,
            host_public,
            task,
        })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
        self.faults.kill_connections();
    }
}

struct Ctx {
    root: PathBuf,
    user: String,
    password: String,
    authorized: PublicKey,
    faults: Arc<Faults>,
}

struct SshSession {
    ctx: Arc<Ctx>,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl SshSession {
    fn new(ctx: Arc<Ctx>) -> Self {
        Self {
            ctx,
            channels: HashMap::new(),
        }
    }
}

impl russh::server::Handler for SshSession {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if user == self.ctx.user && password == self.ctx.password {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: russh::server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if user == self.ctx.user && *key.public_key == self.ctx.authorized {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp"
            && let Some(channel) = self.channels.remove(&channel_id)
        {
            session.channel_success(channel_id)?;
            let sftp = Sftp::new(self.ctx.clone());
            tokio::spawn(russh_sftp::server::run(channel.into_stream(), sftp));
        } else {
            session.channel_failure(channel_id)?;
        }
        Ok(())
    }
}

enum Open {
    File(fs::File),
    Dir { path: PathBuf, done: bool },
}

struct Sftp {
    ctx: Arc<Ctx>,
    handles: HashMap<String, Open>,
    next: u64,
}

impl Sftp {
    fn new(ctx: Arc<Ctx>) -> Self {
        Self {
            ctx,
            handles: HashMap::new(),
            next: 0,
        }
    }

    /// `/a/b` → `<root>/a/b`; a path that climbs out of the root is refused.
    fn map(&self, path: &str) -> Result<PathBuf, StatusCode> {
        let mut out = self.ctx.root.clone();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(StatusCode::PermissionDenied),
                p => out.push(p),
            }
        }
        Ok(out)
    }

    fn new_handle(&mut self, open: Open) -> String {
        self.next += 1;
        let h = format!("h{}", self.next);
        self.handles.insert(h.clone(), open);
        h
    }
}

fn code(e: &std::io::Error) -> StatusCode {
    match e.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn attrs_of(m: &fs::Metadata) -> FileAttributes {
    FileAttributes {
        size: Some(m.len()),
        uid: Some(m.uid()),
        gid: Some(m.gid()),
        permissions: Some(m.mode()),
        atime: Some(u32::try_from(m.atime()).unwrap_or(0)),
        mtime: Some(u32::try_from(m.mtime()).unwrap_or(0)),
        ..Default::default()
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en".into(),
    }
}

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if self.ctx.faults.deny_open.load(Ordering::Relaxed) {
            return Err(StatusCode::PermissionDenied);
        }
        let path = self.map(&filename)?;
        let mut o = fs::OpenOptions::new();
        o.read(pflags.contains(OpenFlags::READ))
            .write(pflags.contains(OpenFlags::WRITE))
            .append(pflags.contains(OpenFlags::APPEND))
            .create(pflags.contains(OpenFlags::CREATE))
            .truncate(pflags.contains(OpenFlags::TRUNCATE));
        let f = o.open(path).map_err(|e| code(&e))?;
        Ok(Handle {
            id,
            handle: self.new_handle(Open::File(f)),
        })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let delay = self.ctx.faults.read_delay_ms.load(Ordering::Relaxed);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let Some(Open::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        f.seek(SeekFrom::Start(offset)).map_err(|e| code(&e))?;
        let mut buf = vec![0u8; len.min(65536) as usize];
        let n = f.read(&mut buf).map_err(|e| code(&e))?;
        if n == 0 {
            return Err(StatusCode::Eof);
        }
        buf.truncate(n);
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let Some(Open::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        f.seek(SeekFrom::Start(offset)).map_err(|e| code(&e))?;
        f.write_all(&data).map_err(|e| code(&e))?;
        Ok(ok(id))
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let m = fs::symlink_metadata(self.map(&path)?).map_err(|e| code(&e))?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&m),
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let m = fs::metadata(self.map(&path)?).map_err(|e| code(&e))?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&m),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let Some(Open::File(f)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure);
        };
        let m = f.metadata().map_err(|e| code(&e))?;
        Ok(Attrs {
            id,
            attrs: attrs_of(&m),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        #[cfg(unix)]
        if let Some(mode) = attrs.permissions {
            use std::os::unix::fs::PermissionsExt;
            let p = self.map(&path)?;
            fs::set_permissions(&p, fs::Permissions::from_mode(mode & 0o7777))
                .map_err(|e| code(&e))?;
        }
        #[cfg(not(unix))]
        let _ = (path, attrs);
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let p = self.map(&path)?;
        if !p.is_dir() {
            return Err(if p.exists() {
                StatusCode::Failure
            } else {
                StatusCode::NoSuchFile
            });
        }
        fs::read_dir(&p).map_err(|e| code(&e))?;
        Ok(Handle {
            id,
            handle: self.new_handle(Open::Dir {
                path: p,
                done: false,
            }),
        })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let hostile = self.ctx.faults.hostile_entries.load(Ordering::Relaxed);
        let Some(Open::Dir { path, done }) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        if *done {
            return Err(StatusCode::Eof);
        }
        *done = true;
        let mut files = Vec::new();
        for item in fs::read_dir(&*path).map_err(|e| code(&e))? {
            let item = item.map_err(|e| code(&e))?;
            let m = fs::symlink_metadata(item.path()).map_err(|e| code(&e))?;
            files.push(File::new(
                item.file_name().to_string_lossy().into_owned(),
                attrs_of(&m),
            ));
        }
        if hostile > 0 {
            let plain = FileAttributes {
                permissions: Some(FileMode::REG.bits() | 0o644),
                size: Some(1),
                ..Default::default()
            };
            for i in 0..hostile {
                files.push(File::new(format!("bogus-{i}"), plain.clone()));
            }
            files.push(File::new("evil/name", plain.clone()));
            files.push(File::new(
                "huge",
                FileAttributes {
                    size: Some(u64::MAX),
                    ..plain
                },
            ));
        }
        Ok(Name { id, files })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        fs::remove_file(self.map(&filename)?).map_err(|e| code(&e))?;
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        fs::create_dir(self.map(&path)?).map_err(|e| code(&e))?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        fs::remove_dir(self.map(&path)?).map_err(|e| code(&e))?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let mut parts: Vec<&str> = Vec::new();
        for p in path.split('/') {
            match p {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        let abs = format!("/{}", parts.join("/"));
        Ok(Name {
            id,
            files: vec![File::dummy(abs)],
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        fs::rename(self.map(&oldpath)?, self.map(&newpath)?).map_err(|e| code(&e))?;
        Ok(ok(id))
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let t = fs::read_link(self.map(&path)?).map_err(|e| code(&e))?;
        Ok(Name {
            id,
            files: vec![File::dummy(t.to_string_lossy().into_owned())],
        })
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        // OpenSSH sends (target, link) in the opposite order to the draft; the client
        // crate follows OpenSSH, so the first field here is the link location.
        symlink(&targetpath, self.map(&linkpath)?).map_err(|e| code(&e))?;
        Ok(ok(id))
    }
}
