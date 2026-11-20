//! The sparse record index (design D4): the offset of every [`STRIDE`]th record, built
//! in one sequential pass that can be cancelled and reports progress in batches.

use std::sync::atomic::{AtomicBool, Ordering};

use fm_vfs::FmError;

use super::Dialect;
use super::scan::{Fields, MAX_FIELDS};
use super::source::{ByteSource, Reader};

/// Records between two checkpoints.
pub const STRIDE: u64 = 32;
/// Read-ahead of the indexer for block sources.
pub const CHUNK: usize = 4 << 20;
/// Records between two progress reports.
const BATCH_RECORDS: u64 = 1 << 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    /// `checkpoints[k]` is the offset of record `k * STRIDE`.
    pub checkpoints: Vec<u64>,
    /// Records indexed so far.
    pub rows: u64,
    /// Most fields seen in one record (capped at [`MAX_FIELDS`]).
    pub max_fields: usize,
    pub malformed: u64,
    /// Data bytes scanned so far.
    pub scanned: u64,
    pub done: bool,
}

impl Index {
    /// Where row `row` can be read from: the checkpoint offset and how many records to
    /// skip after it. `None` until the row is indexed.
    pub fn locate(&self, row: u64) -> Option<(u64, u64)> {
        if row >= self.rows {
            return None;
        }
        let at = self.checkpoints.get((row / STRIDE) as usize)?;
        Some((*at, row % STRIDE))
    }

    /// Fold a progress report in.
    pub fn apply(&mut self, delta: IndexDelta) {
        self.checkpoints.extend(delta.checkpoints);
        self.rows = delta.rows;
        self.max_fields = self.max_fields.max(delta.max_fields);
        self.malformed = delta.malformed;
        self.scanned = delta.scanned;
        self.done = delta.done;
    }
}

/// New checkpoints and totals since the previous report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexDelta {
    pub checkpoints: Vec<u64>,
    pub rows: u64,
    pub max_fields: usize,
    pub malformed: u64,
    pub scanned: u64,
    pub done: bool,
}

/// Index `src` from `start` (the first data record). `publish` receives batches; the
/// last one has `done` set. A set `cancel` ends the pass early without a final report.
pub fn build(
    src: &dyn ByteSource,
    d: Dialect,
    start: u64,
    cancel: &AtomicBool,
    mut publish: impl FnMut(IndexDelta),
) -> Result<(), FmError> {
    let mut reader = Reader::new(src, d, start, CHUNK);
    let mut delta = IndexDelta::default();
    if start < src.len() {
        delta.checkpoints.push(start);
    }
    let mut since = 0u64;
    while let Some(found) = reader.next(None::<&mut Fields>)? {
        delta.rows += 1;
        delta.max_fields = delta.max_fields.max(found.fields.min(MAX_FIELDS));
        delta.malformed += u64::from(found.malformed);
        if delta.rows % STRIDE == 0 && found.end < src.len() {
            delta.checkpoints.push(found.end);
        }
        since += 1;
        if since >= BATCH_RECORDS {
            since = 0;
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            delta.scanned = found.end;
            let report = IndexDelta {
                checkpoints: std::mem::take(&mut delta.checkpoints),
                ..delta.clone()
            };
            publish(report);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    delta.scanned = src.len();
    delta.done = true;
    publish(delta);
    Ok(())
}

/// Read up to `count` records from `offset` after skipping `skip`, with their offsets.
pub fn read_rows(
    src: &dyn ByteSource,
    d: Dialect,
    offset: u64,
    skip: u64,
    count: usize,
) -> Result<Vec<(u64, super::Row)>, FmError> {
    let mut reader = Reader::new(src, d, offset, 256 << 10);
    for _ in 0..skip {
        if reader.next(None)?.is_none() {
            return Ok(Vec::new());
        }
    }
    let mut out = Vec::with_capacity(count.min(4096));
    let mut fields = Fields::default();
    while out.len() < count {
        let Some(found) = reader.next(Some(&mut fields))? else {
            break;
        };
        out.push((found.start, super::Row::from_fields(&fields)));
    }
    Ok(out)
}
