//! The quote-aware record scanner (design D1). One function, [`scan_record`], decides
//! where a record ends and optionally collects its fields, so the indexer, the row reader
//! and the view jobs always agree on record boundaries.

use memchr::{memchr, memchr2};

use super::Dialect;

/// A quoted field longer than this is malformed: the rest of its record runs to the next
/// line break as if unquoted.
pub const MAX_RECORD: usize = 1 << 20;
/// A record with no line break within this many bytes ends here.
pub const MAX_HARD: usize = 4 << 20;
/// Fields kept per record; further fields are scanned but dropped.
pub const MAX_FIELDS: usize = 16_384;
/// Bytes kept per field for display; longer values are cut (and marked).
pub const MAX_CELL: usize = 64 << 10;

/// Separator between fields in [`Fields::data`]; a needle without it cannot match
/// across two fields.
const SEP: u8 = 0;

/// The fields of one record, in one reusable buffer.
#[derive(Debug, Default, Clone)]
pub struct Fields {
    /// Field bytes, each followed by [`SEP`].
    pub data: Vec<u8>,
    /// End of each field in `data` (exclusive, before its separator).
    ends: Vec<usize>,
    /// Fields cut at [`MAX_CELL`].
    pub cut: Vec<usize>,
    /// Fields beyond [`MAX_FIELDS`] that were dropped.
    pub dropped: usize,
}

impl Fields {
    pub fn clear(&mut self) {
        self.data.clear();
        self.ends.clear();
        self.cut.clear();
        self.dropped = 0;
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Field `i`, or empty when the record has fewer fields.
    pub fn get(&self, i: usize) -> &[u8] {
        let Some(&end) = self.ends.get(i) else {
            return &[];
        };
        let start = match i.checked_sub(1) {
            Some(p) => self.ends.get(p).map_or(0, |e| e + 1),
            None => 0,
        };
        self.data.get(start..end).unwrap_or(&[])
    }

    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        (0..self.len()).map(|i| self.get(i))
    }

    /// Every field joined by the separator byte (for "any column" text search).
    pub fn joined(&self) -> &[u8] {
        &self.data
    }

    fn begin_field(&mut self) -> bool {
        if self.ends.len() >= MAX_FIELDS {
            self.dropped += 1;
            return false;
        }
        true
    }

    fn push_bytes(&mut self, start: usize, bytes: &[u8]) {
        let room = MAX_CELL.saturating_sub(self.data.len() - start);
        if bytes.len() > room {
            let take = floor_char_boundary(bytes, room);
            self.data
                .extend_from_slice(bytes.get(..take).unwrap_or(&[]));
            let field = self.ends.len();
            if self.cut.last() != Some(&field) {
                self.cut.push(field);
            }
        } else {
            self.data.extend_from_slice(bytes);
        }
    }

    fn end_field(&mut self, last: bool) {
        // The record's line break may be CRLF: its CR belongs to no field.
        if last && self.data.last() == Some(&b'\r') {
            let start = self.ends.last().map_or(0, |e| e + 1);
            if self.data.len() > start {
                self.data.pop();
            }
        }
        self.ends.push(self.data.len());
        self.data.push(SEP);
    }
}

/// Largest `n <= max` that does not split a UTF-8 sequence in `bytes`.
fn floor_char_boundary(bytes: &[u8], max: usize) -> usize {
    let mut n = max.min(bytes.len());
    while n > 0 && n < bytes.len() && (bytes[n] & 0xC0) == 0x80 {
        n -= 1;
    }
    n
}

/// How a record ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordEnd {
    /// Offset in `buf` where the next record starts.
    pub next: usize,
    /// A quoted field ran past [`MAX_RECORD`], or the record past [`MAX_HARD`].
    pub malformed: bool,
    /// Fields in the record (including dropped ones).
    pub fields: usize,
}

