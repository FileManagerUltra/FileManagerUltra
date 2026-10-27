//! Reading logs through the `Vfs`: tail-first windows, `.gz`, earlier chunks, follow.

use std::io::{BufRead, BufReader, Read};

use fm_vfs::{Entry, FmError, Vfs};

use super::parse::{self, Header};
use super::{LogDoc, LogFormat, LogLimits, LogRecord};
use crate::{Limits, read_head, text};

/// Read one line of at most `cap` bytes into `buf` (the excess is discarded). Returns
/// the raw bytes consumed, `None` at end of input.
fn read_line(
    r: &mut impl BufRead,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<Option<usize>> {
    buf.clear();
    let mut used = 0usize;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return Ok((used > 0).then_some(used));
        }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        let room = cap.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..take.min(room)]);
        r.consume(take);
        used += take;
        if done {
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(Some(used));
        }
    }
}

fn to_record(h: Option<Header>, line: &str) -> LogRecord {
    match h {
        Some(h) => LogRecord {
            ts: h.ts,
            level: h.level,
            status: h.status,
            fields: h.fields,
            text: line.to_string(),
            orphan: false,
        },
        None => LogRecord {
            ts: None,
            level: None,
            status: None,
            fields: Vec::new(),
            text: line.to_string(),
            orphan: true,
        },
    }
}

/// Parse lines from `r` into records; stops after `byte_limit` raw bytes. Returns the
/// records and the raw bytes consumed.
fn parse_stream(
    r: impl Read,
    format: LogFormat,
    lim: &LogLimits,
    byte_limit: u64,
) -> Result<(Vec<LogRecord>, u64), FmError> {
    let mut r = BufReader::with_capacity(1 << 16, r);
    let mut buf = Vec::new();
    let mut records: Vec<LogRecord> = Vec::new();
    let mut consumed = 0u64;
    let mut hidden = 0usize;
    while consumed < byte_limit && records.len() < lim.max_records {
        let Some(n) = read_line(&mut r, &mut buf, lim.max_line)? else {
            break;
        };
        consumed += n as u64;
        let line = String::from_utf8_lossy(&buf);
        match parse::header(format, &line) {
            Some(h) => {
                flush_hidden(&mut records, &mut hidden);
                records.push(to_record(Some(h), &line));
            }
            None => match records.last_mut() {
                Some(last) if last.extra_lines() < lim.max_record_lines => {
                    last.text.push('\n');
                    last.text.push_str(&line);
                }
                Some(_) => hidden += 1,
                None => records.push(to_record(None, &line)),
            },
        }
    }
    flush_hidden(&mut records, &mut hidden);
    Ok((records, consumed))
}

fn flush_hidden(records: &mut [LogRecord], hidden: &mut usize) {
    if *hidden > 0
        && let Some(last) = records.last_mut()
    {
        last.text.push_str(&format!("\n… {hidden} more lines"));
    }
    *hidden = 0;
}

fn size_of(records: &[LogRecord]) -> u64 {
    records.iter().map(|r| r.text.len() as u64 + 32).sum()
}

fn skip_partial_line(r: &mut impl BufRead) -> std::io::Result<u64> {
    let mut total = 0u64;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return Ok(total);
        }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        r.consume(take);
        total += take as u64;
        if done {
            return Ok(total);
        }
    }
}

fn pick_format(entry: &Entry, head: &[u8]) -> LogFormat {
    let path = entry.uri.path_str().replace('\\', "/").to_ascii_lowercase();
    let sample = text::decode(head);
    if path.contains("/var/report/") && sample.trim_start().starts_with('{') {
        return LogFormat::MagentoReport;
    }
    match super::detect_text(&sample) {
        Some(LogFormat::NginxAccess) if path.contains("apache") || path.contains("httpd") => {
            LogFormat::ApacheAccess
        }
        Some(LogFormat::NginxError) if path.contains("apache") => LogFormat::ApacheError,
        Some(f) => f,
        None => LogFormat::Generic,
    }
}

