use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::index::{Index, STRIDE, build, read_rows};
use super::query::{Condition, Filter, Op, SortKey, ViewError, ViewSpec, compare, run_view};
use super::scan::{Fields, MAX_CELL, MAX_FIELDS, MAX_RECORD, scan_record};
use super::source::{ByteSource, MemSource, Reader};
use super::*;

const COMMA: Dialect = Dialect {
    delimiter: b',',
    quote: Some(b'"'),
};

fn fields_of(data: &[u8], d: &Dialect) -> Vec<Vec<String>> {
    let src = MemSource(Arc::from(data));
    let mut r = Reader::new(&src, *d, 0, 4096);
    let mut f = Fields::default();
    let mut out = Vec::new();
    while r.next(Some(&mut f)).expect("read").is_some() {
        out.push(
            f.iter()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .collect(),
        );
    }
    out
}

/// A source that is not in memory, read in tiny blocks, to exercise refills.
struct Chunky(Vec<u8>);

impl ByteSource for Chunky {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>, fm_vfs::FmError> {
        let start = (offset as usize).min(self.0.len());
        let end = (start + len.min(7)).min(self.0.len());
        Ok(self.0[start..end].to_vec())
    }
}

#[test]
fn quoted_newlines_html_and_escaped_quotes_stay_in_one_record() {
    let data = b"code,description,name\r\nroot,\"<style>\n#a{x:\"\"y\"\"}\n</style>\",\"BTN de Haas\"\r\nnext,,plain\n";
    let rows = fields_of(data, &COMMA);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1][1], "<style>\n#a{x:\"y\"}\n</style>");
    assert_eq!(rows[1][2], "BTN de Haas");
    assert_eq!(rows[2], vec!["next", "", "plain"]);
    // No carriage return survives at the end of a CRLF record.
    assert_eq!(rows[0][2], "name");
}

#[test]
fn a_stray_quote_inside_an_unquoted_field_is_text() {
    let data = b"id,name\n1,Screen 27\" wide\n2,next\n";
    let rows = fields_of(data, &COMMA);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1][1], "Screen 27\" wide");
    assert_eq!(rows[2], vec!["2", "next"]);
}

#[test]
fn block_reads_find_the_same_records_as_memory() {
    let mut data = Vec::new();
    for i in 0..200 {
        data.extend_from_slice(format!("{i},\"multi\nline {i}\",x\r\n").as_bytes());
    }
    let mem = MemSource(Arc::from(&data[..]));
    let chunky = Chunky(data.clone());
    let collect = |src: &dyn ByteSource| {
        let mut r = Reader::new(src, COMMA, 0, 16);
        let mut out = Vec::new();
        while let Some(f) = r.next(None).expect("read") {
            out.push((f.start, f.end));
        }
        out
    };
    let a = collect(&mem);
    assert_eq!(a.len(), 200);
    assert_eq!(a, collect(&chunky));
}

#[test]
fn unclosed_quote_is_cut_and_the_rest_still_reads() {
    let mut data = b"a,b\n1,\"never closed\n".to_vec();
    data.extend(std::iter::repeat_n(b'x', MAX_RECORD + 10));
    data.extend_from_slice(b"\n2,after\n3,more\n");
    let src = MemSource(Arc::from(&data[..]));
    let mut r = Reader::new(&src, COMMA, 0, 4096);
    let mut found = Vec::new();
    while let Some(f) = r.next(None).expect("read") {
        found.push(f);
    }
    assert!(found.iter().any(|f| f.malformed), "{found:?}");
    let rows = fields_of(&data, &COMMA);
    assert_eq!(rows.last().map(|r| r[1].as_str()), Some("more"));
    assert!(rows.len() >= 4, "{}", rows.len());
}

#[test]
fn caps_on_fields_and_cells() {
    let mut data = vec![b'y'; MAX_CELL + 100];
    data.push(b',');
    data.extend_from_slice("é".repeat(10).as_bytes());
    let mut f = Fields::default();
    scan_record(&data, 0, &COMMA, true, Some(&mut f)).expect("record");
    assert_eq!(f.get(0).len(), MAX_CELL);
    assert_eq!(f.cut, vec![0]);
    assert_eq!(f.get(1), "é".repeat(10).as_bytes());

    let wide = vec![b','; MAX_FIELDS + 50];
    let end = scan_record(&wide, 0, &COMMA, true, Some(&mut f)).expect("record");
    assert_eq!(f.len(), MAX_FIELDS);
    assert_eq!(end.fields, MAX_FIELDS + 51);
}

