use fm_testkit::MemFs;
use fm_vfs::{Entry, Vfs, VfsUri};

use super::*;
use crate::Limits;

fn stat(fs: &MemFs, path: &str) -> Entry {
    fs.stat(&VfsUri::parse(path).expect("uri")).expect("stat")
}

fn monolog(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!(
            "[2024-05-01 10:{:02}:{:02}] main.INFO: line {i}\n",
            i / 60 % 60,
            i % 60
        ));
        if i % 10 == 0 {
            s.push_str(&format!("[2024-05-01 10:{:02}:{:02}] main.ERROR: boom {i}\n#0 /a.php(1): f()\n#1 {{main}}\n", i / 60 % 60, i % 60));
        }
    }
    s
}

fn small() -> LogLimits {
    LogLimits {
        initial_tail: 2000,
        chunk: 2000,
        max_bytes: 1 << 30,
        ..LogLimits::default()
    }
}

#[test]
fn stack_traces_fold_into_their_record() {
    let fs = MemFs::new();
    fs.put_str("/a.log", monolog(30).as_bytes()).expect("put");
    let doc = open_doc_with(
        &fs,
        &stat(&fs, "/a.log"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    assert_eq!(doc.format, LogFormat::Monolog);
    assert_eq!(doc.records.len(), 33);
    let err = doc
        .records
        .iter()
        .find(|r| r.level == Some(Level::Error))
        .expect("error");
    assert_eq!(err.extra_lines(), 2);
    assert!(!doc.more_before);
}

#[test]
fn tail_first_then_earlier_chunks_cover_everything_once() {
    let fs = MemFs::new();
    fs.put_str("/a.log", monolog(400).as_bytes()).expect("put");
    let e = stat(&fs, "/a.log");
    let lim = small();
    let mut doc = open_doc_with(&fs, &e, &Limits::default(), &lim).expect("doc");
    assert!(doc.more_before);
    let tail_len = doc.records.len();
    assert!(tail_len < 440);
    assert!(doc.records.last().expect("last").text.contains("line 399"));
    while doc.more_before {
        doc = load::load_earlier(&fs, &e, &doc, &lim).expect("earlier");
    }
    assert_eq!(doc.records.len(), 440);
    assert!(doc.records.first().expect("first").text.contains("line 0"));
    assert!(doc.records.iter().all(|r| !r.orphan));
}

#[test]
fn markers_keep_their_place_when_the_window_changes() {
    let fs = MemFs::new();
    fs.put_str("/a.log", monolog(400).as_bytes()).expect("put");
    let e = stat(&fs, "/a.log");
    let lim = small();
    let mut doc = open_doc_with(&fs, &e, &Limits::default(), &lim).expect("doc");
    doc.mark_here();
    doc.mark_here();
    assert_eq!(doc.marks.len(), 1, "same spot is marked once");
    let after_last = doc.base + doc.records.len() as i64;
    let tail_text = doc.records.last().expect("last").text.clone();
    while doc.more_before {
        doc = load::load_earlier(&fs, &e, &doc, &lim).expect("earlier");
    }
    assert_eq!(doc.marks, vec![after_last]);
    assert_eq!(doc.base + doc.records.len() as i64, after_last);
    assert_eq!(doc.records.last().expect("last").text, tail_text);
    assert!(doc.base <= 0);
    for _ in 0..(LogDoc::MAX_MARKS + 10) {
        doc.records.push(doc.records[0].clone());
        doc.mark_here();
    }
    assert!(doc.marks.len() <= LogDoc::MAX_MARKS);
}

#[test]
fn filters_level_text_regex_and_time() {
    let fs = MemFs::new();
    fs.put_str("/a.log", monolog(30).as_bytes()).expect("put");
    let doc = open_doc_with(
        &fs,
        &stat(&fs, "/a.log"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    let errors = apply_filter(
        &doc,
        &LogFilter {
            min_level: Some(Level::Error),
            ..Default::default()
        },
    )
    .expect("f");
    assert_eq!(errors.len(), 3);
    let text = apply_filter(
        &doc,
        &LogFilter {
            query: "LINE 7".into(),
            ..Default::default()
        },
    )
    .expect("f");
    assert_eq!(text.len(), 1);
    let re = apply_filter(
        &doc,
        &LogFilter {
            query: r"(?m)boom \d*0$".into(),
            regex: true,
            ..Default::default()
        },
    )
    .expect("f");
    assert_eq!(re.len(), 3);
    let from = parse_bound("2024-05-01 10:00:20").expect("bound");
    let t = apply_filter(
        &doc,
        &LogFilter {
            from: Some(from),
            ..Default::default()
        },
    )
    .expect("f");
    assert!(t.len() < doc.records.len() && !t.is_empty());
    assert!(
        apply_filter(
            &doc,
            &LogFilter {
                query: "(".into(),
                regex: true,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn hostile_regex_is_bounded() {
    let fs = MemFs::new();
    fs.put_str("/a.log", monolog(5).as_bytes()).expect("put");
    let doc = open_doc_with(
        &fs,
        &stat(&fs, "/a.log"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    let huge = format!("({}){{1000}}", "a".repeat(5000));
    assert!(
        apply_filter(
            &doc,
            &LogFilter {
                query: huge,
                regex: true,
                ..Default::default()
            }
        )
        .is_err()
    );
    let evil = apply_filter(
        &doc,
        &LogFilter {
            query: "(a+)+$".into(),
            regex: true,
            ..Default::default()
        },
    );
    assert!(evil.is_ok());
}

#[test]
fn follow_appends_continues_and_survives_rotation() {
    let fs = MemFs::new();
    fs.put_str("/a.log", monolog(3).as_bytes()).expect("put");
    let lim = LogLimits::default();
    let limits = Limits::default();
    let mut doc = open_doc_with(&fs, &stat(&fs, "/a.log"), &limits, &lim).expect("doc");
    assert_eq!(
        follow_poll(&fs, &stat(&fs, "/a.log"), &doc, &limits, &lim).expect("p"),
        FollowUpdate::Unchanged
    );
    let mut grown = monolog(3);
    grown.push_str("#2 more trace\n[2024-05-01 11:00:00] main.WARNING: new\n");
    fs.put_str("/a.log", grown.as_bytes()).expect("put");
    let before = doc.records.len();
    match follow_poll(&fs, &stat(&fs, "/a.log"), &doc, &limits, &lim).expect("p") {
        FollowUpdate::Appended {
            records,
            end_offset,
        } => doc.append(records, end_offset, &lim),
        other => panic!("{other:?}"),
    }
    assert_eq!(doc.records.len(), before + 1);
    assert!(doc.records[before - 1].text.contains("#2 more trace"));
    assert_eq!(doc.records.last().expect("l").level, Some(Level::Warning));
    fs.put_str("/a.log", b"[2024-06-01 00:00:00] main.INFO: fresh\n")
        .expect("put");
    match follow_poll(&fs, &stat(&fs, "/a.log"), &doc, &limits, &lim).expect("p") {
        FollowUpdate::Reloaded(d) => assert_eq!(d.records.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn gz_and_report_and_formats() {
    use std::io::Write;
    let fs = MemFs::new();
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(monolog(5).as_bytes()).expect("gz");
    fs.put_str("/a.log.1.gz", &enc.finish().expect("fin"))
        .expect("put");
    let doc = open_doc_with(
        &fs,
        &stat(&fs, "/a.log.1.gz"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    assert!(doc.gz && !doc.can_follow());
    assert_eq!(doc.records.len(), 6);

    fs.mkdirs_str("/srv/var/report").expect("dir");
    fs.put_str(
        "/srv/var/report/123",
        br##"{"0":"Boom","1":"#0 trace","url":"/x"}"##,
    )
    .expect("put");
    let r = open_doc_with(
        &fs,
        &stat(&fs, "/srv/var/report/123"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    assert_eq!(r.format, LogFormat::MagentoReport);
    assert!(r.records[0].text.starts_with("Boom\n#0 trace"));

    let access = r#"1.2.3.4 - - [10/Oct/2000:13:55:36 +0000] "GET /a HTTP/1.1" 404 12 "-" "ua""#;
    fs.mkdirs_str("/var/log/apache2").expect("dir");
    fs.put_str(
        "/var/log/apache2/access.log",
        format!("{access}\n{access}\n").as_bytes(),
    )
    .expect("put");
    let a = open_doc_with(
        &fs,
        &stat(&fs, "/var/log/apache2/access.log"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    assert_eq!(a.format, LogFormat::ApacheAccess);
    assert_eq!(a.records[0].status, Some(404));
    assert_eq!(a.records[0].level, Some(Level::Warning));
}

#[test]
fn garbage_and_giant_lines_do_not_break_loading() {
    let fs = MemFs::new();
    let mut data = vec![0xffu8; 300_000];
    data.extend_from_slice(b"\nplain ERROR text\n");
    fs.put_str("/g.log", &data).expect("put");
    let doc = open_doc_with(
        &fs,
        &stat(&fs, "/g.log"),
        &Limits::default(),
        &LogLimits::default(),
    )
    .expect("doc");
    assert_eq!(doc.format, LogFormat::Generic);
    assert!(doc.records[0].text.len() <= 3 * 64 * 1024);
    assert!(doc.records.iter().any(|r| r.level == Some(Level::Error)));
}
