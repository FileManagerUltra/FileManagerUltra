//! tar (ustar, GNU, PAX), plain or inside gzip / bzip2 / xz / Zstandard (design D5).
//!
//! A plain tar is read in place: each member is a slice of the container. A compressed
//! tar has no index, so mounting reads it once from the start and records each member's
//! offset in the decompressed stream; reading a member decompresses up to it.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use fm_vfs::FmError;

use super::compress::{Codec, decoder};
use super::{Declared, FormatId, Info, Passwords, Progress, Reader, pipe, read_small};
use crate::index::{Index, Meta, NodeId, NodeKind, normalize};
use crate::limits::INLINE_MEMBER;
use crate::source::{SliceSource, Source, SourceReader};

pub struct TarReader {
    src: Arc<dyn Source>,
    format: FormatId,
    codec: Option<Codec>,
    index: Index,
}

impl TarReader {
    pub fn open(
        src: Arc<dyn Source>,
        format: FormatId,
        progress: &Progress<'_>,
    ) -> Result<Self, FmError> {
        let codec = format.compression();
        let index = match codec {
            None => {
                let mut archive = tar::Archive::new(SourceReader::new(src.clone()));
                let entries = archive.entries_with_seek().map_err(tar_err)?;
                build(entries, src.len(), &|| 0, progress)?
            }
            Some(codec) => {
                let read = Arc::new(AtomicU64::new(0));
                let counting = Counting {
                    inner: SourceReader::new(src.clone()),
                    read: read.clone(),
                };
                let mut archive = tar::Archive::new(decoder(codec, counting)?);
                let entries = archive.entries().map_err(tar_err)?;
                build(
                    entries,
                    src.len(),
                    &|| read.load(Ordering::Relaxed),
                    progress,
                )?
            }
        };
        Ok(Self {
            src,
            format,
            codec,
            index,
        })
    }
}

fn build<R: Read>(
    entries: tar::Entries<'_, R>,
    total: u64,
    done: &dyn Fn() -> u64,
    progress: &Progress<'_>,
) -> Result<Index, FmError> {
    let mut index = Index::new();
    // Data location of every regular file, for hard links that point at it.
    let mut data: HashMap<Vec<String>, (u64, u64, Meta)> = HashMap::new();
    for (n, entry) in entries.enumerate() {
        if n.is_multiple_of(256) {
            if (progress.cancelled)() {
                return Err(FmError::Cancelled);
            }
            (progress.report)(done(), total);
        }
        let entry = entry.map_err(tar_err)?;
        let header = entry.header();
        let raw = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let size = entry.size();
        let offset = entry.raw_file_position();
        let meta = Meta {
            size: Some(size),
            packed: None,
            modified: header
                .mtime()
                .ok()
                .and_then(|s| i64::try_from(s).ok())
                .and_then(crate::time::from_unix),
            mode: header.mode().ok().map(|m| m & 0o7777),
            uid: header.uid().ok().and_then(|u| u32::try_from(u).ok()),
            gid: header.gid().ok().and_then(|g| u32::try_from(g).ok()),
            encrypted: false,
            locator: offset,
        };
        let link = || {
            entry
                .link_name_bytes()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default()
        };
        use tar::EntryType as T;
        let kind = match header.entry_type() {
            T::Regular | T::Continuous => {
                if let Ok(parts) = normalize(&raw) {
                    data.insert(parts, (offset, size, meta.clone()));
                }
                NodeKind::File
            }
            T::Directory => NodeKind::Dir,
            T::Symlink => NodeKind::Symlink(link()),
            T::Link => {
                // A hard link reads the data of the entry it names, if that is inside.
                let target = normalize(&link()).ok().and_then(|p| data.get(&p).cloned());
                if let Some((off, len, target_meta)) = target {
                    let meta = Meta {
                        size: Some(len),
                        locator: off,
                        ..target_meta
                    };
                    index.add(&raw, NodeKind::File, meta)?;
                    continue;
                }
                NodeKind::Other
            }
            T::XGlobalHeader | T::XHeader | T::GNULongName | T::GNULongLink => continue,
            _ => NodeKind::Other,
        };
        index.add(&raw, kind, meta)?;
    }
    (progress.report)(done(), total);
    Ok(index)
}

impl Reader for TarReader {
    fn format(&self) -> FormatId {
        self.format
    }

    fn index(&self) -> &Index {
        &self.index
    }

    fn info(&self) -> Info {
        Info {
            methods: self
                .codec
                .map(|c| vec![c.label().to_string()])
                .unwrap_or_default(),
            encrypted: false,
            names_encrypted: false,
        }
    }

