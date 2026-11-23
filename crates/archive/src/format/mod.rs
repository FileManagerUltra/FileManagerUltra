//! Archive formats: recognition by name and signature, and one [`Reader`] per format.

use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use fm_vfs::FmError;

use crate::index::{Index, NodeId};
use crate::secret::Secret;
use crate::source::Source;

pub mod compress;
pub mod sevenz;
pub mod single;
pub mod tar;
pub mod zip;

/// Every archive kind this crate knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormatId {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    SevenZ,
    Iso,
    Gz,
    Bz2,
    Xz,
    Zst,
    Rar,
}

impl FormatId {
    /// Short name shown to users ("tar.gz", "7z").
    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::TarBz2 => "tar.bz2",
            Self::TarXz => "tar.xz",
            Self::TarZst => "tar.zst",
            Self::SevenZ => "7z",
            Self::Iso => "ISO",
            Self::Gz => "gzip",
            Self::Bz2 => "bzip2",
            Self::Xz => "xz",
            Self::Zst => "Zstandard",
            Self::Rar => "RAR",
        }
    }

    /// Stable id for parameters and stored references.
    pub fn id(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::TarBz2 => "tar.bz2",
            Self::TarXz => "tar.xz",
            Self::TarZst => "tar.zst",
            Self::SevenZ => "7z",
            Self::Iso => "iso",
            Self::Gz => "gz",
            Self::Bz2 => "bz2",
            Self::Xz => "xz",
            Self::Zst => "zst",
            Self::Rar => "rar",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ALL.iter().copied().find(|f| f.id() == id)
    }

    /// Whether changes inside this format can be saved (zip, the tar family, 7z).
    pub fn writable(self) -> bool {
        matches!(
            self,
            Self::Zip
                | Self::Tar
                | Self::TarGz
                | Self::TarBz2
                | Self::TarXz
                | Self::TarZst
                | Self::SevenZ
        )
    }

    /// The compression wrapped around a tar or a single file.
    pub fn compression(self) -> Option<compress::Codec> {
        match self {
            Self::TarGz | Self::Gz => Some(compress::Codec::Gzip),
            Self::TarBz2 | Self::Bz2 => Some(compress::Codec::Bzip2),
            Self::TarXz | Self::Xz => Some(compress::Codec::Xz),
            Self::TarZst | Self::Zst => Some(compress::Codec::Zstd),
            _ => None,
        }
    }
}

pub const ALL: [FormatId; 13] = [
    FormatId::Zip,
    FormatId::Tar,
    FormatId::TarGz,
    FormatId::TarBz2,
    FormatId::TarXz,
    FormatId::TarZst,
    FormatId::SevenZ,
    FormatId::Iso,
    FormatId::Gz,
    FormatId::Bz2,
    FormatId::Xz,
    FormatId::Zst,
    FormatId::Rar,
];

/// Default name patterns (suffixes, compared case-insensitively), most specific first.
pub const SUFFIXES: &[(&str, FormatId)] = &[
    (".tar.gz", FormatId::TarGz),
    (".tgz", FormatId::TarGz),
    (".tar.bz2", FormatId::TarBz2),
    (".tbz2", FormatId::TarBz2),
    (".tbz", FormatId::TarBz2),
    (".tar.xz", FormatId::TarXz),
    (".txz", FormatId::TarXz),
    (".tar.zst", FormatId::TarZst),
    (".tzst", FormatId::TarZst),
    (".tar", FormatId::Tar),
    (".zip", FormatId::Zip),
    (".jar", FormatId::Zip),
    (".7z", FormatId::SevenZ),
    (".iso", FormatId::Iso),
    (".rar", FormatId::Rar),
    (".gz", FormatId::Gz),
    (".bz2", FormatId::Bz2),
    (".xz", FormatId::Xz),
    (".zst", FormatId::Zst),
];

