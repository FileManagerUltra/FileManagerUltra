//! Creating archives (`archive-extract-compress` "Compress to"): walk the selection
//! once, then write zip, tar (plain, gz, bz2, xz) or 7z, optionally encrypted.
//!
//! Links are stored as links and never followed, so a link to `/` cannot pull in the
//! whole disk. Devices, FIFOs and sockets are skipped and reported.

use std::io::{self, Read, Seek, Write};
use std::sync::Arc;
use std::time::SystemTime;

use fm_vfs::{EntryKind, FmError, Vfs, VfsUri};

use crate::format::FormatId;
use crate::limits::{MAX_DEPTH, MAX_ENTRIES};
use crate::secret::Secret;

/// Finds the backend for a location (the engine's registry, seen from a job).
pub type VfsFor = Arc<dyn Fn(&VfsUri) -> Result<Arc<dyn Vfs>, FmError> + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkKind {
    Dir,
    File,
    Symlink(String),
}

/// One item to store, with its path inside the new archive.
#[derive(Clone, Debug)]
pub struct WalkItem {
    pub uri: VfsUri,
    pub rel: String,
    pub kind: WalkKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub mode: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Walk {
    pub items: Vec<WalkItem>,
    /// Display paths of special files that were left out.
    pub skipped: Vec<String>,
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
}

/// Collect `sources` (files and folders, recursively). Each source is stored under its
/// own name at the archive root.
pub fn walk(
    sources: &[VfsUri],
    vfs_for: &VfsFor,
    cancelled: &dyn Fn() -> bool,
) -> Result<Walk, FmError> {
    let mut out = Walk::default();
    let mut stack: Vec<(VfsUri, String, usize)> = Vec::new();
    for src in sources.iter().rev() {
        let name = src
            .file_name()
            .ok_or_else(|| FmError::InvalidParams("a source has no name".into()))?;
        stack.push((src.clone(), name.to_string(), 1));
    }
    while let Some((uri, rel, depth)) = stack.pop() {
        if cancelled() {
            return Err(FmError::Cancelled);
        }
        if out.items.len() >= MAX_ENTRIES {
            return Err(FmError::backend(
                "archive",
                "too many items for one archive (limit 1 000 000)",
            ));
        }
        let vfs = vfs_for(&uri)?;
        let entry = vfs.stat(&uri)?;
        let base = WalkItem {
            uri: uri.clone(),
            rel: rel.clone(),
            kind: WalkKind::File,
            size: 0,
            modified: entry.modified,
            mode: entry.permissions,
        };
        match &entry.kind {
            EntryKind::Dir => {
                if depth > MAX_DEPTH {
                    return Err(FmError::backend(
                        "archive",
                        format!("folders nested deeper than {MAX_DEPTH} levels"),
                    ));
                }
                out.dirs += 1;
                out.items.push(WalkItem {
                    kind: WalkKind::Dir,
                    ..base
                });
                let mut children = vfs.read_dir(&uri)?;
                children.sort_by(|a, b| a.name.cmp(&b.name));
                for child in children.into_iter().rev() {
                    stack.push((child.uri, format!("{rel}/{}", child.name), depth + 1));
                }
            }
            EntryKind::File => {
                let size = entry.size.unwrap_or(0);
                out.files += 1;
                out.bytes = out.bytes.saturating_add(size);
                out.items.push(WalkItem { size, ..base });
            }
            EntryKind::Symlink { target } => out.items.push(WalkItem {
                kind: WalkKind::Symlink(target.clone()),
                ..base
            }),
            EntryKind::Other => out.skipped.push(rel),
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Fast,
    Normal,
    Best,
}

impl Level {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "fast" => Some(Self::Fast),
            "normal" => Some(Self::Normal),
            "best" => Some(Self::Best),
            _ => None,
        }
    }

    fn preset(self) -> u32 {
        match self {
            Self::Fast => 1,
            Self::Normal => 6,
            Self::Best => 9,
        }
    }
}

/// Formats Compress to… can create (tar.zst stays read-only: the pure-Rust encoder only
/// has its fastest level and still contains unimplemented paths, spike 1.4).
pub const WRITABLE: [FormatId; 6] = [
    FormatId::Zip,
    FormatId::TarGz,
    FormatId::TarXz,
    FormatId::TarBz2,
    FormatId::Tar,
    FormatId::SevenZ,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressOptions {
    pub format: FormatId,
    pub level: Level,
    pub password: Option<Secret>,
    /// zip only: weak traditional encryption instead of AES-256, for old tools. The `zip`
    /// crate cannot write it, so asking for it is refused (see design notes).
    pub zip_crypto: bool,
    /// 7z only: also encrypt the file names (the default with a password).
    pub encrypt_names: bool,
}

impl CompressOptions {
    pub fn validate(&self) -> Result<(), FmError> {
        if !WRITABLE.contains(&self.format) {
            return Err(FmError::InvalidParams(format!(
                "cannot create {} archives",
                self.format.label()
            )));
        }
        let encrypts = matches!(self.format, FormatId::Zip | FormatId::SevenZ);
        if self.password.as_ref().is_some_and(|p| !p.is_empty()) && !encrypts {
            return Err(FmError::InvalidParams(
                "tar has no encryption; choose 7z or zip".into(),
            ));
        }
        if self.zip_crypto {
            return Err(FmError::Unsupported(
                "ZipCrypto cannot be written; new zips use AES-256",
            ));
        }
        if self.password.as_ref().is_some_and(Secret::is_empty) {
            return Err(FmError::InvalidParams("the password is empty".into()));
        }
        Ok(())
    }
}

/// Progress and cancel hooks for a write.
pub struct Hooks<'a> {
    /// Bytes of file data read so far, and the item being stored.
    pub progress: &'a mut dyn FnMut(u64, &str),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

/// Write the archive for `walk` to `out` and return `out` when done.
pub fn compress<W: Write + Seek>(
    out: W,
    walk: &Walk,
    vfs_for: &VfsFor,
    opts: &CompressOptions,
    hooks: &mut Hooks<'_>,
) -> Result<W, FmError> {
    opts.validate()?;
    match opts.format {
        FormatId::Zip => write_zip(out, walk, vfs_for, opts, hooks),
        FormatId::SevenZ => write_7z(out, walk, vfs_for, opts, hooks),
        FormatId::Tar => write_tar(out, walk, vfs_for, hooks),
        FormatId::TarGz => {
            let level = flate2::Compression::new(opts.level.preset());
            let enc = flate2::write::GzEncoder::new(out, level);
            write_tar(enc, walk, vfs_for, hooks)?
                .finish()
                .map_err(FmError::from)
        }
        FormatId::TarBz2 => {
            let level = bzip2::Compression::new(opts.level.preset());
            let enc = bzip2::write::BzEncoder::new(out, level);
            write_tar(enc, walk, vfs_for, hooks)?
                .finish()
                .map_err(FmError::from)
        }
        FormatId::TarXz => {
            let enc = lzma_rust2::XzWriter::new(
                out,
                lzma_rust2::XzOptions::with_preset(opts.level.preset()),
            )?;
            write_tar(enc, walk, vfs_for, hooks)?
                .finish()
                .map_err(FmError::from)
        }
        _ => Err(FmError::Unsupported("this format cannot be written")),
    }
}

/// A file's data, counted for progress and stopped by cancel.
pub(crate) struct Counted<'a, 'b> {
    pub(crate) inner: Box<dyn Read + Send>,
    pub(crate) hooks: &'a mut Hooks<'b>,
    pub(crate) done: &'a mut u64,
    pub(crate) name: &'a str,
}

const CANCELLED: &str = "fm-archive: cancelled";

impl Read for Counted<'_, '_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if (self.hooks.cancelled)() {
            return Err(io::Error::other(CANCELLED));
        }
        let n = self.inner.read(out)?;
        *self.done = self.done.saturating_add(n as u64);
        (self.hooks.progress)(*self.done, self.name);
        Ok(n)
    }
}