    fn open(&self, id: NodeId, _: &Passwords) -> Result<Box<dyn Read + Send>, FmError> {
        let node = self
            .index
            .node(id)
            .filter(|n| n.kind == NodeKind::File)
            .ok_or_else(|| FmError::NotFound("archive member".into()))?;
        let (offset, size) = (node.meta.locator, node.meta.size.unwrap_or(0));
        match self.codec {
            None => {
                let slice: Arc<dyn Source> =
                    Arc::new(SliceSource::new(self.src.clone(), offset, size));
                let reader = SourceReader::new(slice);
                if size <= INLINE_MEMBER {
                    return Ok(Box::new(io::Cursor::new(read_small(reader, size)?)));
                }
                Ok(Box::new(reader))
            }
            Some(codec) => {
                let src = self.src.clone();
                Ok(pipe(move |w| {
                    let mut dec = decoder(codec, SourceReader::new(src))?;
                    let skipped = io::copy(&mut dec.by_ref().take(offset), &mut io::sink())?;
                    if skipped < offset {
                        return Err(FmError::backend("archive", "archive ends early"));
                    }
                    let mut member = Declared::new(dec.take(size), size);
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        let n = member.read(&mut buf)?;
                        if n == 0 {
                            return Ok(());
                        }
                        w.write_all(buf.get(..n).unwrap_or(&[]))?;
                    }
                }))
            }
        }
    }
}

struct Counting<R> {
    inner: R,
    read: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(out)?;
        self.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

fn tar_err(err: io::Error) -> FmError {
    FmError::backend("archive", format!("tar: {err}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::source::MemSource;

    pub(crate) fn sample_tar() -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        let mut add = |path: &str, kind: tar::EntryType, data: &[u8], link: Option<&str>| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(kind);
            h.set_size(data.len() as u64);
            h.set_mode(0o4755);
            h.set_uid(0);
            h.set_mtime(1_700_000_000);
            if let Some(l) = link {
                h.set_link_name(l).expect("link");
            }
            // `append_data` would refuse `..`; write the raw name like a hostile tool would.
            let name = h.as_old_mut().name.as_mut();
            name.fill(0);
            for (slot, byte) in name.iter_mut().zip(path.bytes()) {
                *slot = byte;
            }
            h.set_cksum();
            b.append(&h, data).expect("append");
        };
        add("www/", tar::EntryType::Directory, b"", None);
        add(
            "www/index.html",
            tar::EntryType::Regular,
            b"<h1>tar</h1>",
            None,
        );
        add("bin/busybox", tar::EntryType::Regular, b"ELF", None);
        add("bin/sh", tar::EntryType::Link, b"", Some("bin/busybox"));
        add("bin/ls", tar::EntryType::Symlink, b"", Some("busybox"));
        add("dev/sda", tar::EntryType::Block, b"", None);
        add("../../.bashrc", tar::EntryType::Regular, b"rc", None);
        add(
            "big.bin",
            tar::EntryType::Regular,
            &vec![9u8; 2 << 20],
            None,
        );
        b.into_inner().expect("tar")
    }

    fn check(format: FormatId, bytes: Vec<u8>) {
        let r = TarReader::open(Arc::new(MemSource::new(bytes)), format, &Progress::none())
            .expect("open");
        let idx = r.index();
        let read = |path: &str| {
            let id = idx.lookup(path).expect(path);
            let mut out = Vec::new();
            r.open(id, &Passwords::default())
                .expect("open")
                .read_to_end(&mut out)
                .expect("read");
            out
        };
        assert_eq!(read("www/index.html"), b"<h1>tar</h1>");
        assert_eq!(read("bin/sh"), b"ELF", "hard link reads its target");
        assert_eq!(read(".bashrc"), b"rc");
        assert_eq!(read("big.bin").len(), 2 << 20);
        let sh = idx
            .lookup("bin/busybox")
            .and_then(|id| idx.node(id))
            .expect("node");
        assert_eq!(sh.meta.mode, Some(0o4755));
        assert_eq!(sh.meta.uid, Some(0));
        let dev = idx
            .lookup("dev/sda")
            .and_then(|id| idx.node(id))
            .expect("dev");
        assert_eq!(dev.kind, NodeKind::Other);
        let ls = idx
            .lookup("bin/ls")
            .and_then(|id| idx.node(id))
            .expect("ls");
        assert_eq!(ls.kind, NodeKind::Symlink("busybox".into()));
    }

    #[test]
    fn plain_and_compressed_tars_read_the_same() {
        use std::io::Write as _;
        let tar = sample_tar();
        check(FormatId::Tar, tar.clone());
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).expect("gz");
        check(FormatId::TarGz, gz.finish().expect("gz"));
        let zst = ruzstd::encoding::compress_to_vec(
            &tar[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        check(FormatId::TarZst, zst);
    }

    #[test]
    fn progress_and_cancel() {
        use std::io::Write as _;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&sample_tar()).expect("gz");
        let bytes = gz.finish().expect("gz");
        let cancelled = Progress {
            report: &|_, _| {},
            cancelled: &|| true,
        };
        let src: Arc<dyn Source> = Arc::new(MemSource::new(bytes));
        assert!(matches!(
            TarReader::open(src.clone(), FormatId::TarGz, &cancelled),
            Err(FmError::Cancelled)
        ));
        let seen = std::sync::Mutex::new(Vec::new());
        let report = |d: u64, t: u64| seen.lock().expect("lock").push((d, t));
        let p = Progress {
            report: &report,
            cancelled: &|| false,
        };
        TarReader::open(src, FormatId::TarGz, &p).expect("open");
        let seen = seen.lock().expect("lock");
        let last = seen.last().expect("reported");
        assert!(last.0 > 0 && last.0 <= last.1);
    }
}
