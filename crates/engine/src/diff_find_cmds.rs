//! Find inside a Preview pane's Diff view. The `preview.find.*` commands (Ctrl+F, F3) act on the
//! diff while it is shown; matches are plain-text, bounded in query length and count.

use std::{ops::Range, sync::Arc};

use fm_layout::PaneId;
use fm_vfs::FmError;
use regex::RegexBuilder;
use serde_json::Map;

use crate::{
    Params, Session,
    diff_cmds::{DiffStatus, Outcome, Row, Tag},
    preview_find_cmds::{MAX_MATCHES, MAX_QUERY},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: usize,
    /// Match is on the right (changed) side of the row.
    pub right: bool,
    pub bytes: Range<usize>,
}

#[derive(Debug, Default)]
pub struct DiffFind {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    pub hits: Vec<Hit>,
    pub capped: bool,
    pub current: usize,
    /// UI focus and reveal requests; each is handled once by the pane's widgets.
    pub focus: u64,
    pub reveal: u64,
    searched: Option<(usize, String, bool)>,
}

pub(crate) fn active(s: &Session, pane: PaneId) -> bool {
    s.pane_diff(pane).is_some_and(|d| d.active)
}

fn focused_active(s: &Session) -> bool {
    s.preview_pane(&Params::Null)
        .is_ok_and(|pane| active(s, pane))
}

pub(crate) fn available(s: &Session) -> bool {
    focused_active(s)
}

pub(crate) fn can_navigate(s: &Session) -> bool {
    s.preview_pane(&Params::Null).is_ok_and(|pane| {
        s.diffs
            .finds
            .get(&pane)
            .is_some_and(|f| active(s, pane) && f.open && !f.hits.is_empty())
    })
}

pub(crate) fn can_close(s: &Session) -> bool {
    s.preview_pane(&Params::Null).is_ok_and(|pane| {
        s.diffs
            .finds
            .get(&pane)
            .is_some_and(|f| active(s, pane) && f.open)
    })
}

fn lines(row: &Row) -> [Option<(bool, &str)>; 2] {
    let left = row.left.as_ref().map(|l| (false, l.text.as_str()));
    let right = row.right.as_ref().map(|l| (true, l.text.as_str()));
    match row.tag {
        // Identical on both sides: count it once.
        Tag::Equal => [right.or(left), None],
        _ => [left, right],
    }
}

fn search(rows: &[Row], query: &str, case_sensitive: bool) -> (Vec<Hit>, bool) {
    let mut hits = Vec::new();
    let Ok(re) = RegexBuilder::new(&regex::escape(query))
        .case_insensitive(!case_sensitive)
        .build()
    else {
        return (hits, false);
    };
    for (row, r) in rows.iter().enumerate() {
        for (right, text) in lines(r).into_iter().flatten() {
            for m in re.find_iter(text) {
                if hits.len() >= MAX_MATCHES {
                    return (hits, true);
                }
                hits.push(Hit {
                    row,
                    right,
                    bytes: m.range(),
                });
            }
        }
    }
    (hits, false)
}

/// Recompute the matches when the query or the compared content changed.
pub fn prepare(s: &mut Session, pane: PaneId) {
    let outcome = match s.diff_view(pane).map(|v| &v.status) {
        Some(DiffStatus::Ready(o)) => Some(o.clone()),
        _ => None,
    };
    let Some(find) = s.diffs.finds.get_mut(&pane) else {
        return;
    };
    let id = outcome.as_ref().map_or(0, |o| Arc::as_ptr(o) as usize);
    let want = (id, find.query.clone(), find.case_sensitive);
    if !find.open || find.searched.as_ref() == Some(&want) {
        return;
    }
    let rows: &[Row] = match outcome.as_deref() {
        Some(Outcome::Text(m)) if !find.query.is_empty() => &m.rows,
        _ => &[],
    };
    let (hits, capped) = search(rows, &find.query, find.case_sensitive);
    find.hits = hits;
    find.capped = capped;
    find.current = 0;
    find.reveal = find.reveal.wrapping_add(1);
    find.searched = Some(want);
}

pub(crate) fn change(
    s: &mut Session,
    pane: PaneId,
    object: Option<&Map<String, Params>>,
    mode: &str,
) -> Result<(), FmError> {
    let find = s.diffs.finds.entry(pane).or_default();
    match mode {
        "open" => {
            find.open = true;
            find.focus = find.focus.wrapping_add(1);
        }
        "close" => {
            if !find.open {
                return Err(FmError::Unsupported("Diff Find is not open"));
            }
            find.open = false;
        }
        "set" => {
            let object =
                object.ok_or_else(|| FmError::InvalidParams("expected search options".into()))?;
            let query = object
                .get("query")
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("query is required".into()))?;
            if query.chars().count() > MAX_QUERY {
                return Err(FmError::InvalidParams("query is too long".into()));
            }
            find.query = query.into();
            if let Some(v) = object.get("case_sensitive").and_then(Params::as_bool) {
                find.case_sensitive = v;
            }
        }
        _ => {
            let count = find.hits.len();
            if count == 0 || !find.open {
                return Err(FmError::Unsupported("no diff search matches"));
            }
            find.current = if mode == "next" {
                (find.current + 1) % count
            } else {
                (find.current + count - 1) % count
            };
            find.reveal = find.reveal.wrapping_add(1);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_git::diff::Line;

    fn row(left: Option<&str>, right: Option<&str>, tag: Tag) -> Row {
        let line = |t: &str| Line {
            no: 1,
            text: t.into(),
            marks: Vec::new(),
        };
        Row {
            left: left.map(line),
            right: right.map(line),
            tag,
        }
    }

    #[test]
    fn matches_each_visible_line_once_and_respect_case() {
        let rows = vec![
            row(Some("Été"), Some("Été"), Tag::Equal),
            row(Some("old foo"), Some("new FOO"), Tag::Replace),
            row(None, Some("foo"), Tag::Insert),
        ];
        let (hits, capped) = search(&rows, "foo", false);
        assert!(!capped);
        assert_eq!(
            hits.iter().map(|h| (h.row, h.right)).collect::<Vec<_>>(),
            vec![(1, false), (1, true), (2, true)]
        );
        assert_eq!(search(&rows, "foo", true).0.len(), 2);
        assert_eq!(search(&rows, "été", false).0.len(), 1);
        assert!(search(&rows, "a.c(", false).0.is_empty());
    }
}
