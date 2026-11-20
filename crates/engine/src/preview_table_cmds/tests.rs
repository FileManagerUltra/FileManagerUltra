use fm_layout::PaneKind;
use fm_vfs::{Entry, EntryKind, VfsUri};
use serde_json::json;

use crate::preview_cmds::{Body, PreviewSlot};
use crate::preview_table::tests::{open_table, wait};
use crate::preview_table::{TableView, ViewStatus};
use crate::{Session, preview_table};
use fm_layout::PaneId;

fn session(csv: &[u8]) -> (Session, PaneId) {
    let mut s = Session::new().expect("session");
    let pane = s.layout.focused();
    s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
    let slot = s
        .previews
        .slots
        .entry(pane)
        .or_insert_with(PreviewSlot::new);
    slot.subject = Some(Entry::new(
        VfsUri::parse("/t/data.csv").expect("uri"),
        EntryKind::File,
    ));
    slot.body = Body::Table(Box::new(open_table(
        csv,
        preview_table::CACHE_ROWS as u64 * 1024,
    )));
    (s, pane)
}

fn view(s: &Session, pane: PaneId) -> &TableView {
    s.preview_slot(pane)
        .and_then(|slot| slot.table())
        .expect("table")
}

const RATES: &[u8] = b"Code,Country,Rate\nNL-high,NL,21\nNL-low,NL,9\nBE,BE,21\nDE,DE,19\n";

#[test]
fn sort_cycles_and_adds_keys() {
    let (mut s, pane) = session(RATES);
    assert!(s.command_enabled("preview.table.sort"));
    s.dispatch("preview.table.sort", &json!({"column": 2, "order": "desc"}))
        .expect("sort");
    assert_eq!(view(&s, pane).sort.len(), 1);
    s.dispatch(
        "preview.table.sort",
        &json!({"column": 0, "order": "asc", "add": true}),
    )
    .expect("add");
    assert_eq!(view(&s, pane).sort.len(), 2);
    assert!(wait(view(&s, pane), |v| v.view_status() == ViewStatus::Ready));
    s.dispatch(
        "preview.table.sort",
        &json!({"column": 2, "order": "none", "add": true}),
    )
    .expect("remove");
    assert_eq!(view(&s, pane).sort.len(), 1);
    s.dispatch("preview.table.sort", &json!({"column": 0, "order": "none"}))
        .expect("clear");
    assert!(view(&s, pane).sort.is_empty());
}

#[test]
fn filters_conditions_and_clear() {
    let (mut s, pane) = session(RATES);
    s.dispatch("preview.table.filter", &json!({"query": "nl", "column": 1}))
        .expect("filter");
    assert!(wait(view(&s, pane), |v| v.view_status() == ViewStatus::Ready));
    assert_eq!(view(&s, pane).row_count(), 2);
    s.dispatch(
        "preview.table.condition.add",
        &json!({"column": 2, "op": "greater", "value": "20"}),
    )
    .expect("condition");
    assert!(wait(view(&s, pane), |v| v.view_status()
        == ViewStatus::Ready
        && v.row_count() == 1));
    s.dispatch("preview.table.condition.remove", &json!({"index": 0}))
        .expect("remove");
    assert!(wait(view(&s, pane), |v| v.view_status()
        == ViewStatus::Ready
        && v.row_count() == 2));
    s.dispatch("preview.table.filter.clear", &json!({}))
        .expect("clear");
    assert_eq!(view(&s, pane).row_count(), 4);
}

#[test]
fn select_copy_goto_hide_and_record() {
    let (mut s, pane) = session(RATES);
    s.dispatch("preview.table.select", &json!({"row": 1, "column": 0}))
        .expect("select");
    s.dispatch("preview.table.copy", &json!({"what": "cell"}))
        .expect("copy");
    assert_eq!(s.clipboard_out.take().as_deref(), Some("NL-low"));
    s.dispatch("preview.table.copy", &json!({"what": "row"}))
        .expect("copy row");
    assert_eq!(s.clipboard_out.take().as_deref(), Some("NL-low\tNL\t9"));
    s.dispatch("preview.table.column.hide", &json!({"column": 1}))
        .expect("hide");
    s.dispatch("preview.table.copy", &json!({"what": "header"}))
        .expect("copy header");
    assert_eq!(s.clipboard_out.take().as_deref(), Some("Code\tRate"));
    s.dispatch("preview.table.columns.show_all", &json!({}))
        .expect("show");
    assert!(view(&s, pane).hidden.is_empty());
    s.dispatch("preview.table.goto", &json!({"row": 4}))
        .expect("goto");
    assert_eq!(view(&s, pane).selected, Some((3, 0)));
    assert_eq!(view(&s, pane).scroll_to.map(|(r, _)| r), Some(3));
    s.dispatch("preview.table.record", &json!({}))
        .expect("record");
    assert!(view(&s, pane).record);
    s.dispatch("preview.table.record", &json!({"show": false}))
        .expect("record off");
    assert!(!view(&s, pane).record);
}

