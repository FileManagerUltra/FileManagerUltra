//! Filtering and sorting (design D7, D8): one sequential pass that keeps the matching
//! records' offsets and row numbers, with sort keys when a sort is asked for.

use std::cmp::Ordering;

use fm_vfs::FmError;
use memchr::memmem;
use regex::bytes::{Regex, RegexBuilder};

use super::scan::Fields;
use super::source::{ByteSource, Reader};
use super::{Dialect, parse_number};

/// Column conditions per table.
pub const MAX_CONDITIONS: usize = 16;
/// Sort keys per table.
pub const MAX_SORT_KEYS: usize = 3;
/// Longest filter text or condition value.
pub const MAX_QUERY: usize = 1024;
/// Rows a filtered or sorted view may hold.
pub const MAX_VIEW_ROWS: usize = 20_000_000;
/// Memory a sort's keys may take.
pub const SORT_BUDGET: usize = 512 << 20;
/// Bytes of a text value used as its sort key.
pub const TEXT_KEY: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    Greater,
    Less,
    Empty,
    NotEmpty,
}

impl Op {
    pub const ALL: [Op; 8] = [
        Op::Contains,
        Op::NotContains,
        Op::Equals,
        Op::NotEquals,
        Op::Greater,
        Op::Less,
        Op::Empty,
        Op::NotEmpty,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Op::Contains => "contains",
            Op::NotContains => "not_contains",
            Op::Equals => "equals",
            Op::NotEquals => "not_equals",
            Op::Greater => "greater",
            Op::Less => "less",
            Op::Empty => "empty",
            Op::NotEmpty => "not_empty",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Op::Contains => "contains",
            Op::NotContains => "does not contain",
            Op::Equals => "equals",
            Op::NotEquals => "does not equal",
            Op::Greater => "greater than",
            Op::Less => "less than",
            Op::Empty => "is empty",
            Op::NotEmpty => "is not empty",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.id() == s)
    }

    /// The operator compares with a value (not "is empty").
    pub fn takes_value(self) -> bool {
        !matches!(self, Op::Empty | Op::NotEmpty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Condition {
    pub column: usize,
    pub op: Op,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Filter {
    pub query: String,
    pub regex: bool,
    /// Limit the quick filter to one column.
    pub column: Option<usize>,
    pub conditions: Vec<Condition>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        self.query.is_empty() && self.conditions.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SortKey {
    pub column: usize,
    pub descending: bool,
}

/// Lower-case `s` for case-insensitive matching.
fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// Case-insensitive "contains" of a pre-folded needle.
struct Needle {
    folded: String,
    ascii: bool,
    finder: memmem::Finder<'static>,
}

impl Needle {
    fn new(text: &str) -> Self {
        let folded = fold(text);
        let finder = memmem::Finder::new(folded.as_bytes()).into_owned();
        Self {
            ascii: folded.is_ascii(),
            folded,
            finder,
        }
    }

    fn found_in(&self, hay: &[u8], scratch: &mut Vec<u8>) -> bool {
        if self.folded.is_empty() {
            return true;
        }
        if self.ascii {
            scratch.clear();
            scratch.extend(hay.iter().map(u8::to_ascii_lowercase));
            self.finder.find(scratch).is_some()
        } else {
            fold(&String::from_utf8_lossy(hay)).contains(&self.folded)
        }
    }

    fn equals(&self, hay: &[u8]) -> bool {
        if self.ascii {
            hay.eq_ignore_ascii_case(self.folded.as_bytes())
        } else {
            fold(&String::from_utf8_lossy(hay)) == self.folded
        }
    }
}

enum Quick {
    None,
    Text(Box<Needle>),
    Regex(Regex),
}

struct Compiled {
    column: usize,
    op: Op,
    needle: Needle,
    number: Option<f64>,
}

/// A compiled [`Filter`].
pub struct Matcher {
    quick: Quick,
    column: Option<usize>,
    conditions: Vec<Compiled>,
    decimal_comma: bool,
}

impl Matcher {
    pub fn new(filter: &Filter, decimal_comma: bool) -> Result<Self, String> {
        if filter.query.len() > MAX_QUERY {
            return Err("the filter text is too long".into());
        }
        if filter.conditions.len() > MAX_CONDITIONS {
            return Err(format!("at most {MAX_CONDITIONS} conditions"));
        }
        let quick = if filter.query.is_empty() {
            Quick::None
        } else if filter.regex {
            Quick::Regex(
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
            Quick::Text(Box::new(Needle::new(&filter.query)))
        };
        let conditions = filter
            .conditions
            .iter()
            .map(|c| Compiled {
                column: c.column,
                op: c.op,
                needle: Needle::new(&c.value),
                number: parse_number(c.value.as_bytes(), decimal_comma),
            })
            .collect();
        Ok(Self {
            quick,
            column: filter.column,
            conditions,
            decimal_comma,
        })
    }

    pub fn matches(&self, f: &Fields, scratch: &mut Vec<u8>) -> bool {
        let quick = match (&self.quick, self.column) {
            (Quick::None, _) => true,
            (Quick::Text(n), Some(c)) => n.found_in(f.get(c), scratch),
            (Quick::Text(n), None) => n.found_in(f.joined(), scratch),
            (Quick::Regex(re), Some(c)) => re.is_match(f.get(c)),
            (Quick::Regex(re), None) => f.iter().any(|v| re.is_match(v)),
        };
        quick && self.conditions.iter().all(|c| self.check(c, f, scratch))
    }

    fn check(&self, c: &Compiled, f: &Fields, scratch: &mut Vec<u8>) -> bool {
        let v = f.get(c.column);
        match c.op {
            Op::Contains => c.needle.found_in(v, scratch),
            Op::NotContains => !c.needle.found_in(v, scratch),
            Op::Equals => c.needle.equals(v.trim_ascii()),
            Op::NotEquals => !c.needle.equals(v.trim_ascii()),
            Op::Empty => v.trim_ascii().is_empty(),
            Op::NotEmpty => !v.trim_ascii().is_empty(),
            Op::Greater | Op::Less => {
                if v.trim_ascii().is_empty() {
                    return false;
                }
                let ord = match (c.number, parse_number(v, self.decimal_comma)) {
                    (Some(want), Some(have)) => have.total_cmp(&want),
                    _ => fold(&String::from_utf8_lossy(v.trim_ascii())).cmp(&c.needle.folded),
                };
                if c.op == Op::Greater {
                    ord == Ordering::Greater
                } else {
                    ord == Ordering::Less
                }
            }
        }
    }
}

/// A sort key value.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Num(f64),
    Text(Box<[u8]>),
    Empty,
}

impl Key {
    fn of(v: &[u8], numeric: bool, decimal_comma: bool) -> Key {
        let t = v.trim_ascii();
        if t.is_empty() {
            return Key::Empty;
        }
        if numeric && let Some(n) = parse_number(t, decimal_comma) {
            return Key::Num(n);
        }
        let s = String::from_utf8_lossy(t);
        let mut folded = fold(&s).into_bytes();
        if folded.len() > TEXT_KEY {
            folded.truncate(TEXT_KEY);
        }
        Key::Text(folded.into_boxed_slice())
    }

    fn heap(&self) -> usize {
        match self {
            Key::Text(t) => t.len(),
            _ => 0,
        }
    }
}

/// Compare two keys: numbers before text, empty last in both directions.
pub fn compare(a: &Key, b: &Key, descending: bool) -> Ordering {
    let ord = match (a, b) {
        (Key::Empty, Key::Empty) => return Ordering::Equal,
        (Key::Empty, _) => return Ordering::Greater,
        (_, Key::Empty) => return Ordering::Less,
        (Key::Num(x), Key::Num(y)) => x.total_cmp(y),
        (Key::Num(_), Key::Text(_)) => Ordering::Less,
        (Key::Text(_), Key::Num(_)) => Ordering::Greater,
        (Key::Text(x), Key::Text(y)) => x.cmp(y),
    };
    if descending { ord.reverse() } else { ord }
}

/// What a view job computes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewSpec {
    pub filter: Filter,
    pub sort: Vec<SortKey>,
    /// Per column: sort by numeric value.
    pub numeric: Vec<bool>,
    pub decimal_comma: bool,
}

/// Why a view job stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// Bad input from the user (invalid regex, too many conditions): keep the old view.
    Invalid(String),
    /// Over a budget.
    TooLarge(String),
    Cancelled,
    Read(String),
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewError::Invalid(s) | ViewError::TooLarge(s) | ViewError::Read(s) => f.write_str(s),
            ViewError::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl From<FmError> for ViewError {
    fn from(e: FmError) -> Self {
        ViewError::Read(e.to_string())
    }
}

/// One row of a view: the record's offset and its row number in the file (0-based).
pub type ViewRow = (u64, u64);

/// Run a filter and/or sort over every record from `start`. `stop` is polled now and
/// then; `progress` receives the data offset reached.
pub fn run_view(
    src: &dyn ByteSource,
    d: Dialect,
    start: u64,
    spec: &ViewSpec,
    stop: &dyn Fn() -> bool,
    progress: &dyn Fn(u64),
) -> Result<Vec<ViewRow>, ViewError> {
    if spec.sort.len() > MAX_SORT_KEYS {
        return Err(ViewError::Invalid(format!(
            "sort by at most {MAX_SORT_KEYS} columns"
        )));
    }
    let matcher = if spec.filter.is_empty() {
        None
    } else {
        Some(Matcher::new(&spec.filter, spec.decimal_comma).map_err(ViewError::Invalid)?)
    };
    let mut reader = Reader::new(src, d, start, 4 << 20);
    let mut fields = Fields::default();
    let mut scratch = Vec::new();
    let mut plain: Vec<ViewRow> = Vec::new();
    let mut keyed: Vec<(Vec<Key>, u64, u64)> = Vec::new();
    let mut key_bytes = 0usize;
    let mut row = 0u64;
    while let Some(found) = reader.next(Some(&mut fields))? {
        if row.is_multiple_of(4096) {
            if stop() {
                return Err(ViewError::Cancelled);
            }
            progress(found.start);
        }
        let this = row;
        row += 1;
        if let Some(m) = &matcher
            && !m.matches(&fields, &mut scratch)
        {
            continue;
        }
        if spec.sort.is_empty() {
            if plain.len() >= MAX_VIEW_ROWS {
                return Err(ViewError::TooLarge(format!(
                    "more than {} rows match; narrow the filter",
                    MAX_VIEW_ROWS
                )));
            }
            plain.push((found.start, this));
        } else {
            let keys: Vec<Key> = spec
                .sort
                .iter()
                .map(|k| {
                    Key::of(
                        fields.get(k.column),
                        spec.numeric.get(k.column).copied().unwrap_or(false),
                        spec.decimal_comma,
                    )
                })
                .collect();
            key_bytes +=
                48 + keys.len() * size_of::<Key>() + keys.iter().map(Key::heap).sum::<usize>();
            if key_bytes > SORT_BUDGET || keyed.len() >= MAX_VIEW_ROWS {
                return Err(ViewError::TooLarge(
                    "too many rows to sort; filter the table first".into(),
                ));
            }
            keyed.push((keys, found.start, this));
        }
    }
    if stop() {
        return Err(ViewError::Cancelled);
    }
    if spec.sort.is_empty() {
        return Ok(plain);
    }
    // Stable: equal keys keep file order.
    keyed.sort_by(|a, b| {
        for (i, k) in spec.sort.iter().enumerate() {
            let ord = match (a.0.get(i), b.0.get(i)) {
                (Some(x), Some(y)) => compare(x, y, k.descending),
                _ => Ordering::Equal,
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
    Ok(keyed.into_iter().map(|(_, off, row)| (off, row)).collect())
}
