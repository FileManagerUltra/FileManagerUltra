//! `preview.table.*` commands (change `add-table-preview`): sorting, filtering, dialect
//! overrides, selection, copying, Go to row, hidden columns and the Record panel. Every
//! parameter is checked before the view changes.

use fm_layout::PaneId;
use fm_preview::table::TableOptions;
use fm_preview::table::query::{Condition, MAX_CONDITIONS, MAX_QUERY, MAX_SORT_KEYS, Op, SortKey};
use fm_vfs::FmError;
use serde_json::Map;

use crate::preview_table::TableView;
use crate::{CommandSpec, Params, Session};

fn bad(message: impl Into<String>) -> FmError {
    FmError::InvalidParams(message.into())
}

/// The params object, refusing keys outside `allowed` (plus `pane`).
fn object<'a>(params: &'a Params, allowed: &[&str]) -> Result<&'a Map<String, Params>, FmError> {
    static EMPTY: std::sync::OnceLock<Map<String, Params>> = std::sync::OnceLock::new();
    let map = match params {
        Params::Null => EMPTY.get_or_init(Map::new),
        Params::Object(m) => m,
        _ => return Err(bad("expected an object")),
    };
    if let Some(k) = map
        .keys()
        .find(|k| k.as_str() != "pane" && !allowed.contains(&k.as_str()))
    {
        return Err(bad(format!("unknown parameter {k}")));
    }
    if map.get("pane").is_some_and(|p| p.as_u64().is_none()) {
        return Err(bad("pane must be a non-negative integer"));
    }
    Ok(map)
}

fn uint(map: &Map<String, Params>, key: &str) -> Result<Option<u64>, FmError> {
    match map.get(key) {
        None | Some(Params::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| bad(format!("{key} must be a non-negative integer"))),
    }
}

fn required_uint(map: &Map<String, Params>, key: &str) -> Result<u64, FmError> {
    uint(map, key)?.ok_or_else(|| bad(format!("{key} is required")))
}

fn text<'a>(map: &'a Map<String, Params>, key: &str) -> Result<Option<&'a str>, FmError> {
    match map.get(key) {
        None | Some(Params::Null) => Ok(None),
        Some(Params::String(s)) if s.len() <= MAX_QUERY => Ok(Some(s)),
        Some(Params::String(_)) => Err(bad(format!("{key} is longer than {MAX_QUERY} bytes"))),
        Some(_) => Err(bad(format!("{key} must be a string"))),
    }
}

fn flag(map: &Map<String, Params>, key: &str) -> Result<Option<bool>, FmError> {
    match map.get(key) {
        None | Some(Params::Null) => Ok(None),
        Some(Params::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(bad(format!("{key} must be true or false"))),
    }
}

fn no_table() -> FmError {
    bad("the preview is not showing a table")
}

fn table_pane(s: &Session, params: &Params) -> Result<PaneId, FmError> {
    let pane = s.preview_pane(params)?;
    s.preview_slot(pane)
        .and_then(|slot| slot.table())
        .map(|_| pane)
        .ok_or_else(no_table)
}

fn view_mut(s: &mut Session, pane: PaneId) -> Result<&mut TableView, FmError> {
    s.previews
        .slots
        .get_mut(&pane)
        .and_then(|slot| slot.table_mut())
        .ok_or_else(no_table)
}

fn column(view: &TableView, map: &Map<String, Params>, key: &str) -> Result<usize, FmError> {
    let c = required_uint(map, key)?;
    if c >= view.columns() as u64 {
        return Err(bad(format!("{key} is beyond the last column")));
    }
    Ok(c as usize)
}

fn has_table(s: &Session) -> bool {
    s.previews.slots.values().any(|slot| slot.table().is_some())
}

fn sort(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["column", "order", "add"])?;
    let pane = table_pane(s, params)?;
    let order = text(map, "order")?.ok_or_else(|| bad("order is required"))?;
    let descending = match order {
        "asc" => Some(false),
        "desc" => Some(true),
        "none" => None,
        _ => return Err(bad("order must be asc, desc or none")),
    };
    let add = flag(map, "add")?.unwrap_or(false);
    let view = view_mut(s, pane)?;
    let column = column(view, map, "column")?;
    let mut keys = if add { view.sort.clone() } else { Vec::new() };
    match descending {
        Some(descending) => match keys.iter().position(|k| k.column == column) {
            Some(i) => {
                if let Some(k) = keys.get_mut(i) {
                    k.descending = descending;
                }
            }
            None if keys.len() >= MAX_SORT_KEYS => {
                return Err(bad(format!("sort by at most {MAX_SORT_KEYS} columns")));
            }
            None => keys.push(SortKey { column, descending }),
        },
        None => keys.retain(|k| k.column != column),
    }
    view.sort = keys;
    view.refresh_view();
    Ok(())
}

