//! Bounded multi-selection loading, performed on the source backend's job lane.
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use fm_preview::log::{LogDoc, LogFormat, LogLimits, LogRecord};
use fm_preview::{Limits, Preview, RenderOpts, TextDoc};
use fm_vfs::{Entry, Vfs};

use crate::preview_cmds::{Body, LogView, run_budgeted};

pub const MAX_SOURCES: usize = 64;
pub const MAX_WINDOW: u64 = 32 << 20;

#[derive(Debug)]
pub struct SourceRange {
    pub entry: Entry,
    pub records: Range<usize>,
    pub note: Option<String>,
}

pub(crate) fn render(
    vfs: Arc<dyn Vfs>,
    entries: Vec<Entry>,
    window: u64,
    cancel: Arc<AtomicBool>,
    filters: crate::preview_types::PreviewFilters,
    budget: std::time::Duration,
) -> Body {
    run_budgeted(budget, move || {
        let rules = match filters.compile() {
            Ok(rules) => rules,
            Err(error) => return Body::Static(Arc::new(Preview::Error(error.to_string()))),
        };
        render_inner(&*vfs, entries, window, &cancel, &rules)
    })
    .unwrap_or_else(|| {
        Body::Static(Arc::new(Preview::Error(
            "The selected-file preview took too long. Select fewer files or try again.".into(),
        )))
    })
}

fn text_records(doc: TextDoc) -> (Vec<LogRecord>, Option<String>) {
    let note = doc.truncated.then(|| "text preview truncated".into());
    let records = doc
        .lines
        .into_iter()
        .map(|line| LogRecord {
            text: line.into_iter().map(|span| span.text).collect(),
            ts: None,
            level: None,
            status: None,
            fields: Vec::new(),
            orphan: false,
        })
        .collect();
    (records, note)
}

fn render_inner(
    vfs: &dyn Vfs,
    entries: Vec<Entry>,
    window: u64,
    cancel: &AtomicBool,
    rules: &crate::preview_types::PreviewRules,
) -> Body {
    let count = entries.len().max(1);
    let limits = Limits {
        text_bytes: (8 << 20) / count,
        text_lines: 100_000 / count,
        pretty_bytes: 0,
        image_edge: 512,
        gif_frames: 1,
        dir_entries: 5_000 / count,
        dir_items: 0,
        ..Limits::default()
    };
    let logs = LogLimits {
        initial_tail: window / count as u64,
        max_bytes: window / count as u64,
        max_records: 100_000 / count,
        ..LogLimits::default()
    };
    let rendered: Vec<_> = entries
        .into_iter()
        .take_while(|_| !cancel.load(Ordering::Relaxed))
        .map(|entry| {
            let preview = fm_preview::render_configured(
                vfs,
                &entry,
                &limits,
                RenderOpts::default(),
                &logs,
                rules.kind(&entry.name),
            );
            let preview = if matches!(preview, Preview::Markdown(_)) {
                fm_preview::text::render_text(vfs, &entry, &limits, "text")
                    .unwrap_or_else(|e| Preview::Error(e.to_string()))
            } else {
                preview
            };
            (entry, preview)
        })
        .collect();
    let compatible = rendered.iter().all(|(_, p)| {
        matches!(
            p,
            Preview::Log(_) | Preview::Text(_) | Preview::Structured(_) | Preview::Error(_)
        )
    }) && rendered
        .iter()
        .any(|(_, p)| !matches!(p, Preview::Error(_)));
    if !compatible {
        return Body::Grid(
            rendered
                .into_iter()
                .map(|(entry, preview)| (entry, Arc::new(preview)))
                .collect(),
        );
    }
    let mut doc = LogDoc {
        format: LogFormat::Generic,
        records: Vec::new(),
        more_before: false,
        start_offset: 0,
        end_offset: 0,
        gz: false,
        bytes: 0,
        note: Some(
            "Filters search loaded records only. Use Refresh to reload changes from the selected files.".into(),
        ),
        base: 0,
        marks: Vec::new(),
    };
    let mut sources = Vec::new();
    let mut common_format = None;
    for (entry, preview) in rendered {
        let start = doc.records.len();
        let (records, note) = match preview {
            Preview::Log(log) => {
                common_format = Some(match common_format {
                    None => log.format,
                    Some(format) if format == log.format => format,
                    Some(_) => LogFormat::Generic,
                });
                doc.more_before |= log.more_before || log.start_offset > 0;
                (log.records, log.note)
            }
            Preview::Text(text) => {
                common_format = Some(LogFormat::Generic);
                text_records(text)
            }
            Preview::Structured(structured) => {
                common_format = Some(LogFormat::Generic);
                text_records(structured.raw)
            }
            Preview::Error(error) => (Vec::new(), Some(format!("Cannot preview: {error}"))),
            _ => (Vec::new(), Some("This format cannot be combined.".into())),
        };
        doc.records.extend(records);
        sources.push(SourceRange {
            entry,
            records: start..doc.records.len(),
            note,
        });
    }
    doc.bytes = doc.records.iter().map(|r| r.text.len() as u64).sum();
    doc.format = common_format.unwrap_or(LogFormat::Generic);
    if window >= MAX_WINDOW && doc.more_before {
        doc.note = Some("Combined window is full. Select fewer files for a larger per-file window. Filters search loaded records only.".into());
    }
    let mut view = LogView::new(doc);
    view.sources = sources;
    view.window_bytes = window;
    Body::Log(Box::new(view))
}