pub(crate) fn io_err(e: io::Error) -> FmError {
    if e.to_string().contains(CANCELLED) {
        FmError::Cancelled
    } else {
        FmError::Io(e)
    }
}

fn open(vfs_for: &VfsFor, item: &WalkItem) -> Result<Box<dyn Read + Send>, FmError> {
    vfs_for(&item.uri)?.open_read(&item.uri)
}

pub(crate) fn zip_time(t: Option<SystemTime>) -> zip::DateTime {
    let Some(t) = t else {
        return zip::DateTime::default();
    };
    let (y, mo, d, h, mi, s) = crate::time::to_civil(t);
    u16::try_from(y)
        .ok()
        .and_then(|y| zip::DateTime::from_date_and_time(y, mo, d, h, mi, s).ok())
        .unwrap_or_default()
}

pub(crate) fn zip_err(e: zip::result::ZipError) -> FmError {
    match e {
        zip::result::ZipError::Io(io) => io_err(io),
        other => FmError::backend("archive", other.to_string()),
    }
}

fn write_zip<W: Write + Seek>(
    out: W,
    walk: &Walk,
    vfs_for: &VfsFor,
    opts: &CompressOptions,
    hooks: &mut Hooks<'_>,
) -> Result<W, FmError> {
    let mut w = zip::ZipWriter::new(out);
    let level = i64::from(opts.level.preset());
    let mut done = 0u64;
    for item in &walk.items {
        let mut o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(level))
            .last_modified_time(zip_time(item.modified))
            .large_file(item.size >= u64::from(u32::MAX));
        if let Some(mode) = item.mode {
            // Mark the entry as Unix-made: on Windows the zip crate defaults to DOS,
            // and readers then ignore the stored permission bits.
            o = o.unix_permissions(mode & 0o7777).system(zip::System::Unix);
        }
        match &item.kind {
            WalkKind::Dir => w
                .add_directory(format!("{}/", item.rel), o)
                .map_err(zip_err)?,
            WalkKind::Symlink(target) => w.add_symlink(&item.rel, target, o).map_err(zip_err)?,
            WalkKind::File => {
                let o = match &opts.password {
                    Some(pw) => o.with_aes_encryption(zip::AesMode::Aes256, pw.expose()),
                    None => o,
                };
                w.start_file(&item.rel, o).map_err(zip_err)?;
                let mut data = Counted {
                    inner: open(vfs_for, item)?,
                    hooks,
                    done: &mut done,
                    name: &item.rel,
                };
                io::copy(&mut data, &mut w).map_err(io_err)?;
            }
        }
    }
    w.finish().map_err(zip_err)
}

