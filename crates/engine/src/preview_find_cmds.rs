//! Pane-local find over loaded documents. Matching never runs on the UI thread.

use std::{ops::Range, sync::Arc};

use crossbeam_channel::Receiver;
use fm_layout::PaneId;
use fm_preview::{
    Preview, TextDoc,
    markdown::{Inline, MdBlock},
};
use fm_vfs::FmError;
use regex::RegexBuilder;

use crate::{CommandSpec, Params, Session, preview_cmds::Body};

pub const MAX_QUERY: usize = 1024;
pub const MAX_MATCHES: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Text line, or Markdown paragraph/heading/code block/table cell in render order.
    pub segment: usize,
    pub bytes: Range<usize>,
}

#[derive(Debug)]
pub struct Matches {
    pub hits: Vec<Hit>,
    pub capped: bool,
}

#[derive(Debug)]
struct Pending {
    document: Arc<Preview>,
    pretty: bool,
    options: Options,
    rx: Receiver<Result<Matches, String>>,
}

#[derive(Debug, Default)]
pub struct Find {
    pub open: bool,
    pub options: Options,
    pub matches: Option<Arc<Matches>>,
    pub current: usize,
    pub error: Option<String>,
    pub busy: bool,
    /// UI focus and reveal requests; each is handled once by the pane's widgets.
    pub focus: u64,
    pub reveal: u64,
    document: Option<Arc<Preview>>,
    pretty: bool,
    searched: Options,
    pending: Option<Pending>,
}

pub fn searchable(p: &Preview) -> bool {
    matches!(
        p,
        Preview::Text(_) | Preview::Structured(_) | Preview::Markdown(_)
    )
}

pub fn available(s: &Session) -> bool {
    crate::diff_find_cmds::available(s)
        || s.preview_slot(s.layout.focused()).is_some_and(|slot| {
            matches!(&slot.body, Body::Log(_))
                || matches!(&slot.body, Body::Static(p) if searchable(p))
        }) && s.focused_kind() == Some(fm_layout::PaneKind::Preview)
}

fn text_available(s: &Session) -> bool {
    crate::diff_find_cmds::available(s)
        || available(s)
            && s.preview_slot(s.layout.focused())
                .is_some_and(|slot| matches!(&slot.body, Body::Static(p) if searchable(p)))
}

fn can_navigate(s: &Session) -> bool {
    crate::diff_find_cmds::can_navigate(s)
        || text_available(s)
            && s.preview_slot(s.layout.focused()).is_some_and(|slot| {
                slot.find.open
                    && slot
                        .find
                        .matches
                        .as_ref()
                        .is_some_and(|m| !m.hits.is_empty())
            })
}

fn can_close(s: &Session) -> bool {
    crate::diff_find_cmds::can_close(s)
        || s.focused_kind() == Some(fm_layout::PaneKind::Preview)
            && s.preview_slot(s.layout.focused())
                .is_some_and(|slot| slot.find.open)
}

fn segments(p: &Preview, pretty: bool, visit: &mut impl FnMut(String)) {
    fn text(doc: &TextDoc, visit: &mut impl FnMut(String)) {
        for line in &doc.lines {
            visit(line.iter().map(|span| span.text.as_str()).collect());
        }
    }
    fn inlines(parts: &[Inline]) -> String {
        parts.iter().map(|part| part.text.as_str()).collect()
    }
    fn markdown(blocks: &[MdBlock], visit: &mut impl FnMut(String)) {
        for block in blocks {
            match block {
                MdBlock::Heading(_, parts) | MdBlock::Para(parts) => visit(inlines(parts)),
                MdBlock::Code(code) => visit(code.text.clone()),
                MdBlock::List { items, .. } => {
                    for item in items {
                        markdown(&item.blocks, visit);
                    }
                }
                MdBlock::Quote(blocks) | MdBlock::Centered(blocks) => markdown(blocks, visit),
                MdBlock::Table(table) => {
                    let header = (!table.headerless).then_some(&table.header);
                    for row in header.into_iter().chain(&table.rows) {
                        for column in 0..table.alignments.len().max(1) {
                            visit(inlines(row.get(column).map_or(&[], Vec::as_slice)));
                        }
                    }
                }
                MdBlock::Rule | MdBlock::Images(_) => {}
            }
        }
    }
    match p {
        Preview::Text(doc) => text(doc, visit),
        Preview::Structured(doc) => text(
            if pretty {
                doc.pretty.as_ref().unwrap_or(&doc.raw)
            } else {
                &doc.raw
            },
            visit,
        ),
        Preview::Markdown(doc) if pretty => text(&doc.source, visit),
        Preview::Markdown(doc) => markdown(&doc.blocks, visit),
        _ => {}
    }
}