fn filter(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["query", "regex", "column"])?;
    let pane = table_pane(s, params)?;
    let query = text(map, "query")?
        .ok_or_else(|| bad("query is required"))?
        .to_string();
    let regex = flag(map, "regex")?.unwrap_or(false);
    let view = view_mut(s, pane)?;
    let col = match map.get("column") {
        None | Some(Params::Null) => None,
        Some(_) => Some(column(view, map, "column")?),
    };
    if regex && let Some(e) = pattern_error(&query) {
        // Refuse a bad pattern here, so the command reports it and nothing changes.
        return Err(bad(e));
    }
    view.filter.query = query;
    view.filter.regex = regex;
    view.filter.column = col;
    view.refresh_view();
    Ok(())
}

/// Why `query` is not a usable regular expression, if it is not (the filter bar checks
/// while typing, so a half-typed pattern is not reported as an error).
pub fn pattern_error(query: &str) -> Option<String> {
    regex::bytes::RegexBuilder::new(query)
        .size_limit(1 << 20)
        .dfa_size_limit(4 << 20)
        .build()
        .err()
        .map(|e| {
            format!(
                "invalid pattern: {}",
                e.to_string().lines().last().unwrap_or("")
            )
        })
}

fn condition_add(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["column", "op", "value"])?;
    let pane = table_pane(s, params)?;
    let op = text(map, "op")?
        .and_then(Op::parse)
        .ok_or_else(|| bad("op must be contains, not_contains, equals, not_equals, greater, less, empty or not_empty"))?;
    let value = text(map, "value")?.unwrap_or("").to_string();
    if op.takes_value() && value.is_empty() {
        return Err(bad("value is required for this operator"));
    }
    let view = view_mut(s, pane)?;
    let column = column(view, map, "column")?;
    if view.filter.conditions.len() >= MAX_CONDITIONS {
        return Err(bad(format!("at most {MAX_CONDITIONS} conditions")));
    }
    let condition = Condition { column, op, value };
    if !view.filter.conditions.contains(&condition) {
        view.filter.conditions.push(condition);
        view.refresh_view();
    }
    Ok(())
}

fn condition_remove(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["index"])?;
    let pane = table_pane(s, params)?;
    let index = required_uint(map, "index")?;
    let view = view_mut(s, pane)?;
    if index >= view.filter.conditions.len() as u64 {
        return Err(bad("no condition with that index"));
    }
    view.filter.conditions.remove(index as usize);
    view.refresh_view();
    Ok(())
}

fn filter_clear(s: &mut Session, params: &Params) -> Result<(), FmError> {
    object(params, &[])?;
    let pane = table_pane(s, params)?;
    let view = view_mut(s, pane)?;
    view.filter = Default::default();
    view.refresh_view();
    Ok(())
}