/// `site.tar.gz` → (`site`, `.tar.gz`); names without an archive suffix split at their last
/// dot (`notes.txt` → `notes`, `.txt`), and names without a dot keep an empty suffix.
pub fn split_suffix(name: &str) -> (String, String) {
    let lower = name.to_ascii_lowercase();
    if let Some((suffix, _)) = SUFFIXES
        .iter()
        .find(|(s, _)| lower.len() > s.len() && lower.ends_with(s))
        && let Some(stem) = name.get(..name.len() - suffix.len())
    {
        return (
            stem.to_string(),
            name.get(stem.len()..).unwrap_or("").to_string(),
        );
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    }
}

/// The format a file name suggests, by its suffix.
pub fn by_name(name: &str) -> Option<FormatId> {
    let lower = name.to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.len() > suffix.len() && lower.ends_with(suffix))
        .map(|(_, f)| *f)
}

/// The format the first bytes of a file show (`head` should hold at least 512 bytes,
/// and 32 774 for ISO). `None` when nothing is recognised.
pub fn sniff(head: &[u8]) -> Option<FormatId> {
    let starts = |magic: &[u8]| head.starts_with(magic);
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") || starts(b"PK\x07\x08") {
        return Some(FormatId::Zip);
    }
    if starts(b"7z\xBC\xAF\x27\x1C") {
        return Some(FormatId::SevenZ);
    }
    if starts(b"Rar!\x1A\x07") {
        return Some(FormatId::Rar);
    }
    if starts(&[0x1f, 0x8b]) {
        return Some(FormatId::Gz);
    }
    if starts(b"BZh") {
        return Some(FormatId::Bz2);
    }
    if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        return Some(FormatId::Xz);
    }
    if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some(FormatId::Zst);
    }
    if head.get(257..262) == Some(b"ustar") {
        return Some(FormatId::Tar);
    }
    if head.get(32_769..32_774) == Some(b"CD001") {
        return Some(FormatId::Iso);
    }
    None
}

/// Whether the signature in `head` fits the format the name suggested.
pub fn signature_fits(named: FormatId, head: &[u8]) -> bool {
    let Some(found) = sniff(head) else {
        // Old (v7) tars have no magic at all; everything else must show its signature.
        return named == FormatId::Tar;
    };
    match named {
        FormatId::TarGz => found == FormatId::Gz,
        FormatId::TarBz2 => found == FormatId::Bz2,
        FormatId::TarXz => found == FormatId::Xz,
        FormatId::TarZst => found == FormatId::Zst,
        other => found == other,
    }
}

/// What the archive preview shows (`archive-browsing` "Archive preview").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub methods: Vec<String>,
    pub encrypted: bool,
    pub names_encrypted: bool,
}

/// Passwords known for one mounted archive (at most 8), tried in order.
#[derive(Debug, Default)]
pub struct Passwords(Mutex<Vec<Secret>>);

pub const MAX_PASSWORDS: usize = 8;

impl Passwords {
    pub fn list(&self) -> Vec<Secret> {
        self.0.lock().map(|v| v.clone()).unwrap_or_default()
    }

    pub fn add(&self, secret: Secret) {
        if let Ok(mut v) = self.0.lock()
            && !v.contains(&secret)
        {
            v.insert(0, secret);
            v.truncate(MAX_PASSWORDS);
        }
    }

    pub fn forget(&self) {
        if let Ok(mut v) = self.0.lock() {
            v.clear();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().map(|v| v.is_empty()).unwrap_or(true)
    }
}

/// One opened archive: its tree and member data.
pub trait Reader: Send + Sync {
    fn format(&self) -> FormatId;
    fn index(&self) -> &Index;
    fn info(&self) -> Info;
    /// The data of a file node. Encrypted members try the known passwords and fail with
    /// `PasswordRequired` / `WrongPassword` when none fits.
    fn open(&self, id: NodeId, passwords: &Passwords) -> Result<Box<dyn Read + Send>, FmError>;
}

/// Hooks for long index builds (a compressed tar is read once from the start).
pub struct Progress<'a> {
    pub report: &'a (dyn Fn(u64, u64) + Sync),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

impl Progress<'_> {
    pub fn none() -> Progress<'static> {
        Progress {
            report: &|_, _| {},
            cancelled: &|| false,
        }
    }
}