fn write_tar<W: Write>(
    out: W,
    walk: &Walk,
    vfs_for: &VfsFor,
    hooks: &mut Hooks<'_>,
) -> Result<W, FmError> {
    let mut b = tar::Builder::new(out);
    b.follow_symlinks(false);
    let mut done = 0u64;
    for item in &walk.items {
        let mut h = tar::Header::new_gnu();
        let mtime = item
            .modified
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        h.set_mtime(mtime);
        match &item.kind {
            WalkKind::Dir => {
                h.set_entry_type(tar::EntryType::Directory);
                h.set_mode(item.mode.unwrap_or(0o755) & 0o7777);
                h.set_size(0);
                b.append_data(&mut h, format!("{}/", item.rel), io::empty())
                    .map_err(io_err)?;
            }
            WalkKind::Symlink(target) => {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_mode(0o777);
                h.set_size(0);
                // Store the target byte for byte (path handling would rewrite `/` as `//`);
                // targets longer than the header field go through the GNU long-link entry.
                if target.len() <= 100 {
                    h.set_link_name_literal(target).map_err(io_err)?;
                    b.append_data(&mut h, &item.rel, io::empty())
                        .map_err(io_err)?;
                } else {
                    b.append_link(&mut h, &item.rel, target).map_err(io_err)?;
                }
            }
            WalkKind::File => {
                h.set_entry_type(tar::EntryType::Regular);
                h.set_mode(item.mode.unwrap_or(0o644) & 0o7777);
                h.set_size(item.size);
                let data = Counted {
                    inner: open(vfs_for, item)?,
                    hooks,
                    done: &mut done,
                    name: &item.rel,
                };
                // A file that changed size while being read must not corrupt the tar.
                let exact = Exact {
                    inner: data,
                    left: item.size,
                };
                b.append_data(&mut h, &item.rel, exact).map_err(io_err)?;
            }
        }
    }
    b.into_inner().map_err(io_err)
}

