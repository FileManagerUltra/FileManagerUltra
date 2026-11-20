//! Delimited text as a table (change `add-table-preview`): dialect and header detection,
//! the quote-aware scanner, byte sources, the sparse record index, sorting and filtering.
//! Pure data and functions; the engine owns the threads that run them.

pub mod index;
pub mod query;
pub mod scan;
pub mod source;

use std::io::Read;

use fm_vfs::{Entry, FmError, Vfs};

use crate::{Limits, Preview, TextDoc, text};
use scan::{Fields, scan_record};

/// Files up to this size are held in memory; larger ones are read in blocks.
pub const MEMORY_BYTES: u64 = 256 << 20;
/// Bytes read to detect the dialect and show the first rows.
pub const SAMPLE_BYTES: usize = 1 << 20;
/// Rows parsed for the first view (and column types and widths).
pub const SAMPLE_ROWS: usize = 2_000;
/// Delimiters tried by detection, in order of preference on a tie.
pub const DELIMITERS: [u8; 4] = *b",;\t|";

/// How the text is split into records and fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dialect {
    pub delimiter: u8,
    /// `None`: quotes are ordinary characters.
    pub quote: Option<u8>,
}

impl Default for Dialect {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: Some(b'"'),
        }
    }
}

/// Human name of a delimiter.
pub fn delimiter_label(d: u8) -> &'static str {
    match d {
        b',' => "Comma",
        b';' => "Semicolon",
        b'\t' => "Tab",
        b'|' => "Pipe",
        _ => "Other",
    }
}

/// How the file's bytes become the UTF-8 the table reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8 (a BOM, if any, is skipped by the data start).
    Utf8,
    /// UTF-16 with a BOM, transcoded to UTF-8 in memory.
    Utf16Le,
    Utf16Be,
}

/// The user's choices; `None` means detect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableOptions {
    pub delimiter: Option<u8>,
    pub header: Option<bool>,
}

/// What a column holds, from the sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    Integer,
    Decimal,
    Text,
}

impl ColumnKind {
    pub fn numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Decimal)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Integer => "whole numbers",
            Self::Decimal => "numbers",
            Self::Text => "text",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub kind: ColumnKind,
    /// Display width in characters, from the header and the sample.
    pub width: u16,
}

/// One parsed record for display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub cells: Vec<Box<str>>,
    /// Cells cut at the display cap.
    pub cut: Vec<usize>,
}

impl Row {
    pub fn from_fields(f: &Fields) -> Self {
        Self {
            cells: f
                .iter()
                .map(|b| String::from_utf8_lossy(b).into_owned().into_boxed_str())
                .collect(),
            cut: f.cut.clone(),
        }
    }

    pub fn cell(&self, i: usize) -> &str {
        self.cells.get(i).map_or("", |c| c)
    }
}

/// A table preview: everything needed to show the first rows at once and to open the
/// whole file in the background.
#[derive(Clone, Debug, PartialEq)]
pub struct TableDoc {
    pub dialect: Dialect,
    /// What detection chose (shown next to "Auto").
    pub detected: Dialect,
    pub header: bool,
    pub detected_header: bool,
    pub options: TableOptions,
    pub encoding: Encoding,
    /// Offset (in UTF-8 data) of the first data record, after a BOM and the header.
    pub data_start: u64,
    pub columns: Vec<Column>,
    /// The first records, for the view before the index is ready.
    pub sample: Vec<Row>,
    /// `sample` holds every record of the file.
    pub sample_complete: bool,
    /// Malformed records seen in the sample.
    pub sample_malformed: u64,
    /// Decimal commas count as decimal points (`;`-delimited files).
    pub decimal_comma: bool,
    /// The file size on disk.
    pub size: u64,
    /// The raw text, for the Source toggle.
    pub raw: TextDoc,
}

impl TableDoc {
    /// Name of column `i`, also for columns beyond the header ("Column 7").
    pub fn column_name(&self, i: usize) -> String {
        self.columns
            .get(i)
            .map_or_else(|| format!("Column {}", i + 1), |c| c.name.clone())
    }

    pub fn column_kind(&self, i: usize) -> ColumnKind {
        self.columns.get(i).map_or(ColumnKind::Text, |c| c.kind)
    }

