//! Random-access sources: the archive file (through any `Vfs`), in-memory bytes for
//! tests and nested reads, and a `Read + Seek` adapter the format crates consume.

use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use fm_vfs::{FmError, Vfs, VfsUri};

use crate::limits::{BLOCK, CACHED_BLOCKS};

/// Positional, thread-safe reads of one container.
pub trait Source: Send + Sync {
    /// Total length in bytes.
    fn len(&self) -> u64;

    /// Read into `buf` from `offset`; returns 0 at or past the end.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FmError>;

    /// `true` when the source has no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Bytes held in memory (tests, small nested archives, spooled data).
pub struct MemSource(Arc<[u8]>);

impl MemSource {
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }
}

impl Source for MemSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FmError> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let Some(rest) = self.0.get(start..) else {
            return Ok(0);
        };
        let n = rest.len().min(buf.len());
        if let (Some(dst), Some(src)) = (buf.get_mut(..n), rest.get(..n)) {
            dst.copy_from_slice(src);
        }
        Ok(n)
    }
}

/// A private local file (an unpacked copy in the temporary folder), read positionally.
pub struct FileSource {
    file: Mutex<std::fs::File>,
    len: u64,
}

impl FileSource {
    pub fn open(path: &std::path::Path) -> Result<Self, FmError> {
        let file = std::fs::File::open(path).map_err(FmError::Io)?;
        let len = file.metadata().map_err(FmError::Io)?.len();
        Ok(Self {
            file: Mutex::new(file),
            len,
        })
    }
}

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FmError> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        if offset >= self.len {
            return Ok(0);
        }
        let mut file = self
            .file
            .lock()
            .map_err(|_| FmError::backend("archive", "temporary copy poisoned"))?;
        file.seek(SeekFrom::Start(offset)).map_err(FmError::Io)?;
        file.read(buf).map_err(FmError::Io)
    }
}

/// The archive file on any backend, read in [`BLOCK`]-sized pieces with a small LRU, so
/// listing a zip on SFTP reads its central directory, not the whole file.
pub struct VfsSource {
    vfs: Arc<dyn Vfs>,
    uri: VfsUri,
    len: u64,
    modified: Option<SystemTime>,
    cache: Mutex<VecDeque<(u64, Arc<[u8]>)>>,
}

impl VfsSource {
    /// Stat `uri` and wrap it; fails if it is not a readable file.
    pub fn open(vfs: Arc<dyn Vfs>, uri: VfsUri) -> Result<Self, FmError> {
        let entry = vfs.stat(&uri)?;
        if !entry.is_file() {
            return Err(FmError::NotADirectory(uri.to_string()));
        }
        Ok(Self {
            len: entry.size.unwrap_or(0),
            modified: entry.modified,
            vfs,
            uri,
            cache: Mutex::new(VecDeque::new()),
        })
    }

    pub fn uri(&self) -> &VfsUri {
        &self.uri
    }

    pub fn modified(&self) -> Option<SystemTime> {
        self.modified
    }

    fn block(&self, index: u64) -> Result<Arc<[u8]>, FmError> {
        if let Ok(mut cache) = self.cache.lock()
            && let Some(pos) = cache.iter().position(|(i, _)| *i == index)
            && let Some(hit) = cache.remove(pos)
        {
            let data = hit.1.clone();
            cache.push_front(hit);
            return Ok(data);
        }
        let start = index.saturating_mul(BLOCK);
        let want = BLOCK.min(self.len.saturating_sub(start));
        let mut reader = self.vfs.open_read_at(&self.uri, start)?;
        let mut data = Vec::with_capacity(usize::try_from(want).unwrap_or(0));
        reader.by_ref().take(want).read_to_end(&mut data)?;
        let data: Arc<[u8]> = data.into();
        if let Ok(mut cache) = self.cache.lock() {
            cache.push_front((index, data.clone()));
            cache.truncate(CACHED_BLOCKS);
        }
        Ok(data)
    }
}

impl Source for VfsSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FmError> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let block = self.block(offset / BLOCK)?;
        let within = usize::try_from(offset % BLOCK).unwrap_or(usize::MAX);
        let Some(rest) = block.get(within..) else {
            return Ok(0);
        };
        let n = rest.len().min(buf.len());
        if let (Some(dst), Some(src)) = (buf.get_mut(..n), rest.get(..n)) {
            dst.copy_from_slice(src);
        }
        Ok(n)
    }
}