/// Exactly `left` bytes: a shorter file fails, extra bytes are cut.
struct Exact<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let room = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(out.len());
        let Some(part) = out.get_mut(..room) else {
            return Ok(0);
        };
        let n = self.inner.read(part)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "a file got shorter while it was being stored",
            ));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

fn write_7z<W: Write + Seek>(
    out: W,
    walk: &Walk,
    vfs_for: &VfsFor,
    opts: &CompressOptions,
    hooks: &mut Hooks<'_>,
) -> Result<W, FmError> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    let sz = |e: sevenz_rust2::Error| match e {
        sevenz_rust2::Error::Io(io, _) => io_err(io),
        other => FmError::backend("archive", other.to_string()),
    };
    let mut w = sevenz_rust2::ArchiveWriter::new(out).map_err(sz)?;
    let lzma2: sevenz_rust2::EncoderConfiguration =
        Lzma2Options::from_level(opts.level.preset()).into();
    match &opts.password {
        Some(pw) => {
            w.set_content_methods(vec![
                AesEncoderOptions::new(sevenz_rust2::Password::new(pw.expose())).into(),
                lzma2,
            ]);
            w.set_encrypt_header(opts.encrypt_names);
        }
        None => {
            w.set_content_methods(vec![lzma2]);
        }
    }
    let mut done = 0u64;
    for item in &walk.items {
        match &item.kind {
            // 7z has no portable link entry here: links are left out and reported.
            WalkKind::Symlink(_) => continue,
            WalkKind::Dir => {
                let mut e = sevenz_rust2::ArchiveEntry::new_directory(&item.rel);
                set_7z_time(&mut e, item.modified);
                w.push_archive_entry::<&[u8]>(e, None).map_err(sz)?;
            }
            WalkKind::File => {
                let mut e = sevenz_rust2::ArchiveEntry::new_file(&item.rel);
                e.size = item.size;
                set_7z_time(&mut e, item.modified);
                let data = Counted {
                    inner: open(vfs_for, item)?,
                    hooks,
                    done: &mut done,
                    name: &item.rel,
                };
                w.push_archive_entry(e, Some(data)).map_err(sz)?;
            }
        }
    }
    w.finish().map_err(io_err)
}

fn set_7z_time(e: &mut sevenz_rust2::ArchiveEntry, t: Option<SystemTime>) {
    if let Some(t) = t
        && let Ok(nt) = sevenz_rust2::NtTime::try_from(t)
    {
        e.has_last_modified_date = true;
        e.last_modified_date = nt;
    }
}