    pub fn numeric_columns(&self) -> Vec<bool> {
        self.columns.iter().map(|c| c.kind.numeric()).collect()
    }
}

/// Encoding from a byte-order mark, and how many bytes it takes.
pub fn encoding_of(head: &[u8]) -> (Encoding, usize) {
    if head.starts_with(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, 2)
    } else if head.starts_with(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, 2)
    } else if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8, 3)
    } else {
        (Encoding::Utf8, 0)
    }
}

/// UTF-16 (after its BOM) as UTF-8.
pub fn transcode_utf16(bytes: &[u8], enc: Encoding) -> Vec<u8> {
    let f: fn([u8; 2]) -> u16 = match enc {
        Encoding::Utf16Be => u16::from_be_bytes,
        _ => u16::from_le_bytes,
    };
    let units = bytes.as_chunks::<2>().0.iter().map(|c| f(*c));
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect::<String>()
        .into_bytes()
}

fn ext_is_tab(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tsv") || lower.ends_with(".tab")
}

/// Field counts of the first records of `data` with `d` (complete records only).
fn field_counts(data: &[u8], d: &Dialect, eof: bool, max: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pos = 0;
    while out.len() < max {
        let Some(end) = scan_record(data, pos, d, eof, None) else {
            break;
        };
        if end.next == pos {
            break;
        }
        // Blank lines say nothing about the delimiter.
        let blank = data
            .get(pos..end.next)
            .is_some_and(|r| r.iter().all(|b| *b == b'\n' || *b == b'\r'));
        if !blank {
            out.push(end.fields);
        }
        pos = end.next;
    }
    out
}

/// Pick the delimiter whose field counts are the most consistent (and, on a tie, give
/// the most fields). `.tsv`/`.tab` files prefer tab when it splits anything.
pub fn detect_delimiter(data: &[u8], eof: bool, name: &str) -> u8 {
    let tab_file = ext_is_tab(name);
    let mut best: Option<(u8, f64)> = None;
    for delim in DELIMITERS {
        let d = Dialect {
            delimiter: delim,
            quote: Some(b'"'),
        };
        let counts = field_counts(data, &d, eof, 100);
        if counts.is_empty() {
            continue;
        }
        let mut freq: std::collections::HashMap<usize, usize> = Default::default();
        for c in &counts {
            *freq.entry(*c).or_default() += 1;
        }
        let Some((&mode, &n)) = freq.iter().max_by_key(|(k, v)| (**v, **k)) else {
            continue;
        };
        if mode < 2 {
            continue;
        }
        if tab_file && delim == b'\t' {
            return b'\t';
        }
        let consistency = n as f64 / counts.len() as f64;
        let score = consistency * 100.0 + (mode.min(200) as f64) / 200.0;
        if best.is_none_or(|(_, s)| score > s + 1e-9) {
            best = Some((delim, score));
        }
    }
    best.map_or(if tab_file { b'\t' } else { b',' }, |(d, _)| d)
}