fn dialect(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["delimiter", "header"])?;
    let pane = table_pane(s, params)?;
    let current = s
        .preview_slot(pane)
        .and_then(|slot| slot.table())
        .map(|t| t.doc.options)
        .unwrap_or_default();
    let delimiter = match text(map, "delimiter")? {
        None => current.delimiter,
        Some("auto") => None,
        Some("comma") => Some(b','),
        Some("semicolon") => Some(b';'),
        Some("tab") => Some(b'\t'),
        Some("pipe") => Some(b'|'),
        Some(_) => return Err(bad("delimiter must be auto, comma, semicolon, tab or pipe")),
    };
    let header = match text(map, "header")? {
        None => current.header,
        Some("auto") => None,
        Some("yes") => Some(true),
        Some("no") => Some(false),
        Some(_) => return Err(bad("header must be auto, yes or no")),
    };
    s.rerender_table(pane, TableOptions { delimiter, header })
}

fn select(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["row", "column"])?;
    let pane = table_pane(s, params)?;
    let row = required_uint(map, "row")?;
    let view = view_mut(s, pane)?;
    let column = column(view, map, "column")?;
    if row >= view.row_count() {
        return Err(bad("row is beyond the last row"));
    }
    view.selected = Some((row, column));
    view.scroll_to(row);
    Ok(())
}

fn goto(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["row"])?;
    let pane = table_pane(s, params)?;
    let row = required_uint(map, "row")?;
    let view = view_mut(s, pane)?;
    if row == 0 || row > view.row_count() {
        return Err(bad(format!(
            "row must be between 1 and {}",
            view.row_count()
        )));
    }
    let column = view.selected.map_or(0, |(_, c)| c);
    view.selected = Some((row - 1, column));
    view.scroll_to(row - 1);
    Ok(())
}

/// Tab-separated text for a clipboard: tabs and line breaks inside a value become spaces.
fn tsv_value(v: &str) -> String {
    v.replace(['\t', '\n', '\r'], " ")
}

fn copy(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["what"])?;
    let pane = table_pane(s, params)?;
    let what = text(map, "what")?.unwrap_or("cell");
    let view = s
        .preview_slot(pane)
        .and_then(|slot| slot.table())
        .ok_or_else(no_table)?;
    let visible: Vec<usize> = (0..view.columns())
        .filter(|c| !view.hidden.contains(c))
        .collect();
    let out = match what {
        "header" => visible
            .iter()
            .map(|c| tsv_value(&view.doc.column_name(*c)))
            .collect::<Vec<_>>()
            .join("\t"),
        "cell" | "row" => {
            let (row, col) = view.selected.ok_or_else(|| bad("no cell is selected"))?;
            let (_, data) = view
                .row(row)
                .ok_or_else(|| bad("the row is still being read; try again"))?;
            if what == "cell" {
                data.cell(col).to_string()
            } else {
                visible
                    .iter()
                    .map(|c| tsv_value(data.cell(*c)))
                    .collect::<Vec<_>>()
                    .join("\t")
            }
        }
        _ => return Err(bad("what must be cell, row or header")),
    };
    s.clipboard_out = Some(out);
    Ok(())
}

fn column_hide(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["column", "hidden"])?;
    let pane = table_pane(s, params)?;
    let hidden = flag(map, "hidden")?.unwrap_or(true);
    let view = view_mut(s, pane)?;
    let column = column(view, map, "column")?;
    if hidden {
        if view.hidden.len() + 1 >= view.columns() && !view.hidden.contains(&column) {
            return Err(bad("at least one column stays visible"));
        }
        view.hidden.insert(column);
    } else {
        view.hidden.remove(&column);
    }
    Ok(())
}

fn show_all(s: &mut Session, params: &Params) -> Result<(), FmError> {
    object(params, &[])?;
    let pane = table_pane(s, params)?;
    view_mut(s, pane)?.hidden.clear();
    Ok(())
}