#[test]
fn the_last_visible_column_cannot_be_hidden() {
    let (mut s, _) = session(b"a,b\n1,2\n");
    s.dispatch("preview.table.column.hide", &json!({"column": 0}))
        .expect("hide");
    assert!(
        s.dispatch("preview.table.column.hide", &json!({"column": 1}))
            .is_err()
    );
}

#[test]
fn dialect_reads_the_table_again() {
    let (mut s, pane) = session(b"a;b\n1;2\n");
    s.dispatch(
        "preview.table.dialect",
        &json!({"delimiter": "comma", "header": "no"}),
    )
    .expect("dialect");
    assert!(matches!(
        s.preview_slot(pane).map(|slot| &slot.body),
        Some(Body::Loading)
    ));
}

#[test]
fn bad_parameters_change_nothing() {
    let (mut s, pane) = session(RATES);
    let cases = [
        ("preview.table.sort", json!({"column": 3, "order": "asc"})),
        ("preview.table.sort", json!({"column": 0, "order": "up"})),
        ("preview.table.sort", json!({"column": -1, "order": "asc"})),
        ("preview.table.sort", json!({"column": 0})),
        (
            "preview.table.sort",
            json!({"column": 0, "order": "asc", "extra": 1}),
        ),
        ("preview.table.filter", json!({"query": 5})),
        ("preview.table.filter", json!({"query": "x".repeat(5000)})),
        (
            "preview.table.filter",
            json!({"query": "([", "regex": true}),
        ),
        ("preview.table.filter", json!({"query": "a", "column": 99})),
        (
            "preview.table.condition.add",
            json!({"column": 0, "op": "like", "value": "a"}),
        ),
        (
            "preview.table.condition.add",
            json!({"column": 0, "op": "equals"}),
        ),
        (
            "preview.table.condition.add",
            json!({"column": 9, "op": "empty"}),
        ),
        ("preview.table.condition.remove", json!({"index": 0})),
        ("preview.table.dialect", json!({"delimiter": ":"})),
        ("preview.table.dialect", json!({"header": true})),
        ("preview.table.select", json!({"row": 99, "column": 0})),
        ("preview.table.select", json!({"row": 0, "column": 99})),
        ("preview.table.copy", json!({"what": "everything"})),
        ("preview.table.copy", json!({"what": "cell"})),
        ("preview.table.goto", json!({"row": 0})),
        ("preview.table.goto", json!({"row": 99})),
        ("preview.table.column.hide", json!({"column": 99})),
        ("preview.table.record", json!({"show": "yes"})),
        ("preview.table.columns.show_all", json!([1, 2])),
        ("preview.table.filter.clear", json!("x")),
    ];
    for (id, params) in cases {
        assert!(s.dispatch(id, &params).is_err(), "{id} {params}");
    }
    let v = view(&s, pane);
    assert!(v.sort.is_empty() && v.filter.is_empty() && v.selected.is_none());
    assert!(v.hidden.is_empty() && !v.record);
    // Fifteen conditions are fine, the seventeenth is refused.
    for i in 0..16 {
        s.dispatch(
            "preview.table.condition.add",
            &json!({"column": 0, "op": "not_contains", "value": format!("v{i}")}),
        )
        .expect("condition");
    }
    assert!(
        s.dispatch(
            "preview.table.condition.add",
            &json!({"column": 0, "op": "not_contains", "value": "more"}),
        )
        .is_err()
    );
}

#[test]
fn commands_need_a_table() {
    let mut s = Session::new().expect("session");
    let pane = s.layout.focused();
    s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
    assert!(!s.command_enabled("preview.table.sort"));
    assert!(
        s.dispatch("preview.table.sort", &json!({"column": 0, "order": "asc"}))
            .is_err()
    );
}
