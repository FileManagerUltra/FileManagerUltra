use std::sync::Arc;
use std::time::{Duration, Instant};

use fm_preview::Preview;
use fm_preview::table::query::{Condition, Op, SortKey};
use fm_preview::table::{TableOptions, render};
use fm_testkit::MemFs;
use fm_vfs::{Vfs, VfsUri};

use super::*;

pub(crate) fn open_table(csv: &[u8], memory_cap: u64) -> TableView {
    let fs = MemFs::new();
    fs.mkdirs_str("/t").expect("dir");
    fs.put_str("/t/data.csv", csv).expect("write");
    let uri = VfsUri::parse("/t/data.csv").expect("uri");
    let entry = fs.stat(&uri).expect("stat");
    let Ok(Preview::Table(doc)) = render(
        &fs,
        &entry,
        &fm_preview::Limits::default(),
        TableOptions::default(),
    ) else {
        panic!("not a table");
    };
    let vfs: Arc<dyn Vfs> = Arc::new(fs);
    TableView::open_capped(vfs, entry, *doc, Waker::default(), memory_cap)
}

pub(crate) fn wait(view: &TableView, mut done: impl FnMut(&TableView) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if done(view) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// All rows of the current view, waiting for the reader.
fn all_rows(view: &TableView) -> Vec<(u64, Vec<String>)> {
    let n = view.row_count();
    let mut got = Vec::new();
    assert!(wait(view, |v| {
        let rows = v.rows(0, n as usize);
        if rows.iter().all(Option::is_some) {
            got = rows
                .into_iter()
                .flatten()
                .map(|(r, row)| (r, row.cells.iter().map(|c| c.to_string()).collect()))
                .collect();
            true
        } else {
            false
        }
    }));
    got
}

fn big_csv(rows: usize) -> Vec<u8> {
    let mut data = b"id,name,price\n".to_vec();
    for i in 0..rows {
        data.extend_from_slice(format!("{i},\"item\n{}\",{}\n", i % 7, (i * 37) % 1000).as_bytes());
    }
    data
}

#[test]
fn rows_beyond_the_sample_are_read_through_the_index() {
    let view = open_table(&big_csv(5_000), MEMORY_BYTES);
    assert!(wait(&view, |v| v.indexing().is_none()));
    assert_eq!(view.total_rows(), 5_000);
    assert_eq!(view.columns(), 3);
    let row = 4_321;
    let mut got = None;
    assert!(wait(&view, |v| {
        got = v.row(row);
        got.is_some()
    }));
    let (n, data) = got.expect("row");
    assert_eq!(n, row);
    assert_eq!(data.cell(0), "4321");
    assert_eq!(data.cell(1), format!("item\n{}", 4321 % 7));
}

#[test]
fn large_files_are_read_in_blocks() {
    // A memory cap below the file size forces the block source.
    let csv = big_csv(3_000);
    let view = open_table(&csv, 4096);
    assert!(wait(&view, |v| v.indexing().is_none()));
    assert_eq!(view.total_rows(), 3_000);
    assert!(view.note().is_none());
    let mut got = None;
    assert!(wait(&view, |v| {
        got = v.row(2_999);
        got.is_some()
    }));
    assert_eq!(got.expect("row").1.cell(0), "2999");
}

#[test]
fn sort_and_filter_produce_a_view_with_original_row_numbers() {
    let mut view = open_table(b"name,price\nb,10\na,9.5\nc,\nd,100\n", MEMORY_BYTES);
    view.sort = vec![SortKey {
        column: 1,
        descending: false,
    }];
    view.refresh_view();
    assert!(wait(&view, |v| v.view_status() == ViewStatus::Ready));
    let rows = all_rows(&view);
    let order: Vec<u64> = rows.iter().map(|r| r.0).collect();
    assert_eq!(order, vec![1, 0, 3, 2]);
    view.filter.conditions.push(Condition {
        column: 1,
        op: Op::Greater,
        value: "9.9".into(),
    });
    view.refresh_view();
    assert!(wait(&view, |v| v.view_status() == ViewStatus::Ready));
    let names: Vec<String> = all_rows(&view)
        .into_iter()
        .map(|r| r.1[0].clone())
        .collect();
    assert_eq!(names, vec!["b", "d"]);
    // Clearing both returns to file order without a job.
    view.sort.clear();
    view.filter = Default::default();
    view.refresh_view();
    assert_eq!(view.view_status(), ViewStatus::Natural);
    assert_eq!(view.row_count(), 4);
}

#[test]
fn a_failed_filter_keeps_the_previous_rows() {
    let mut view = open_table(b"a\nx\ny\n", MEMORY_BYTES);
    view.filter.query = "x".into();
    view.refresh_view();
    assert!(wait(&view, |v| v.view_status() == ViewStatus::Ready));
    view.filter.query = "([".into();
    view.filter.regex = true;
    view.refresh_view();
    assert!(wait(&view, |v| matches!(
        v.view_status(),
        ViewStatus::Failed(_)
    )));
    assert_eq!(view.row_count(), 1);
}

#[test]
fn dropping_the_view_ends_its_threads() {
    let view = open_table(&big_csv(20_000), 4096);
    let alive = view.alive_probe();
    drop(view);
    let end = Instant::now() + Duration::from_secs(10);
    while alive() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive(), "threads still hold the table");
}

#[test]
fn rows_arrive_without_blocking_the_caller() {
    let view = open_table(&big_csv(50_000), 4096);
    let start = Instant::now();
    let rows = view.rows(40_000, 50);
    // Rows far past the sample are fetched only on request, so a caller that did not wait
    // for I/O gets placeholders back (a wall-clock limit alone was flaky on busy CI).
    assert!(
        rows.iter().all(Option::is_none),
        "rows() read the file itself instead of requesting the rows"
    );
    assert!(start.elapsed() < Duration::from_secs(2), "rows() blocked");
    // The request is then served in the background.
    assert!(wait(&view, |v| v
        .rows(40_000, 50)
        .iter()
        .all(Option::is_some)));
}