/// Parse a number as a person would read it: optional sign, digits with at most one
/// decimal separator (`.`, or also `,` when `decimal_comma`), optional exponent.
pub fn parse_number(b: &[u8], decimal_comma: bool) -> Option<f64> {
    let s = b.trim_ascii();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    let mut text = String::with_capacity(s.len());
    let mut digits = 0;
    let mut seps = 0;
    let mut exp = false;
    for (i, &c) in s.iter().enumerate() {
        match c {
            b'0'..=b'9' => {
                digits += 1;
                text.push(c as char);
            }
            b'+' | b'-' if i == 0 || matches!(s.get(i - 1), Some(b'e' | b'E')) => {
                text.push(c as char)
            }
            b'.' if !exp => {
                seps += 1;
                text.push('.');
            }
            b',' if decimal_comma && !exp => {
                seps += 1;
                text.push('.');
            }
            b'e' | b'E' if digits > 0 && !exp => {
                exp = true;
                text.push('e');
            }
            _ => return None,
        }
    }
    if digits == 0 || seps > 1 {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn looks_numeric(b: &[u8], decimal_comma: bool) -> bool {
    parse_number(b, decimal_comma).is_some()
}

/// Whether the first record is a header, given parsed records.
pub fn detect_header(records: &[Fields], decimal_comma: bool) -> bool {
    let Some(first) = records.first() else {
        return false;
    };
    let names: Vec<&[u8]> = first.iter().collect();
    // A numeric "name" means the first row is data.
    if names
        .iter()
        .any(|n| !n.trim_ascii().is_empty() && looks_numeric(n, decimal_comma))
    {
        return false;
    }
    let body = records.get(1..).unwrap_or(&[]);
    if body.is_empty() {
        return names.iter().all(|n| !n.trim_ascii().is_empty());
    }
    // A column that is numeric below a text name is the clearest sign.
    for (i, name) in names.iter().enumerate() {
        if name.trim_ascii().is_empty() {
            continue;
        }
        let values: Vec<&[u8]> = body
            .iter()
            .map(|r| r.get(i))
            .filter(|v| !v.trim_ascii().is_empty())
            .collect();
        if !values.is_empty() && values.iter().all(|v| looks_numeric(v, decimal_comma)) {
            return true;
        }
    }
    // Otherwise: distinct, non-empty names are a header; a first row that repeats
    // values or has gaps looks like data.
    let mut seen = std::collections::HashSet::new();
    names
        .iter()
        .all(|n| !n.trim_ascii().is_empty() && seen.insert(n.to_ascii_lowercase()))
}

/// Column kinds from body records.
fn column_kinds(rows: &[Fields], columns: usize, decimal_comma: bool) -> Vec<ColumnKind> {
    (0..columns)
        .map(|i| {
            let mut numbers = 0usize;
            let mut decimals = false;
            let mut others = 0usize;
            for r in rows {
                let v = r.get(i).trim_ascii();
                if v.is_empty() {
                    continue;
                }
                if looks_numeric(v, decimal_comma) {
                    numbers += 1;
                    decimals |= v.iter().any(|c| matches!(c, b'.' | b',' | b'e' | b'E'));
                } else {
                    others += 1;
                }
            }
            // Mostly numbers (a few "n/a" do not make it text).
            if numbers > 0 && others * 20 <= numbers {
                if decimals {
                    ColumnKind::Decimal
                } else {
                    ColumnKind::Integer
                }
            } else {
                ColumnKind::Text
            }
        })
        .collect()
}

fn width_of(b: &[u8]) -> usize {
    let first_line = b.split(|c| *c == b'\n').next().unwrap_or(&[]);
    String::from_utf8_lossy(first_line).chars().count()
}

/// Detection and the first rows, from the start of the (UTF-8) data.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub dialect: Dialect,
    pub detected: Dialect,
    pub header: bool,
    pub detected_header: bool,
    pub decimal_comma: bool,
    /// Offset of the first data record in `data` (after BOM and header).
    pub data_start: u64,
    pub columns: Vec<Column>,
    pub sample: Vec<Row>,
    pub sample_complete: bool,
    pub sample_malformed: u64,
}

/// Analyse the start of the data. `bom` is skipped; `eof` says `data` is the whole file.
pub fn analyze(data: &[u8], bom: usize, eof: bool, name: &str, options: TableOptions) -> Analysis {
    let body = data.get(bom..).unwrap_or(&[]);
    let detected = Dialect {
        delimiter: detect_delimiter(body, eof, name),
        quote: Some(b'"'),
    };
    let dialect = Dialect {
        delimiter: options.delimiter.unwrap_or(detected.delimiter),
        ..detected
    };
    let decimal_comma = dialect.delimiter == b';';
    // Parse the first records.
    let mut records: Vec<Fields> = Vec::new();
    let mut ends: Vec<usize> = Vec::new();
    let mut malformed = 0u64;
    let mut pos = bom;
    let mut complete = false;
    while records.len() <= SAMPLE_ROWS {
        let mut f = Fields::default();
        match scan_record(data, pos, &dialect, eof, Some(&mut f)) {
            Some(end) if end.next > pos => {
                malformed += u64::from(end.malformed);
                records.push(f);
                ends.push(end.next);
                pos = end.next;
            }
            Some(_) => {
                complete = true;
                break;
            }
            None => break,
        }
    }
    if pos >= data.len() && eof {
        complete = true;
    }
    let detected_header = detect_header(
        records.get(..records.len().min(50)).unwrap_or(&[]),
        decimal_comma,
    );
    let header = options.header.unwrap_or(detected_header);
    let (names, body_rows, data_start) = if header && !records.is_empty() {
        let names: Vec<String> = records
            .first()
            .map(|f| {
                f.iter()
                    .map(|b| String::from_utf8_lossy(b).trim().to_string())
                    .collect()
            })
            .unwrap_or_default();
        (
            names,
            records.get(1..).unwrap_or(&[]),
            ends.first().copied().unwrap_or(bom) as u64,
        )
    } else {
        (Vec::new(), &records[..], bom as u64)
    };
    let body_rows = body_rows
        .get(..body_rows.len().min(SAMPLE_ROWS))
        .unwrap_or(&[]);
    let columns_n = body_rows
        .iter()
        .map(Fields::len)
        .max()
        .unwrap_or(0)
        .max(names.len());
    let kinds = column_kinds(body_rows, columns_n, decimal_comma);
    let columns = (0..columns_n)
        .map(|i| {
            let name = names
                .get(i)
                .filter(|n| !n.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Column {}", i + 1));
            let widest = body_rows
                .iter()
                .take(500)
                .map(|r| width_of(r.get(i)))
                .max()
                .unwrap_or(0);
            Column {
                width: widest.max(name.chars().count()).clamp(3, 60) as u16,
                name,
                kind: kinds.get(i).copied().unwrap_or(ColumnKind::Text),
            }
        })
        .collect();
    // The sample is complete only if no record was left out by the row cap.
    let sample_complete = complete && records.len() <= SAMPLE_ROWS;
    Analysis {
        dialect,
        detected,
        header,
        detected_header,
        decimal_comma,
        data_start,
        columns,
        sample: body_rows.iter().map(Row::from_fields).collect(),
        sample_complete,
        sample_malformed: malformed,
    }
}

/// Read the start of a file as UTF-8 data: `(bytes, bom, eof, encoding)`. For UTF-16 the
/// bytes are already transcoded and the BOM is gone.
pub fn read_sample(
    vfs: &dyn Vfs,
    entry: &Entry,
) -> Result<(Vec<u8>, usize, bool, Encoding), FmError> {
    let mut raw = Vec::new();
    vfs.open_read(&entry.uri)?
        .take(SAMPLE_BYTES as u64 * 2 + 1)
        .read_to_end(&mut raw)?;
    let (enc, bom) = encoding_of(&raw);
    match enc {
        Encoding::Utf8 => {
            let eof = raw.len() <= SAMPLE_BYTES;
            raw.truncate(SAMPLE_BYTES);
            Ok((raw, bom, eof, enc))
        }
        _ => {
            let eof = raw.len() <= SAMPLE_BYTES * 2;
            let mut body = raw.get(bom..).unwrap_or(&[]).to_vec();
            body.truncate(SAMPLE_BYTES * 2);
            if body.len() % 2 == 1 {
                body.pop();
            }
            Ok((transcode_utf16(&body, enc), 0, eof, enc))
        }
    }
}

/// The table preview of `entry`.
pub fn render(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    options: TableOptions,
) -> Result<Preview, FmError> {
    let (data, bom, eof, encoding) = read_sample(vfs, entry)?;
    let a = analyze(&data, bom, eof, &entry.name, options);
    let raw_bytes = data.get(..data.len().min(limits.text_bytes)).unwrap_or(&[]);
    let raw_text = text::decode(raw_bytes);
    let size = entry.size.unwrap_or(data.len() as u64);
    let raw = text::make_doc(
        &raw_text,
        "csv",
        limits,
        !eof || data.len() > limits.text_bytes,
        size,
    );
    Ok(Preview::Table(Box::new(TableDoc {
        dialect: a.dialect,
        detected: a.detected,
        header: a.header,
        detected_header: a.detected_header,
        options,
        encoding,
        data_start: a.data_start,
        columns: a.columns,
        sample: a.sample,
        sample_complete: a.sample_complete,
        sample_malformed: a.sample_malformed,
        decimal_comma: a.decimal_comma,
        size,
        raw,
    })))
}

#[cfg(test)]
mod tests;