/// Open `src` as `format`. `name` is the archive's file name, used in messages.
pub fn open(
    src: Arc<dyn Source>,
    format: FormatId,
    name: &str,
    passwords: &Passwords,
    progress: &Progress<'_>,
) -> Result<Box<dyn Reader>, FmError> {
    match format {
        FormatId::Zip => Ok(Box::new(zip::ZipReader::open(src, name)?)),
        FormatId::Tar | FormatId::TarGz | FormatId::TarBz2 | FormatId::TarXz | FormatId::TarZst => {
            Ok(Box::new(tar::TarReader::open(src, format, progress)?))
        }
        FormatId::Gz | FormatId::Bz2 | FormatId::Xz | FormatId::Zst => {
            single::open(src, format, name, progress)
        }
        FormatId::SevenZ => Ok(Box::new(sevenz::SevenZReader::open(src, name, passwords)?)),
        FormatId::Iso => Err(FmError::Unsupported("ISO images are not supported yet")),
        FormatId::Rar => Err(FmError::Unsupported("RAR archives need 7-Zip or unrar")),
    }
}

/// Run `fill` on its own thread and read what it writes, a bounded chunk at a time, so a
/// member of any size streams with little memory. Dropping the reader stops the thread
/// at its next write.
pub fn pipe<F>(fill: F) -> Box<dyn Read + Send>
where
    F: FnOnce(&mut dyn Write) -> Result<(), FmError> + Send + 'static,
{
    let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(4);
    let spawned = std::thread::Builder::new()
        .name("fm-archive-member".into())
        .spawn(move || {
            let mut w = ChunkWriter {
                tx: tx.clone(),
                buf: Vec::with_capacity(CHUNK),
            };
            // A decoder panic must end the read with an error, never as a short file.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                fill(&mut w).and_then(|()| w.flush_chunk().map_err(FmError::from))
            }))
            .unwrap_or_else(|_| Err(panicked()));
            if let Err(err) = result {
                let _ = tx.send(Err(io::Error::other(err.to_string())));
            }
        });
    match spawned {
        Ok(_) => Box::new(PipeReader {
            rx,
            cur: Vec::new(),
            pos: 0,
        }),
        Err(err) => Box::new(FailedReader(Some(err))),
    }
}

const CHUNK: usize = 256 * 1024;

struct ChunkWriter {
    tx: SyncSender<io::Result<Vec<u8>>>,
    buf: Vec<u8>,
}

impl ChunkWriter {
    fn flush_chunk(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::replace(&mut self.buf, Vec::with_capacity(CHUNK));
        self.tx
            .send(Ok(chunk))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "reader closed"))
    }
}

impl Write for ChunkWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK {
            self.flush_chunk()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct PipeReader {
    rx: Receiver<io::Result<Vec<u8>>>,
    cur: Vec<u8>,
    pos: usize,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.cur.len() {
            match self.rx.recv() {
                Ok(Ok(chunk)) => {
                    self.cur = chunk;
                    self.pos = 0;
                }
                Ok(Err(err)) => return Err(err),
                Err(_) => return Ok(0),
            }
        }
        let rest = self.cur.get(self.pos..).unwrap_or(&[]);
        let n = rest.len().min(out.len());
        if let (Some(dst), Some(src)) = (out.get_mut(..n), rest.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.pos += n;
        Ok(n)
    }
}

struct FailedReader(Option<io::Error>);

impl Read for FailedReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(self
            .0
            .take()
            .unwrap_or_else(|| io::Error::other("member reader failed")))
    }
}

/// The error a caught decoder panic becomes.
pub fn panicked() -> FmError {
    FmError::backend("archive", "the archive could not be read (decoder failure)")
}

/// Run `f`, turning a panic inside a decoder into an error (security: hostile archives).
pub fn guarded<T>(f: impl FnOnce() -> Result<T, FmError>) -> Result<T, FmError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(panicked()))
}

/// A reader whose every read is guarded against decoder panics.
pub struct CatchRead<R>(pub R);

impl<R: Read> Read for CatchRead<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.read(out)))
            .unwrap_or_else(|_| Err(io::Error::other(panicked().to_string())))
    }
}

