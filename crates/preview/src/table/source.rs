//! Where table bytes come from (design D3) and a record reader over them.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::{Arc, Mutex};

use fm_vfs::{FmError, Vfs, VfsUri};

use super::Dialect;
use super::scan::{Fields, MAX_HARD, RecordEnd, scan_record};

/// Random access to the (UTF-8) bytes of a table.
pub trait ByteSource: Send + Sync {
    fn len(&self) -> u64;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Up to `len` bytes from `offset`; fewer only at the end of the data.
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, FmError>;

    /// The whole data, when it is in memory.
    fn as_slice(&self) -> Option<&[u8]> {
        None
    }
}

/// A table held in memory.
#[derive(Debug, Clone)]
pub struct MemSource(pub Arc<[u8]>);

impl ByteSource for MemSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, FmError> {
        let start = (offset as usize).min(self.0.len());
        let end = start.saturating_add(len).min(self.0.len());
        Ok(self.0.get(start..end).unwrap_or(&[]).to_vec())
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.0)
    }
}

/// Block size of [`VfsSource`].
pub const BLOCK: usize = 1 << 20;
/// Blocks kept by [`VfsSource`].
pub const BLOCKS_KEPT: usize = 16;

/// Cached blocks and their use order (oldest first).
type Blocks = (HashMap<u64, Arc<Vec<u8>>>, VecDeque<u64>);

/// A large file read in blocks through its filesystem, with a small LRU.
pub struct VfsSource {
    vfs: Arc<dyn Vfs>,
    uri: VfsUri,
    len: u64,
    blocks: Mutex<Blocks>,
}

impl VfsSource {
    pub fn new(vfs: Arc<dyn Vfs>, uri: VfsUri, len: u64) -> Self {
        Self {
            vfs,
            uri,
            len,
            blocks: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    fn block(&self, n: u64) -> Result<Arc<Vec<u8>>, FmError> {
        if let Ok(g) = self.blocks.lock()
            && let Some(b) = g.0.get(&n)
        {
            return Ok(b.clone());
        }
        let offset = n * BLOCK as u64;
        let want = (self.len.saturating_sub(offset)).min(BLOCK as u64);
        let mut buf = Vec::with_capacity(want as usize);
        self.vfs
            .open_read_at(&self.uri, offset)?
            .take(want)
            .read_to_end(&mut buf)?;
        let b = Arc::new(buf);
        if let Ok(mut g) = self.blocks.lock() {
            let (map, order) = &mut *g;
            map.insert(n, b.clone());
            order.retain(|k| *k != n);
            order.push_back(n);
            while order.len() > BLOCKS_KEPT {
                if let Some(old) = order.pop_front() {
                    map.remove(&old);
                }
            }
        }
        Ok(b)
    }
}

impl ByteSource for VfsSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, FmError> {
        let end = offset.saturating_add(len as u64).min(self.len);
        let mut out = Vec::with_capacity(end.saturating_sub(offset) as usize);
        let mut at = offset;
        while at < end {
            let n = at / BLOCK as u64;
            let block = self.block(n)?;
            let within = (at - n * BLOCK as u64) as usize;
            let take = ((end - at) as usize).min(block.len().saturating_sub(within));
            if take == 0 {
                break;
            }
            out.extend_from_slice(block.get(within..within + take).unwrap_or(&[]));
            at += take as u64;
        }
        Ok(out)
    }
}

/// One record found by a [`Reader`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    /// Offset of the record in the data.
    pub start: u64,
    /// Offset where the next record starts.
    pub end: u64,
    pub malformed: bool,
    pub fields: usize,
}

/// Reads records one after another from an offset of a source.
pub struct Reader<'a> {
    src: &'a dyn ByteSource,
    d: Dialect,
    /// Read-ahead for sources that are not in memory.
    buf: Vec<u8>,
    /// Data offset of `buf[0]` (or of the slice start for memory sources).
    base: u64,
    pos: usize,
    eof: bool,
    chunk: usize,
}

impl<'a> Reader<'a> {
    pub fn new(src: &'a dyn ByteSource, d: Dialect, offset: u64, chunk: usize) -> Self {
        let in_memory = src.as_slice().is_some();
        Self {
            src,
            d,
            buf: Vec::new(),
            base: if in_memory { 0 } else { offset },
            pos: if in_memory { offset as usize } else { 0 },
            eof: in_memory,
            chunk: chunk.max(4096),
        }
    }

    /// Offset of the next record.
    pub fn offset(&self) -> u64 {
        self.base + self.pos as u64
    }

    /// The next record, with its fields in `sink` when given; `None` at the end.
    pub fn next(&mut self, sink: Option<&mut Fields>) -> Result<Option<Found>, FmError> {
        let mut sink = sink;
        loop {
            let data: &[u8] = match self.src.as_slice() {
                Some(s) => s,
                None => &self.buf,
            };
            if self.pos >= data.len() && self.eof {
                return Ok(None);
            }
            if let Some(RecordEnd {
                next,
                malformed,
                fields,
            }) = scan_record(data, self.pos, &self.d, self.eof, sink.as_deref_mut())
            {
                let found = Found {
                    start: self.base + self.pos as u64,
                    end: self.base + next as u64,
                    malformed,
                    fields,
                };
                self.pos = next;
                return Ok(Some(found));
            }
            self.refill()?;
        }
    }

    fn refill(&mut self) -> Result<(), FmError> {
        // Drop what was consumed, then read on; the scanner needs up to MAX_HARD bytes
        // past a record start, so the chunk grows until it can decide.
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.base += self.pos as u64;
            self.pos = 0;
        }
        let want = if self.buf.len() >= self.chunk {
            (self.buf.len()).min(MAX_HARD)
        } else {
            self.chunk
        };
        let at = self.base + self.buf.len() as u64;
        let more = self.src.read_at(at, want)?;
        if more.is_empty() {
            self.eof = true;
        } else {
            if at + (more.len() as u64) >= self.src.len() {
                self.eof = true;
            }
            self.buf.extend_from_slice(&more);
        }
        Ok(())
    }
}