#[test]
fn cut_cells_keep_whole_characters() {
    let mut data = vec![b'a'; MAX_CELL - 1];
    data.extend_from_slice("€€".as_bytes());
    let mut f = Fields::default();
    scan_record(&data, 0, &COMMA, true, Some(&mut f)).expect("record");
    assert!(std::str::from_utf8(f.get(0)).is_ok());
}

#[test]
fn detects_delimiters_of_the_sample_shapes() {
    let semicolon =
        b"\"key\";\"sku\";\"price\"\r\n\"1_0\";\"1\";\"30.30\"\n\"2_0\";\"2\";\"1,5\"\r\n";
    assert_eq!(detect_delimiter(semicolon, true, "export.csv"), b';');
    let tsv = b"259216\tZandslurf 27x60cm | Groen\t0.0000\t50.0000\n110505\tKunstmest 12-10-18 | 20 kg.\t0.0000\t50.0000\n";
    assert_eq!(detect_delimiter(tsv, true, "query.tsv"), b'\t');
    assert_eq!(detect_delimiter(tsv, true, "query.txt"), b'\t');
    let comma = b"Name,Region,Resource ID\nus-east-1a,us-east-1,use1-az1\n";
    assert_eq!(detect_delimiter(comma, true, "x.csv"), b',');
    let pipe = b"a|b|c\n1|2|3\n";
    assert_eq!(detect_delimiter(pipe, true, "x.csv"), b'|');
}

#[test]
fn header_detection() {
    let parse = |data: &[u8], d: Dialect| {
        let mut out = Vec::new();
        let mut pos = 0;
        loop {
            let mut f = Fields::default();
            match scan_record(data, pos, &d, true, Some(&mut f)) {
                Some(e) if e.next > pos => {
                    out.push(f);
                    pos = e.next;
                }
                _ => break,
            }
        }
        out
    };
    let tab = Dialect {
        delimiter: b'\t',
        quote: Some(b'"'),
    };
    let tsv = parse(
        b"259216\tZandslurf\t0.0000\t50.0000\n110505\tKunstmest\t0.0000\t50.0000\n",
        tab,
    );
    assert!(!detect_header(&tsv, false));
    let ec2 = parse(
        b"Name,Region,Resource ID\nus-east-1a,us-east-1,use1-az1\n",
        COMMA,
    );
    assert!(detect_header(&ec2, false));
    let rates = parse(b"\"Code\",\"Rate\"\n\"US-CA\",\"8.2500\"\n", COMMA);
    assert!(detect_header(&rates, false));
}

#[test]
fn analysis_of_a_bom_semicolon_crlf_file() {
    let data = "\u{FEFF}Name;Price;Note\r\n\"Kroon-Oil\";\"30,31\";\"two\nlines\"\nBar;9,5;\r\n"
        .as_bytes();
    let (enc, bom) = encoding_of(data);
    assert_eq!((enc, bom), (Encoding::Utf8, 3));
    let a = analyze(data, bom, true, "export.csv", TableOptions::default());
    assert_eq!(a.dialect.delimiter, b';');
    assert!(a.header);
    assert_eq!(a.columns[0].name, "Name");
    assert_eq!(a.columns[1].kind, ColumnKind::Decimal);
    assert_eq!(a.sample.len(), 2);
    assert_eq!(a.sample[0].cell(2), "two\nlines");
    assert_eq!(a.sample[1].cell(2), "");
    assert!(a.sample_complete);
    // Data starts after the BOM and the header line.
    assert_eq!(a.data_start as usize, "\u{FEFF}Name;Price;Note\r\n".len());
}