/// Scan the record starting at `start` in `buf`. `eof` says `buf` ends where the file
/// does; otherwise `None` means the record may continue past `buf` and the caller must
/// supply more bytes (always at least up to `start + MAX_HARD`, or the end of the file).
/// With `sink`, the record's fields are collected into it (it is cleared first).
pub fn scan_record(
    buf: &[u8],
    start: usize,
    d: &Dialect,
    eof: bool,
    mut sink: Option<&mut Fields>,
) -> Option<RecordEnd> {
    if let Some(f) = sink.as_deref_mut() {
        f.clear();
    }
    let len = buf.len();
    if start >= len {
        return eof.then_some(RecordEnd {
            next: len,
            malformed: false,
            fields: 0,
        });
    }
    let hard = start.saturating_add(MAX_HARD);
    let quoted_limit = start.saturating_add(MAX_RECORD);
    let delim = d.delimiter;
    let mut i = start;
    let mut fields = 0usize;
    let mut malformed = false;
    loop {
        // A field starts at `i`.
        fields += 1;
        let keep = match sink.as_deref_mut() {
            Some(f) => f.begin_field(),
            None => false,
        };
        let field_data_start = sink.as_deref().map_or(0, |f| f.data.len());
        let mut unquoted_from = i;
        if d.quote.is_some_and(|q| buf.get(i) == Some(&q)) && !malformed {
            let q = d.quote.unwrap_or(b'"');
            let mut j = i + 1;
            loop {
                let window_end = len.min(quoted_limit);
                match buf.get(j..window_end).and_then(|w| memchr(q, w)) {
                    Some(p) => {
                        let at = j + p;
                        if keep && let Some(f) = sink.as_deref_mut() {
                            f.push_bytes(field_data_start, buf.get(j..at).unwrap_or(&[]));
                        }
                        match buf.get(at + 1) {
                            Some(&c) if c == q => {
                                if keep && let Some(f) = sink.as_deref_mut() {
                                    f.push_bytes(field_data_start, &[q]);
                                }
                                j = at + 2;
                            }
                            Some(_) => {
                                j = at + 1;
                                break;
                            }
                            None if eof => {
                                j = at + 1;
                                break;
                            }
                            None => return None,
                        }
                    }
                    None if len >= quoted_limit => {
                        // Unclosed within the cap: the rest of the record is unquoted.
                        if keep && let Some(f) = sink.as_deref_mut() {
                            f.push_bytes(field_data_start, buf.get(j..quoted_limit).unwrap_or(&[]));
                        }
                        j = quoted_limit;
                        malformed = true;
                        break;
                    }
                    None if eof => {
                        if keep && let Some(f) = sink.as_deref_mut() {
                            f.push_bytes(field_data_start, buf.get(j..len).unwrap_or(&[]));
                        }
                        j = len;
                        malformed = true;
                        break;
                    }
                    None => return None,
                }
            }
            unquoted_from = j;
        }
        // Unquoted part (all of an unquoted field, or what follows a closing quote).
        let window_end = len.min(hard);
        let found = if malformed {
            // After an unclosed quote only a line break ends the record.
            buf.get(unquoted_from..window_end)
                .and_then(|w| memchr(b'\n', w))
                .map(|p| unquoted_from + p)
        } else {
            buf.get(unquoted_from..window_end)
                .and_then(|w| memchr2(delim, b'\n', w))
                .map(|p| unquoted_from + p)
        };
        match found {
            Some(at) => {
                if keep && let Some(f) = sink.as_deref_mut() {
                    f.push_bytes(field_data_start, buf.get(unquoted_from..at).unwrap_or(&[]));
                }
                let newline = buf.get(at) == Some(&b'\n');
                if let Some(f) = sink.as_deref_mut()
                    && keep
                {
                    f.end_field(newline);
                }
                if newline {
                    return Some(RecordEnd {
                        next: at + 1,
                        malformed,
                        fields,
                    });
                }
                i = at + 1;
            }
            None if len >= hard => {
                if keep && let Some(f) = sink.as_deref_mut() {
                    f.push_bytes(
                        field_data_start,
                        buf.get(unquoted_from..hard).unwrap_or(&[]),
                    );
                    f.end_field(true);
                }
                return Some(RecordEnd {
                    next: hard,
                    malformed: true,
                    fields,
                });
            }
            None if eof => {
                if keep && let Some(f) = sink.as_deref_mut() {
                    f.push_bytes(field_data_start, buf.get(unquoted_from..len).unwrap_or(&[]));
                    f.end_field(true);
                }
                return Some(RecordEnd {
                    next: len,
                    malformed,
                    fields,
                });
            }
            None => return None,
        }
    }
}
