//! Log mode: parse nginx/Apache/Magento/Pimcore/PHP logs into records, filter them, and
//! keep a bounded window of a possibly huge file (tail first, older content on demand).

mod load;
pub mod parse;

pub use load::{
    EarlierChunk, FollowUpdate, follow_poll, load_earlier, open_doc, open_doc_with, read_earlier,
};

use regex::RegexBuilder;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogFormat {
    NginxAccess,
    NginxError,
    ApacheAccess,
    ApacheError,
    /// Magento and Pimcore (Monolog line format).
    Monolog,
    Php,
    MagentoReport,
    Generic,
}

impl LogFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::NginxAccess => "nginx access",
            Self::NginxError => "nginx error",
            Self::ApacheAccess => "Apache access",
            Self::ApacheError => "Apache error",
            Self::Monolog => "Monolog (Magento/Pimcore)",
            Self::Php => "PHP",
            Self::MagentoReport => "Magento report",
            Self::Generic => "log",
        }
    }

    pub fn is_access(self) -> bool {
        matches!(self, Self::NginxAccess | Self::ApacheAccess)
    }

    /// Column titles of the access-log table view.
    pub const ACCESS_COLUMNS: [&'static str; 6] = ["Time", "IP", "Method", "URL", "Status", "Size"];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Debug,
        Level::Info,
        Level::Notice,
        Level::Warning,
        Level::Error,
        Level::Critical,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Notice => "Notice",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Critical => "Critical",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| l.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogRecord {
    pub ts: Option<i64>,
    pub level: Option<Level>,
    pub status: Option<u16>,
    /// Access-log columns (see [`LogFormat::ACCESS_COLUMNS`]); empty otherwise.
    pub fields: Vec<String>,
    /// First line is the header; the rest are continuation lines (stack trace).
    pub text: String,
    /// Started without a header line (the window began mid-record).
    pub orphan: bool,
}

impl LogRecord {
    /// Stable-ish identity for per-record UI state (fold) across window changes.
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.ts.hash(&mut h);
        self.first_line().hash(&mut h);
        h.finish()
    }

    pub fn first_line(&self) -> &str {
        self.text.lines().next().unwrap_or("")
    }

    pub fn extra_lines(&self) -> usize {
        self.text.lines().count().saturating_sub(1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogDoc {
    pub format: LogFormat,
    pub records: Vec<LogRecord>,
    /// Records before the window exist (older content not loaded).
    pub more_before: bool,
    /// Byte offset of the first loaded line (0 once everything is loaded).
    pub start_offset: u64,
    /// Byte offset just past the last consumed byte; follow resumes here.
    pub end_offset: u64,
    pub gz: bool,
    /// Bytes of record text held in memory.
    pub bytes: u64,
    pub note: Option<String>,
    /// Sequence number of `records[0]`; earlier chunks shrink it, trimming grows it.
    pub base: i64,
    /// Sequence numbers where the user set a marker (drawn above that record).
    pub marks: Vec<i64>,
}

impl LogDoc {
    /// Most markers kept per log.
    pub const MAX_MARKS: usize = 256;

    /// Put a marker after the last record currently loaded.
    pub fn mark_here(&mut self) {
        let at = self.base + self.records.len() as i64;
        if self.marks.last() != Some(&at) {
            if self.marks.len() >= Self::MAX_MARKS {
                self.marks.remove(0);
            }
            self.marks.push(at);
        }
    }

    pub fn can_follow(&self) -> bool {
        !self.gz && self.format != LogFormat::MagentoReport
    }
}

/// Window limits. Tests shrink them.
#[derive(Clone, Copy, Debug)]
pub struct LogLimits {
    pub initial_tail: u64,
    pub chunk: u64,
    pub max_bytes: u64,
    pub max_line: usize,
    pub max_record_lines: usize,
    /// Record-count cap for shared-budget multi-file previews.
    pub max_records: usize,
}

impl Default for LogLimits {
    fn default() -> Self {
        Self {
            initial_tail: 8 << 20,
            chunk: 8 << 20,
            max_bytes: 64 << 20,
            max_line: 64 << 10,
            max_record_lines: 1000,
            max_records: usize::MAX,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogFilter {
    pub min_level: Option<Level>,
    pub query: String,
    pub regex: bool,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl LogFilter {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Indices of the records that pass `filter`. An invalid regex is an error and leaves
/// the caller's view unchanged.
pub fn apply_filter(doc: &LogDoc, filter: &LogFilter) -> Result<Vec<u32>, String> {
    let re = if filter.regex && !filter.query.is_empty() {
        Some(
            RegexBuilder::new(&filter.query)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .dfa_size_limit(4 << 20)
                .build()
                .map_err(|e| {
                    format!(
                        "invalid pattern: {}",
                        e.to_string().lines().last().unwrap_or("")
                    )
                })?,
        )
    } else {
        None
    };
    let needle = filter.query.to_lowercase();
    let mut out = Vec::new();
    for (ix, r) in doc.records.iter().enumerate() {
        if let Some(min) = filter.min_level
            && r.level.unwrap_or(Level::Info) < min
        {
            continue;
        }
        if filter.from.is_some() || filter.to.is_some() {
            let Some(ts) = r.ts else { continue };
            if filter.from.is_some_and(|f| ts < f) || filter.to.is_some_and(|t| ts > t) {
                continue;
            }
        }
        if !filter.query.is_empty() {
            let hit = match &re {
                Some(re) => re.is_match(&r.text),
                None => r.text.to_lowercase().contains(&needle),
            };
            if !hit {
                continue;
            }
        }
        out.push(ix as u32);
    }
    Ok(out)
}

/// Parse a `YYYY-MM-DD[ HH:MM[:SS]]` bound typed by the user.
pub fn parse_bound(s: &str) -> Option<i64> {
    let s = s.trim();
    let mut padded = s.to_string();
    match s.len() {
        10 => padded.push_str(" 00:00:00"),
        16 => padded.push_str(":00"),
        _ => {}
    }
    parse::parse_iso(&padded)
}

pub use parse::detect_line;

/// Format of a text sample (first non-empty line decides), if it looks like a known log.
pub fn detect_text(sample: &str) -> Option<LogFormat> {
    sample
        .lines()
        .find(|l| !l.trim().is_empty())
        .and_then(detect_line)
}

#[cfg(test)]
mod tests;