fn magento_report(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<LogDoc, FmError> {
    let (bytes, more) = text::read_capped(vfs, entry, limits.text_bytes)?;
    let src = text::decode(&bytes);
    let rec = match serde_json::from_str::<serde_json::Value>(&src) {
        Ok(serde_json::Value::Object(m)) => {
            let mut parts = Vec::new();
            for (k, v) in &m {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if k.chars().all(|c| c.is_ascii_digit()) {
                    parts.push(s);
                } else {
                    parts.push(format!("{k}: {s}"));
                }
            }
            LogRecord {
                ts: None,
                level: Some(super::Level::Error),
                status: None,
                fields: Vec::new(),
                text: parts.join("\n"),
                orphan: false,
            }
        }
        _ => to_record(
            parse::header(LogFormat::Generic, src.lines().next().unwrap_or("")),
            &src,
        ),
    };
    let bytes_len = rec.text.len() as u64;
    Ok(LogDoc {
        format: LogFormat::MagentoReport,
        records: vec![rec],
        more_before: false,
        start_offset: 0,
        end_offset: entry.size.unwrap_or(bytes_len),
        gz: false,
        bytes: bytes_len,
        note: more.then(|| "report truncated".to_string()),
        base: 0,
        marks: Vec::new(),
    })
}

pub fn open_doc(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<LogDoc, FmError> {
    open_doc_with(vfs, entry, limits, &LogLimits::default())
}

pub fn open_doc_with(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    lim: &LogLimits,
) -> Result<LogDoc, FmError> {
    let gz = entry.name.to_ascii_lowercase().ends_with(".gz");
    let raw_head = if gz {
        let mut d = flate2::read::GzDecoder::new(vfs.open_read(&entry.uri)?);
        let mut h = Vec::new();
        let _ = d.by_ref().take(8192).read_to_end(&mut h);
        h
    } else {
        read_head(vfs, entry, 8192)?
    };
    let format = pick_format(entry, &raw_head);
    if format == LogFormat::MagentoReport {
        return magento_report(vfs, entry, limits);
    }
    if gz {
        return open_gz(vfs, entry, format, lim);
    }
    let size = entry.size.unwrap_or(0);
    let mut offset = size.saturating_sub(lim.initial_tail);
    let mut reader = BufReader::with_capacity(1 << 16, vfs.open_read_at(&entry.uri, offset)?);
    if offset > 0 {
        offset += skip_partial_line(&mut reader)?;
    }
    let (records, consumed) = parse_stream(&mut reader, format, lim, lim.initial_tail)?;
    let mut records = records;
    if offset > 0 && records.first().is_some_and(|r| r.orphan) {
        records.remove(0);
    }
    let note = (records.len() >= lim.max_records)
        .then(|| "record limit reached; remaining records not loaded".into());
    Ok(LogDoc {
        format,
        bytes: size_of(&records),
        records,
        more_before: offset > 0,
        start_offset: offset,
        end_offset: offset + consumed,
        gz: false,
        note,
        base: 0,
        marks: Vec::new(),
    })
}

fn open_gz(
    vfs: &dyn Vfs,
    entry: &Entry,
    format: LogFormat,
    lim: &LogLimits,
) -> Result<LogDoc, FmError> {
    let dec = flate2::read::GzDecoder::new(vfs.open_read(&entry.uri)?);
    let (mut records, _) =
        parse_stream(dec.take(lim.max_bytes * 4), format, lim, lim.max_bytes * 4)?;
    let record_cap = records.len() >= lim.max_records;
    let mut more_before = false;
    let mut bytes = size_of(&records);
    if bytes > lim.max_bytes {
        let mut drop = 0;
        while bytes > lim.max_bytes && drop < records.len() {
            bytes -= records[drop].text.len() as u64 + 32;
            drop += 1;
        }
        records.drain(..drop);
        more_before = true;
    }
    Ok(LogDoc {
        format,
        records,
        more_before,
        start_offset: 0,
        end_offset: 0,
        gz: true,
        bytes,
        note: Some(if record_cap {
            "compressed log: record limit reached; remaining records not loaded".into()
        } else {
            "compressed log: read-only, no follow".into()
        }),
        base: 0,
        marks: Vec::new(),
    })
}

/// The chunk of records before a window, read without touching the document.
#[derive(Debug, PartialEq)]
pub struct EarlierChunk {
    pub records: Vec<LogRecord>,
    /// New `start_offset` of the document once applied.
    pub start: u64,
    pub note: Option<String>,
}

/// Read the chunk before byte `start_offset`. Empty (with a note) when the memory cap is
/// reached; the caller applies it with [`LogDoc::apply_earlier`].
pub fn read_earlier(
    vfs: &dyn Vfs,
    entry: &Entry,
    format: LogFormat,
    start_offset: u64,
    held_bytes: u64,
    lim: &LogLimits,
) -> Result<EarlierChunk, FmError> {
    if start_offset == 0 {
        return Ok(EarlierChunk {
            records: Vec::new(),
            start: 0,
            note: None,
        });
    }
    if held_bytes >= lim.max_bytes {
        return Ok(EarlierChunk {
            records: Vec::new(),
            start: start_offset,
            note: Some("memory window is full; use search or open the file in an editor".into()),
        });
    }
    let from = start_offset.saturating_sub(lim.chunk);
    let mut r = BufReader::with_capacity(1 << 16, vfs.open_read_at(&entry.uri, from)?);
    let mut start = from;
    if from > 0 {
        start += skip_partial_line(&mut r)?;
    }
    let span = start_offset.saturating_sub(start);
    let (mut records, _) = parse_stream(&mut r, format, lim, span)?;
    if start > 0 && records.first().is_some_and(|r| r.orphan) {
        records.remove(0);
    }
    Ok(EarlierChunk {
        records,
        start,
        note: None,
    })
}

/// Convenience for tests: read and apply in one step.
pub fn load_earlier(
    vfs: &dyn Vfs,
    entry: &Entry,
    doc: &LogDoc,
    lim: &LogLimits,
) -> Result<LogDoc, FmError> {
    let chunk = read_earlier(vfs, entry, doc.format, doc.start_offset, doc.bytes, lim)?;
    let mut out = doc.clone();
    out.apply_earlier(chunk);
    Ok(out)
}

/// What a follow poll found.
#[derive(Debug, PartialEq)]
pub enum FollowUpdate {
    Unchanged,
    /// New records to append (the first may continue the last record).
    Appended {
        records: Vec<LogRecord>,
        end_offset: u64,
    },
    /// The file shrank or was replaced: reloaded from scratch.
    Reloaded(Box<LogDoc>),
}

pub fn follow_poll(
    vfs: &dyn Vfs,
    entry: &Entry,
    doc: &LogDoc,
    limits: &Limits,
    lim: &LogLimits,
) -> Result<FollowUpdate, FmError> {
    if !doc.can_follow() {
        return Ok(FollowUpdate::Unchanged);
    }
    let now = vfs.stat(&entry.uri)?;
    let size = now.size.unwrap_or(0);
    if size < doc.end_offset {
        let mut fresh = now.clone();
        fresh.name = entry.name.clone();
        return open_doc_with(vfs, &fresh, limits, lim)
            .map(|d| FollowUpdate::Reloaded(Box::new(d)));
    }
    if size == doc.end_offset {
        return Ok(FollowUpdate::Unchanged);
    }
    let mut r = BufReader::with_capacity(1 << 16, vfs.open_read_at(&entry.uri, doc.end_offset)?);
    let (records, consumed) = parse_stream(&mut r, doc.format, lim, size - doc.end_offset)?;
    if records.is_empty() {
        return Ok(FollowUpdate::Unchanged);
    }
    Ok(FollowUpdate::Appended {
        records,
        end_offset: doc.end_offset + consumed,
    })
}

impl LogDoc {
    pub fn apply_earlier(&mut self, chunk: EarlierChunk) {
        if chunk.note.is_some() {
            self.note = chunk.note;
        }
        let mut records = chunk.records;
        // The chunk's last record may be the head of one whose rest we already hold.
        if let (Some(first), Some(prev)) = (self.records.first(), records.last_mut())
            && first.orphan
        {
            prev.text.push('\n');
            prev.text.push_str(&first.text);
            self.records.remove(0);
            self.base += 1;
        }
        let added = records.len() as i64;
        records.append(&mut self.records);
        self.base -= added;
        self.bytes = size_of(&records);
        self.records = records;
        self.start_offset = chunk.start;
        self.more_before = chunk.start > 0;
    }

    /// Merge a follow update; trims the oldest records past the memory cap.
    pub fn append(&mut self, mut records: Vec<LogRecord>, end_offset: u64, lim: &LogLimits) {
        if let (Some(first), Some(last)) = (records.first(), self.records.last_mut())
            && first.orphan
        {
            last.text.push('\n');
            last.text.push_str(&first.text);
            records.remove(0);
        }
        self.records.append(&mut records);
        self.end_offset = end_offset;
        self.bytes = size_of(&self.records);
        if self.bytes > lim.max_bytes {
            let mut drop = 0;
            let mut b = self.bytes;
            while b > lim.max_bytes && drop + 1 < self.records.len() {
                b -= self.records[drop].text.len() as u64 + 32;
                drop += 1;
            }
            self.records.drain(..drop);
            self.base += drop as i64;
            self.marks.retain(|m| *m >= self.base);
            self.bytes = b;
            self.more_before = true;
            self.start_offset = self.start_offset.max(1);
        }
    }
}