/// Reads at most `declared` bytes and fails if the member has more: a decompression
/// bomb or a lying header never produces more than its entry said (security).
pub struct Declared<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Declared<R> {
    pub fn new(inner: R, declared: u64) -> Self {
        Self {
            inner,
            left: declared,
        }
    }
}

impl<R: Read> Read for Declared<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "member is larger than its entry says",
                )),
            };
        }
        let room = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(out.len());
        let Some(part) = out.get_mut(..room) else {
            return Ok(0);
        };
        let n = self.inner.read(part)?;
        self.left -= n as u64;
        Ok(n)
    }
}

/// Read a small member fully (sizes up to [`crate::limits::INLINE_MEMBER`]).
pub fn read_small(mut r: impl Read, declared: u64) -> Result<Vec<u8>, FmError> {
    let mut out = Vec::with_capacity(usize::try_from(declared).unwrap_or(0));
    Declared::new(&mut r, declared).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_signatures() {
        assert_eq!(by_name("Site.TAR.GZ"), Some(FormatId::TarGz));
        assert_eq!(by_name("a.tgz"), Some(FormatId::TarGz));
        assert_eq!(by_name("log.gz"), Some(FormatId::Gz));
        assert_eq!(by_name("x.7z"), Some(FormatId::SevenZ));
        assert_eq!(
            by_name(".zip"),
            None,
            "a bare suffix is a hidden file, not an archive"
        );
        assert_eq!(by_name("notes.txt"), None);
        assert_eq!(sniff(b"PK\x03\x04rest"), Some(FormatId::Zip));
        assert_eq!(
            split_suffix("Site.Tar.GZ"),
            ("Site".into(), ".Tar.GZ".into())
        );
        assert_eq!(split_suffix("notes.txt"), ("notes".into(), ".txt".into()));
        assert_eq!(split_suffix("README"), ("README".into(), String::new()));
        assert!(signature_fits(FormatId::TarGz, &[0x1f, 0x8b, 8]));
        assert!(!signature_fits(FormatId::Zip, b"7z\xBC\xAF\x27\x1C"));
        assert!(!signature_fits(FormatId::SevenZ, b"plain text"));
        for f in ALL {
            assert_eq!(FormatId::from_id(f.id()), Some(f));
        }
    }

    #[test]
    fn declared_size_stops_a_lying_member() {
        let mut out = Vec::new();
        let err = Declared::new(&b"0123456789"[..], 4)
            .read_to_end(&mut out)
            .expect_err("too long");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let mut out = Vec::new();
        Declared::new(&b"0123"[..], 4)
            .read_to_end(&mut out)
            .expect("exact");
        assert_eq!(out, b"0123");
    }

    #[test]
    fn pipe_streams_and_reports_errors() {
        let mut r = pipe(|w| {
            for i in 0..1000u32 {
                w.write_all(&i.to_le_bytes())?;
            }
            Ok(())
        });
        let mut all = Vec::new();
        r.read_to_end(&mut all).expect("read");
        assert_eq!(all.len(), 4000);
        let mut r = pipe(|w| {
            w.write_all(b"part")?;
            Err(FmError::Unsupported("broken"))
        });
        let mut all = Vec::new();
        assert!(r.read_to_end(&mut all).is_err());
        // A panicking producer is an error, not a short read.
        let mut r = pipe(|w| {
            w.write_all(b"part")?;
            panic!("decoder bug");
        });
        let mut all = Vec::new();
        assert!(r.read_to_end(&mut all).is_err());
        // Dropping early ends the producer at its next write.
        let r = pipe(|w| {
            loop {
                w.write_all(&[0u8; 4096])?;
            }
        });
        drop(r);
    }

    #[test]
    fn passwords_are_capped_and_newest_first() {
        let p = Passwords::default();
        for i in 0..10 {
            p.add(Secret::new(format!("pw{i}")));
        }
        p.add(Secret::new("pw9"));
        let list = p.list();
        assert_eq!(list.len(), MAX_PASSWORDS);
        assert_eq!(list.first().map(Secret::expose), Some("pw9"));
        p.forget();
        assert!(p.is_empty());
    }
}