/// A view of `inner` from `offset` for `len` bytes (partitions, items found inside blobs).
pub struct SliceSource {
    inner: Arc<dyn Source>,
    offset: u64,
    len: u64,
}

impl SliceSource {
    pub fn new(inner: Arc<dyn Source>, offset: u64, len: u64) -> Self {
        let len = len.min(inner.len().saturating_sub(offset));
        Self { inner, offset, len }
    }
}

impl Source for SliceSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FmError> {
        if offset >= self.len {
            return Ok(0);
        }
        let room = usize::try_from(self.len - offset).unwrap_or(usize::MAX);
        let take = room.min(buf.len());
        let Some(part) = buf.get_mut(..take) else {
            return Ok(0);
        };
        self.inner.read_at(self.offset.saturating_add(offset), part)
    }
}

/// Several sources read as one (numbered parts of a split archive, design D16).
pub struct ConcatSource {
    parts: Vec<(u64, Arc<dyn Source>)>,
    len: u64,
}

impl ConcatSource {
    pub fn new(parts: Vec<Arc<dyn Source>>) -> Self {
        let mut at = 0u64;
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            let len = part.len();
            out.push((at, part));
            at = at.saturating_add(len);
        }
        Self {
            parts: out,
            len: at,
        }
    }
}

impl Source for ConcatSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, FmError> {
        let idx = self.parts.partition_point(|(start, _)| *start <= offset);
        let Some((start, part)) = idx.checked_sub(1).and_then(|i| self.parts.get(i)) else {
            return Ok(0);
        };
        part.read_at(offset - start, buf)
    }
}

/// `Read + Seek` over a [`Source`]; cheap to clone (each clone has its own position).
#[derive(Clone)]
pub struct SourceReader {
    src: Arc<dyn Source>,
    pos: u64,
}

impl SourceReader {
    pub fn new(src: Arc<dyn Source>) -> Self {
        Self { src, pos: 0 }
    }
}

impl Read for SourceReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.src.read_at(self.pos, buf).map_err(io::Error::other)?;
        self.pos = self.pos.saturating_add(n as u64);
        Ok(n)
    }
}

impl Seek for SourceReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let next = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.src.len().checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let next = next.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek"))?;
        self.pos = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(src: &dyn Source) -> Vec<u8> {
        let mut out = vec![0u8; usize::try_from(src.len()).expect("len")];
        let mut at = 0usize;
        while at < out.len() {
            let n = src
                .read_at(at as u64, out.get_mut(at..).expect("rest"))
                .expect("read");
            assert!(n > 0);
            at += n;
        }
        out
    }

    #[test]
    fn concat_and_slice_read_across_boundaries() {
        let a: Arc<dyn Source> = Arc::new(MemSource::new(b"hello ".to_vec()));
        let b: Arc<dyn Source> = Arc::new(MemSource::new(b"split ".to_vec()));
        let c: Arc<dyn Source> = Arc::new(MemSource::new(b"world".to_vec()));
        let all: Arc<dyn Source> = Arc::new(ConcatSource::new(vec![a, b, c]));
        assert_eq!(read_all(all.as_ref()), b"hello split world");
        let slice = SliceSource::new(all.clone(), 6, 5);
        assert_eq!(read_all(&slice), b"split");
        let mut r = SourceReader::new(all);
        r.seek(SeekFrom::End(-5)).expect("seek");
        let mut s = String::new();
        r.read_to_string(&mut s).expect("read");
        assert_eq!(s, "world");
        assert!(r.seek(SeekFrom::Current(-100)).is_err());
    }

    #[test]
    fn vfs_source_reads_blocks_from_a_backend() {
        let fs = Arc::new(fm_testkit::MemFs::new());
        let uri = VfsUri::parse("/x.bin").expect("uri");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        fs.put_file(&uri, &data).expect("put");
        let src = VfsSource::open(fs, uri).expect("open");
        assert_eq!(src.len(), data.len() as u64);
        let mut buf = [0u8; 10];
        let n = src.read_at(BLOCK - 3, &mut buf).expect("read");
        assert_eq!(
            &buf[..n],
            &data[(BLOCK - 3) as usize..(BLOCK - 3) as usize + n]
        );
        assert_eq!(read_all(&src), data);
    }
}