#[test]
fn headerless_files_get_column_names_and_overrides_apply() {
    let data = b"1\tA\t0.5\n2\tB\t1.5\n";
    let a = analyze(data, 0, true, "q.tsv", TableOptions::default());
    assert!(!a.header);
    assert_eq!(a.columns[0].name, "Column 1");
    assert_eq!(a.sample.len(), 2);
    let forced = analyze(
        data,
        0,
        true,
        "q.tsv",
        TableOptions {
            delimiter: Some(b','),
            header: Some(true),
        },
    );
    assert_eq!(forced.dialect.delimiter, b',');
    assert!(forced.header);
    assert_eq!(forced.columns.len(), 1);
    assert_eq!(forced.sample.len(), 1);
}

#[test]
fn utf16_is_transcoded() {
    let mut raw = vec![0xFF, 0xFE];
    for u in "a\tb\n1\t2\n".encode_utf16() {
        raw.extend_from_slice(&u.to_le_bytes());
    }
    let (enc, bom) = encoding_of(&raw);
    assert_eq!(enc, Encoding::Utf16Le);
    assert_eq!(transcode_utf16(&raw[bom..], enc), b"a\tb\n1\t2\n");
}

#[test]
fn numbers() {
    assert_eq!(parse_number(b" 30.307692 ", false), Some(30.307692));
    assert_eq!(parse_number(b"30,31", true), Some(30.31));
    assert_eq!(parse_number(b"30,31", false), None);
    assert_eq!(parse_number(b"-1e3", false), Some(-1000.0));
    for bad in [&b"inf"[..], b"NaN", b"1.2.3", b"", b"12abc", b"--1", b"."] {
        assert_eq!(
            parse_number(bad, true),
            None,
            "{:?}",
            std::str::from_utf8(bad)
        );
    }
}

fn index_of(src: &dyn ByteSource, start: u64) -> Index {
    let mut idx = Index::default();
    build(src, COMMA, start, &AtomicBool::new(false), |d| idx.apply(d)).expect("index");
    idx
}

#[test]
fn index_locates_every_row() {
    let mut data = b"h1,h2\n".to_vec();
    let n = STRIDE * 5 + 7;
    for i in 0..n {
        data.extend_from_slice(format!("{i},\"v\n{i}\"\n").as_bytes());
    }
    let src = MemSource(Arc::from(&data[..]));
    let idx = index_of(&src, 6);
    assert!(idx.done);
    assert_eq!(idx.rows, n);
    assert_eq!(idx.max_fields, 2);
    for row in [0, 1, STRIDE - 1, STRIDE, STRIDE * 3 + 5, n - 1] {
        let (off, skip) = idx.locate(row).expect("indexed");
        let rows = read_rows(&src, COMMA, off, skip, 1).expect("read");
        assert_eq!(rows[0].1.cell(0), row.to_string());
    }
    assert!(idx.locate(n).is_none());
}

#[test]
fn index_stops_when_cancelled() {
    let data: Vec<u8> = (0..300_000)
        .flat_map(|i| format!("{i},x\n").into_bytes())
        .collect();
    let src = MemSource(Arc::from(&data[..]));
    let mut done = false;
    build(&src, COMMA, 0, &AtomicBool::new(true), |d| done |= d.done).expect("index");
    assert!(!done);
}

fn table(rows: &[&str]) -> MemSource {
    MemSource(Arc::from(rows.join("\n").into_bytes()))
}

fn run(src: &MemSource, spec: &ViewSpec) -> Result<Vec<u64>, ViewError> {
    run_view(src, COMMA, 0, spec, &|| false, &|_| {})
        .map(|v| v.into_iter().map(|(_, row)| row).collect())
}

#[test]
fn numeric_sort_puts_empty_last_both_ways() {
    let src = table(&["a,10", "b,9.5", "c,", "d,100", "e,n/a"]);
    let mut spec = ViewSpec {
        sort: vec![SortKey {
            column: 1,
            descending: false,
        }],
        numeric: vec![false, true],
        ..Default::default()
    };
    assert_eq!(run(&src, &spec).expect("sort"), vec![1, 0, 3, 4, 2]);
    spec.sort[0].descending = true;
    assert_eq!(run(&src, &spec).expect("sort"), vec![4, 3, 0, 1, 2]);
}