/// Links a 7z archive cannot hold, for the job summary.
pub fn left_out_links(walk: &Walk, format: FormatId) -> Vec<String> {
    if format != FormatId::SevenZ {
        return Vec::new();
    }
    walk.items
        .iter()
        .filter(|i| matches!(i.kind, WalkKind::Symlink(_)))
        .map(|i| i.rel.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Passwords, Progress, open as open_archive};
    use crate::source::MemSource;

    fn tree() -> (VfsFor, Vec<VfsUri>) {
        let mem = Arc::new(fm_testkit::MemFs::new());
        mem.mkdirs_str("/p/docs/empty").expect("dirs");
        mem.put_str("/p/docs/a.txt", b"alpha").expect("a");
        mem.put_str("/p/notes.txt", b"notes").expect("n");
        mem.put_symlink(&VfsUri::parse("/p/root-link").expect("u"), "/")
            .expect("link");
        let vfs: Arc<dyn Vfs> = mem;
        let vfs_for: VfsFor = Arc::new(move |_| Ok(vfs.clone()));
        let sources = ["/p/docs", "/p/notes.txt", "/p/root-link"]
            .iter()
            .map(|s| VfsUri::parse(s).expect("uri"))
            .collect();
        (vfs_for, sources)
    }

    fn roundtrip(format: FormatId, password: Option<&str>) -> Box<dyn crate::Reader> {
        let (vfs_for, sources) = tree();
        let w = walk(&sources, &vfs_for, &|| false).expect("walk");
        assert_eq!((w.files, w.dirs), (2, 2));
        let opts = CompressOptions {
            format,
            level: Level::Fast,
            password: password.map(Secret::new),
            zip_crypto: false,
            encrypt_names: true,
        };
        let mut seen = 0u64;
        let mut progress = |d: u64, _: &str| seen = d;
        let mut hooks = Hooks {
            progress: &mut progress,
            cancelled: &|| false,
        };
        let out = compress(io::Cursor::new(Vec::new()), &w, &vfs_for, &opts, &mut hooks)
            .expect("compress")
            .into_inner();
        assert_eq!(seen, 10, "{format:?}");
        let pw = Passwords::default();
        if let Some(p) = password {
            pw.add(Secret::new(p));
        }
        open_archive(
            Arc::new(MemSource::new(out)),
            format,
            "out",
            &pw,
            &Progress::none(),
        )
        .expect("reopen")
    }

    fn read(r: &dyn crate::Reader, path: &str, pw: Option<&str>) -> String {
        let id = r.index().lookup(path).expect(path);
        let p = Passwords::default();
        if let Some(pw) = pw {
            p.add(Secret::new(pw));
        }
        let mut s = String::new();
        r.open(id, &p)
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        s
    }

    #[test]
    fn every_writable_format_round_trips() {
        for format in WRITABLE {
            let r = roundtrip(format, None);
            assert_eq!(read(r.as_ref(), "docs/a.txt", None), "alpha", "{format:?}");
            assert_eq!(read(r.as_ref(), "notes.txt", None), "notes", "{format:?}");
            assert!(
                r.index().lookup("docs/empty").is_some(),
                "{format:?} empty dir"
            );
            if format != FormatId::SevenZ {
                let link = r.index().lookup("root-link").expect("link");
                assert_eq!(
                    r.index().node(link).map(|n| n.kind.clone()),
                    Some(crate::NodeKind::Symlink("/".into())),
                    "{format:?}: links are stored, not followed"
                );
            }
        }
    }

    #[test]
    fn encrypted_zip_and_7z_need_the_password() {
        let z = roundtrip(FormatId::Zip, Some("pw"));
        let id = z.index().lookup("notes.txt").expect("id");
        assert!(matches!(
            z.open(id, &Passwords::default()),
            Err(FmError::PasswordRequired(_))
        ));
        assert_eq!(read(z.as_ref(), "notes.txt", Some("pw")), "notes");
        let s = roundtrip(FormatId::SevenZ, Some("pw"));
        assert!(s.info().names_encrypted);
        assert_eq!(read(s.as_ref(), "docs/a.txt", None), "alpha");
    }

    #[test]
    fn options_are_validated_and_cancel_stops() {
        let bad = |format, password: Option<&str>| CompressOptions {
            format,
            level: Level::Normal,
            password: password.map(Secret::new),
            zip_crypto: false,
            encrypt_names: true,
        };
        assert!(bad(FormatId::TarGz, Some("x")).validate().is_err());
        assert!(bad(FormatId::Zip, Some("")).validate().is_err());
        assert!(bad(FormatId::TarZst, None).validate().is_err());
        assert!(bad(FormatId::Iso, None).validate().is_err());
        assert!(bad(FormatId::SevenZ, Some("x")).validate().is_ok());
        let (vfs_for, sources) = tree();
        assert!(matches!(
            walk(&sources, &vfs_for, &|| true),
            Err(FmError::Cancelled)
        ));
        let w = walk(&sources, &vfs_for, &|| false).expect("walk");
        let mut progress = |_: u64, _: &str| {};
        let mut hooks = Hooks {
            progress: &mut progress,
            cancelled: &|| true,
        };
        let r = compress(
            io::Cursor::new(Vec::new()),
            &w,
            &vfs_for,
            &bad(FormatId::Zip, None),
            &mut hooks,
        );
        assert!(matches!(r, Err(FmError::Cancelled)));
        assert_eq!(left_out_links(&w, FormatId::SevenZ), vec!["root-link"]);
    }
}