fn search(p: &Preview, pretty: bool, options: &Options) -> Result<Matches, String> {
    let pattern = if options.regex {
        options.query.clone()
    } else {
        regex::escape(&options.query)
    };
    let pattern = if options.whole_word {
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern
    };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|e| format!("Invalid search pattern: {e}"))?;
    let mut result = Matches {
        hits: Vec::new(),
        capped: false,
    };
    let mut segment = 0;
    segments(p, pretty, &mut |text| {
        if !result.capped {
            for hit in re.find_iter(&text).filter(|hit| !hit.is_empty()) {
                if result.hits.len() == MAX_MATCHES {
                    result.capped = true;
                    break;
                }
                result.hits.push(Hit {
                    segment,
                    bytes: hit.range(),
                });
            }
        }
        segment += 1;
    });
    Ok(result)
}

/// Poll/start at most one bounded search worker per pane; stale results are discarded.
pub fn prepare(s: &mut Session, pane: PaneId) {
    let Some(slot) = s.previews.slots.get_mut(&pane) else {
        return;
    };
    let find = &mut slot.find;
    let doc = match &slot.body {
        Body::Static(p) if searchable(p) => Some(p),
        _ => None,
    };
    let same = find.pretty == (slot.pretty || slot.source)
        && find.searched == find.options
        && match (&find.document, doc) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
    if !same {
        find.document = doc.cloned();
        find.pretty = slot.pretty || slot.source;
        find.searched = find.options.clone();
        find.matches = None;
        find.error = None;
        find.current = 0;
        find.reveal = find.reveal.wrapping_add(1);
    }
    if let Some(pending) = &find.pending {
        match pending.rx.try_recv() {
            Ok(result) => {
                if doc.is_some_and(|doc| Arc::ptr_eq(doc, &pending.document))
                    && pending.pretty == (slot.pretty || slot.source)
                    && pending.options == find.options
                {
                    match result {
                        Ok(matches) => find.matches = Some(Arc::new(matches)),
                        Err(error) => find.error = Some(error),
                    }
                    find.reveal = find.reveal.wrapping_add(1);
                }
                find.pending = None;
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                find.pending = None;
                find.error =
                    Some("Preview search worker stopped; change the query to retry.".into());
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }
    if find.open
        && !find.options.query.is_empty()
        && find.matches.is_none()
        && find.error.is_none()
        && find.pending.is_none()
        && let Some(document) = doc
    {
        let document = document.clone();
        let options = find.options.clone();
        let pretty = slot.pretty || slot.source;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let worker_doc = document.clone();
        let worker_options = options.clone();
        match std::thread::Builder::new()
            .name("fm-preview-find".into())
            .spawn(move || {
                let _ = tx.send(search(&worker_doc, pretty, &worker_options));
            }) {
            Ok(_) => {
                find.pending = Some(Pending {
                    document,
                    options,
                    pretty,
                    rx,
                })
            }
            Err(error) => find.error = Some(format!("Cannot start preview search: {error}")),
        }
    }
    find.busy = find.pending.is_some();
}

fn change(s: &mut Session, params: &Params, mode: &str) -> Result<(), FmError> {
    let object = if params.is_null() {
        None
    } else {
        Some(
            params
                .as_object()
                .ok_or_else(|| FmError::InvalidParams("expected an object".into()))?,
        )
    };
    if let Some(object) = object {
        for (key, value) in object {
            let valid = match key.as_str() {
                "pane" => value.as_u64().is_some(),
                "query" if mode == "set" => value
                    .as_str()
                    .is_some_and(|q| q.len() <= MAX_QUERY * 4 && q.chars().count() <= MAX_QUERY),
                "case_sensitive" | "whole_word" | "regex" if mode == "set" => value.is_boolean(),
                _ => false,
            };
            if !valid {
                return Err(FmError::InvalidParams(format!(
                    "invalid preview find parameter: {key}"
                )));
            }
        }
    }
    let pane = s.preview_pane(params)?;
    if crate::diff_find_cmds::active(s, pane) {
        return crate::diff_find_cmds::change(s, pane, object, mode);
    }
    let slot = s
        .previews
        .slots
        .get_mut(&pane)
        .ok_or(FmError::Unsupported("no loaded preview"))?;
    if mode != "close"
        && !matches!(&slot.body, Body::Log(_))
        && !matches!(&slot.body, Body::Static(p) if searchable(p))
    {
        return Err(FmError::Unsupported(
            "Find is available in text, structured, Markdown and log previews",
        ));
    }
    if matches!(&slot.body, Body::Log(_)) && !matches!(mode, "open" | "close") {
        return Err(FmError::Unsupported(
            "use the log record search and filters",
        ));
    }
    let find = &mut slot.find;
    match mode {
        "open" => {
            find.open = true;
            find.focus = find.focus.wrapping_add(1);
        }
        "close" => {
            if !find.open {
                return Err(FmError::Unsupported("Preview Find is not open"));
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
            find.options.query = query.into();
            for (key, target) in [
                ("case_sensitive", &mut find.options.case_sensitive),
                ("whole_word", &mut find.options.whole_word),
                ("regex", &mut find.options.regex),
            ] {
                if let Some(value) = object.get(key).and_then(Params::as_bool) {
                    *target = value;
                }
            }
        }
        _ => {
            let count = find.matches.as_ref().map_or(0, |m| m.hits.len());
            if count == 0 || !find.open {
                return Err(FmError::Unsupported("no preview search matches"));
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

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "preview.find.open",
            label: "Find in Preview",
            menu: &["Preview", "Find"],
            shortcut: Some("Ctrl+F"),
            params: "{pane?: number}",
            enabled: available,
            run: |s, p| change(s, p, "open"),
        },
        CommandSpec {
            id: "preview.find.close",
            label: "Close Preview Find",
            menu: &["Preview", "Close Find"],
            shortcut: Some("Escape"),
            params: "{pane?: number}",
            enabled: can_close,
            run: |s, p| change(s, p, "close"),
        },
        CommandSpec {
            id: "preview.find.set",
            label: "Set Preview Find Options",
            menu: &["Preview", "Find Options"],
            shortcut: None,
            params: "{pane?: number, query: string <=1024 characters, case_sensitive?: bool, whole_word?: bool, regex?: bool}",
            enabled: text_available,
            run: |s, p| change(s, p, "set"),
        },
        CommandSpec {
            id: "preview.find.next",
            label: "Next Preview Match",
            menu: &["Preview", "Next Match"],
            shortcut: Some("F3"),
            params: "{pane?: number}",
            enabled: can_navigate,
            run: |s, p| change(s, p, "next"),
        },
        CommandSpec {
            id: "preview.find.previous",
            label: "Previous Preview Match",
            menu: &["Preview", "Previous Match"],
            shortcut: Some("Shift+F3"),
            params: "{pane?: number}",
            enabled: can_navigate,
            run: |s, p| change(s, p, "previous"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_cmds::PreviewSlot;
    use fm_preview::{Class, Span};
    use serde_json::json;

    fn text(lines: &[&str]) -> Preview {
        Preview::Text(TextDoc {
            lines: lines
                .iter()
                .map(|text| {
                    vec![Span {
                        text: (*text).into(),
                        class: Class::Plain,
                    }]
                })
                .collect(),
            lang: "text".into(),
            truncated: false,
            total_bytes: 0,
        })
    }

    fn session(p: Preview) -> (Session, PaneId) {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = fm_layout::PaneKind::Preview;
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(Arc::new(p));
        s.previews.slots.insert(pane, slot);
        (s, pane)
    }

    fn wait(s: &mut Session, pane: PaneId) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            prepare(s, pane);
            let find = &s.preview_slot(pane).expect("slot").find;
            if !find.busy {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "search timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn matching_handles_unicode_case_words_regex_and_zero_width_patterns() {
        let p = text(&["Été été étés", "a.b axb", "猫 猫咪"]);
        let mut options = Options {
            query: "ÉTÉ".into(),
            ..Default::default()
        };
        assert_eq!(search(&p, false, &options).expect("search").hits.len(), 3);
        options.whole_word = true;
        assert_eq!(search(&p, false, &options).expect("words").hits.len(), 2);
        options.case_sensitive = true;
        assert!(search(&p, false, &options).expect("case").hits.is_empty());
        options = Options {
            query: "a.b".into(),
            ..Default::default()
        };
        assert_eq!(search(&p, false, &options).expect("literal").hits.len(), 1);
        options.regex = true;
        assert_eq!(search(&p, false, &options).expect("regex").hits.len(), 2);
        options.query = "^".into();
        assert!(
            search(&p, false, &options)
                .expect("zero width")
                .hits
                .is_empty()
        );
        options.query = "[".into();
        assert!(search(&p, false, &options).is_err());
        options.query = "猫".into();
        options.regex = false;
        options.whole_word = true;
        assert_eq!(
            search(&p, false, &options).expect("unicode words").hits,
            vec![Hit {
                segment: 2,
                bytes: 0..3
            }]
        );
    }

    #[test]
    fn markdown_segments_match_render_order_and_exclude_chrome() {
        let p = Preview::Markdown(fm_preview::markdown::MdDoc {
            blocks: fm_preview::markdown::parse(
                "# **needle**\n\n> quoted needle\n\n- needle\n\n```needle\nneedle\n```\n\n| needle | other |\n| --- | --- |\n| cell | needle |\n",
            ),
            source: Default::default(),
            truncated: false,
            total_bytes: 0,
        });
        let hits = search(
            &p,
            false,
            &Options {
                query: "needle".into(),
                ..Default::default()
            },
        )
        .expect("markdown")
        .hits;
        assert_eq!(
            hits.iter().map(|hit| hit.segment).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 7]
        );
        let joined = text(&["raw"]);
        let Preview::Text(raw) = joined else {
            panic!("text")
        };
        let Preview::Text(pretty) = text(&["pretty", "needle"]) else {
            panic!("text")
        };
        let p = Preview::Structured(fm_preview::StructDoc {
            format: fm_preview::Format::Json,
            raw,
            pretty: Ok(pretty),
            pretty_note: None,
        });
        let options = Options {
            query: "needle".into(),
            ..Default::default()
        };
        assert!(search(&p, false, &options).expect("raw").hits.is_empty());
        assert_eq!(
            search(&p, true, &options).expect("pretty").hits[0].segment,
            1
        );
    }

    #[test]
    fn asynchronous_search_rejects_stale_results_wraps_and_is_pane_local() {
        let (mut s, pane) = session(text(&["old old"]));
        s.dispatch("preview.find.open", &Params::Null)
            .expect("open");
        s.dispatch("preview.find.set", &json!({"query": "old"}))
            .expect("query");
        prepare(&mut s, pane);
        s.previews.slots.get_mut(&pane).expect("slot").body =
            Body::Static(Arc::new(text(&["new", "new", "new"])));
        s.dispatch("preview.find.set", &json!({"query": "new"}))
            .expect("new query");
        wait(&mut s, pane);
        let find = &s.preview_slot(pane).expect("slot").find;
        assert_eq!(find.matches.as_ref().expect("matches").hits.len(), 3);
        assert_eq!(find.current, 0);
        s.dispatch("preview.find.previous", &Params::Null)
            .expect("previous");
        assert_eq!(s.preview_slot(pane).expect("slot").find.current, 2);
        s.dispatch("preview.find.next", &Params::Null)
            .expect("next");
        assert_eq!(s.preview_slot(pane).expect("slot").find.current, 0);
        let other = s
            .layout
            .split(pane, fm_layout::Axis::Horizontal, 0.5)
            .expect("split");
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(Arc::new(text(&["other"])));
        s.layout.state_mut(other).expect("pane").kind = fm_layout::PaneKind::Preview;
        s.previews.slots.insert(other, slot);
        s.dispatch("preview.find.open", &Params::Null)
            .expect("other open");
        assert!(
            s.preview_slot(other)
                .expect("other")
                .find
                .options
                .query
                .is_empty()
        );
        assert_eq!(
            s.preview_slot(pane).expect("first").find.options.query,
            "new"
        );
        s.dispatch("preview.find.close", &json!({"pane": pane}))
            .expect("close");
        assert!(!s.preview_slot(pane).expect("first").find.open);
    }

    #[test]
    fn malformed_parameters_errors_and_match_limits_are_explicit() {
        let (mut s, pane) = session(text(&["x"]));
        for id in [
            "preview.find.open",
            "preview.find.close",
            "preview.find.set",
            "preview.find.next",
            "preview.find.previous",
        ] {
            for bad in [
                json!([]),
                json!("x"),
                json!({"pane": -1}),
                json!({"pane": 999999}),
                json!({"unknown": true}),
            ] {
                assert!(s.dispatch(id, &bad).is_err(), "{id}: {bad}");
            }
        }
        for bad in [
            json!({"query": 4}),
            json!({"query": "x".repeat(MAX_QUERY + 1)}),
            json!({"regex": 1}),
            json!({"whole_word": "yes"}),
            json!({"case_sensitive": null}),
        ] {
            assert!(s.dispatch("preview.find.set", &bad).is_err());
        }
        s.dispatch("preview.find.open", &Params::Null)
            .expect("open");
        s.dispatch("preview.find.set", &json!({"query": "[", "regex": true}))
            .expect("pattern");
        wait(&mut s, pane);
        assert!(s.preview_slot(pane).expect("slot").find.error.is_some());
        s.dispatch("preview.find.set", &json!({"query": "x", "regex": false}))
            .expect("recover");
        wait(&mut s, pane);
        assert_eq!(
            s.preview_slot(pane)
                .expect("slot")
                .find
                .matches
                .as_ref()
                .expect("matches")
                .hits
                .len(),
            1
        );
        let result = search(
            &text(&[&"x".repeat(MAX_MATCHES + 1)]),
            false,
            &Options {
                query: "x".into(),
                ..Default::default()
            },
        )
        .expect("bounded");
        assert_eq!(result.hits.len(), MAX_MATCHES);
        assert!(result.capped);
        s.previews.slots.get_mut(&pane).expect("slot").body =
            Body::Static(Arc::new(Preview::Error("unavailable".into())));
        prepare(&mut s, pane);
        assert!(!s.command_enabled("preview.find.open"));
        assert!(s.preview_slot(pane).expect("slot").find.matches.is_none());
        assert!(s.dispatch("preview.find.open", &Params::Null).is_err());
        assert!(s.dispatch("preview.find.close", &Params::Null).is_ok());
    }
}