#[test]
fn text_sort_is_case_insensitive_and_stable_with_secondary_keys() {
    let src = table(&["b,2", "A,1", "a,3", "B,1"]);
    let spec = ViewSpec {
        sort: vec![
            SortKey {
                column: 0,
                descending: false,
            },
            SortKey {
                column: 1,
                descending: true,
            },
        ],
        numeric: vec![false, true],
        ..Default::default()
    };
    assert_eq!(run(&src, &spec).expect("sort"), vec![2, 1, 0, 3]);
}

#[test]
fn filters_quick_column_regex_and_conditions() {
    let src = table(&["NL,21", "nl,9", "BE,21", "DE,19", ",5"]);
    let quick = |query: &str, column: Option<usize>, regex: bool| ViewSpec {
        filter: Filter {
            query: query.into(),
            regex,
            column,
            conditions: vec![],
        },
        ..Default::default()
    };
    assert_eq!(run(&src, &quick("nl", None, false)).expect("f"), vec![0, 1]);
    assert_eq!(
        run(&src, &quick("21", Some(0), false)).expect("f"),
        Vec::<u64>::new()
    );
    assert_eq!(
        run(&src, &quick("^(be|de)$", Some(0), true)).expect("f"),
        vec![2, 3]
    );
    // Text never matches across two fields.
    assert_eq!(
        run(&src, &quick("nl,", None, false)).expect("f"),
        Vec::<u64>::new()
    );
    let cond = |op: Op, value: &str| ViewSpec {
        filter: Filter {
            conditions: vec![Condition {
                column: 1,
                op,
                value: value.into(),
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(run(&src, &cond(Op::Greater, "20")).expect("f"), vec![0, 2]);
    assert_eq!(run(&src, &cond(Op::Less, "10")).expect("f"), vec![1, 4]);
    assert_eq!(run(&src, &cond(Op::Equals, "21")).expect("f"), vec![0, 2]);
    let empty = ViewSpec {
        filter: Filter {
            conditions: vec![Condition {
                column: 0,
                op: Op::Empty,
                value: String::new(),
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(run(&src, &empty).expect("f"), vec![4]);
}

#[test]
fn invalid_regex_and_cancel_are_errors() {
    let src = table(&["a"]);
    let spec = ViewSpec {
        filter: Filter {
            query: "([a-z".into(),
            regex: true,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(run(&src, &spec), Err(ViewError::Invalid(_))));
    let cancelled = run_view(
        &src,
        COMMA,
        0,
        &ViewSpec {
            filter: Filter {
                query: "a".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        &|| true,
        &|_| {},
    );
    assert_eq!(cancelled, Err(ViewError::Cancelled));
}

#[test]
fn non_ascii_filters_fold_case() {
    let src = table(&["Ärger", "ärger", "arger"]);
    let spec = ViewSpec {
        filter: Filter {
            query: "ÄRGER".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(run(&src, &spec).expect("f"), vec![0, 1]);
}

#[test]
fn key_order() {
    use super::query::Key;
    let n = Key::Num(1.0);
    let t = Key::Text(Box::from(&b"a"[..]));
    assert_eq!(compare(&n, &t, false), std::cmp::Ordering::Less);
    assert_eq!(compare(&Key::Empty, &n, true), std::cmp::Ordering::Greater);
    assert_eq!(compare(&n, &Key::Empty, true), std::cmp::Ordering::Less);
}

#[test]
fn render_through_a_vfs() {
    let vfs = fm_testkit::MemFs::new();
    vfs.mkdirs_str("/t").expect("dir");
    vfs.put_str("/t/data.csv", b"a,b\n1,2\n3,4\n")
        .expect("write");
    let uri = fm_vfs::VfsUri::parse("/t/data.csv").expect("uri");
    let entry = fm_vfs::Vfs::stat(&vfs, &uri).expect("stat");
    match crate::render(&vfs, &entry, &Limits::default()) {
        Preview::Table(t) => {
            assert_eq!(t.columns.len(), 2);
            assert_eq!(t.sample.len(), 2);
            assert!(t.header && t.sample_complete);
        }
        other => panic!("{other:?}"),
    }
}