fn record(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params, &["show"])?;
    let pane = table_pane(s, params)?;
    let show = flag(map, "show")?;
    let view = view_mut(s, pane)?;
    view.record = show.unwrap_or(!view.record);
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "preview.table.sort",
            label: "Table: Sort by Column",
            menu: &["View", "Table", "Sort by Column"],
            shortcut: None,
            params: "{column: number, order: \"asc\"|\"desc\"|\"none\", add?: bool, pane?: number}: sort the shown table; add keeps the other sort columns (up to 3)",
            enabled: has_table,
            run: sort,
        },
        CommandSpec {
            id: "preview.table.filter",
            label: "Table: Quick Filter",
            menu: &["View", "Table", "Quick Filter"],
            shortcut: None,
            params: "{query: string, regex?: bool, column?: number|null, pane?: number}: keep rows containing (or matching) the text, in one column or any",
            enabled: has_table,
            run: filter,
        },
        CommandSpec {
            id: "preview.table.condition.add",
            label: "Table: Add Filter Condition",
            menu: &["View", "Table", "Add Filter Condition"],
            shortcut: None,
            params: "{column: number, op: \"contains\"|\"not_contains\"|\"equals\"|\"not_equals\"|\"greater\"|\"less\"|\"empty\"|\"not_empty\", value?: string, pane?: number}",
            enabled: has_table,
            run: condition_add,
        },
        CommandSpec {
            id: "preview.table.condition.remove",
            label: "Table: Remove Filter Condition",
            menu: &["View", "Table", "Remove Filter Condition"],
            shortcut: None,
            params: "{index: number, pane?: number}",
            enabled: has_table,
            run: condition_remove,
        },
        CommandSpec {
            id: "preview.table.filter.clear",
            label: "Table: Clear Filters",
            menu: &["View", "Table", "Clear Filters"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: has_table,
            run: filter_clear,
        },
        CommandSpec {
            id: "preview.table.dialect",
            label: "Table: Delimiter and Header",
            menu: &["View", "Table", "Delimiter and Header"],
            shortcut: None,
            params: "{delimiter?: \"auto\"|\"comma\"|\"semicolon\"|\"tab\"|\"pipe\", header?: \"auto\"|\"yes\"|\"no\", pane?: number}: read the shown table again with these choices",
            enabled: has_table,
            run: dialect,
        },
        CommandSpec {
            id: "preview.table.select",
            label: "Table: Select Cell",
            menu: &["View", "Table", "Select Cell"],
            shortcut: None,
            params: "{row: number, column: number, pane?: number}: row is the position in the shown (filtered/sorted) rows, from 0",
            enabled: has_table,
            run: select,
        },
        CommandSpec {
            id: "preview.table.copy",
            label: "Table: Copy",
            menu: &["View", "Table", "Copy"],
            shortcut: None,
            params: "{what?: \"cell\"|\"row\"|\"header\", pane?: number}: copy the selected cell, its row (tab-separated) or the column names",
            enabled: has_table,
            run: copy,
        },
        CommandSpec {
            id: "preview.table.goto",
            label: "Table: Go to Row",
            menu: &["View", "Table", "Go to Row"],
            shortcut: None,
            params: "{row: number, pane?: number}: row number in the shown rows, from 1",
            enabled: has_table,
            run: goto,
        },
        CommandSpec {
            id: "preview.table.column.hide",
            label: "Table: Hide Column",
            menu: &["View", "Table", "Hide Column"],
            shortcut: None,
            params: "{column: number, hidden?: bool, pane?: number}",
            enabled: has_table,
            run: column_hide,
        },
        CommandSpec {
            id: "preview.table.columns.show_all",
            label: "Table: Show All Columns",
            menu: &["View", "Table", "Show All Columns"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: has_table,
            run: show_all,
        },
        CommandSpec {
            id: "preview.table.record",
            label: "Table: Record Panel",
            menu: &["View", "Table", "Record Panel"],
            shortcut: None,
            params: "{show?: bool, pane?: number}: show the selected row as name/value pairs (toggles without show)",
            enabled: has_table,
            run: record,
        },
    ]
}

#[cfg(test)]
mod tests;
