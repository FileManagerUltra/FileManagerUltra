//! Whole-document text selection for bounded, already-loaded previews.

use std::sync::Arc;

use fm_layout::{PaneId, PaneKind};
use fm_preview::{Preview, TextDoc};
use fm_vfs::FmError;

use crate::{CommandSpec, Params, Session, preview_cmds::Body};

#[derive(Debug)]
pub struct TextSelection {
    pane: PaneId,
    document: Arc<Preview>,
    pretty: bool,
}

fn document(s: &Session) -> Option<(PaneId, &Arc<Preview>, bool)> {
    let pane = s.layout.focused();
    if s.pane_kind(pane) != Some(PaneKind::Preview) {
        return None;
    }
    let slot = s.preview_slot(pane)?;
    let Body::Static(p) = &slot.body else {
        return None;
    };
    matches!(
        &**p,
        Preview::Text(_) | Preview::Structured(_) | Preview::Markdown(_)
    )
    .then_some((pane, p, slot.pretty || slot.source))
}

pub fn selected(s: &Session, pane: PaneId) -> bool {
    s.preview_text_selection.as_ref().is_some_and(|selection| {
        document(s).is_some_and(|(current, p, pretty)| {
            current == pane
                && selection.pane == pane
                && Arc::ptr_eq(p, &selection.document)
                && pretty == selection.pretty
        })
    })
}

fn plain_text(d: &TextDoc) -> String {
    d.lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contents(p: &Preview, pretty: bool) -> Result<String, FmError> {
    Ok(match p {
        Preview::Text(d) => plain_text(d),
        Preview::Structured(d) => plain_text(if pretty {
            d.pretty.as_ref().unwrap_or(&d.raw)
        } else {
            &d.raw
        }),
        Preview::Markdown(d) if pretty => plain_text(&d.source),
        Preview::Markdown(d) => fm_preview::markdown::plain_text(&d.blocks),
        _ => return Err(FmError::Unsupported("no text preview")),
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "preview.text.select_all",
            label: "Select All Preview Text",
            menu: &["Preview", "Select All Text"],
            shortcut: Some("Ctrl+A"),
            params: "none",
            enabled: |s| document(s).is_some(),
            run: |s, params| {
                if !params.is_null() {
                    return Err(FmError::InvalidParams("no parameters expected".into()));
                }

                let (pane, p, pretty) =
                    document(s).ok_or(FmError::Unsupported("no text preview"))?;
                s.preview_text_selection = Some(TextSelection {
                    pane,
                    document: p.clone(),
                    pretty,
                });
                Ok(())
            },
        },
        CommandSpec {
            id: "preview.text.copy",
            label: "Copy Selected Preview Text",
            menu: &["Preview", "Copy Text"],
            shortcut: Some("Ctrl+C"),
            params: "none",
            enabled: |s| selected(s, s.layout.focused()),
            run: |s, params: &Params| {
                if !params.is_null() {
                    return Err(FmError::InvalidParams("no parameters expected".into()));
                }
                if !selected(s, s.layout.focused()) {
                    return Err(FmError::Unsupported("no preview text selection"));
                }
                let (_, p, pretty) = document(s).ok_or(FmError::Unsupported("no text preview"))?;
                s.clipboard_out = Some(contents(p, pretty)?);
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_cmds::PreviewSlot;
    use fm_preview::{Class, Span};

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

    #[test]
    fn selects_and_copies_all_rows_and_rejects_stale_content() {
        let mut s = Session::new().expect("session");
        assert!(
            s.dispatch("preview.text.select_all", &Params::Null)
                .is_err()
        );
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        let mut slot = PreviewSlot::new();
        let lines: Vec<String> = (0..100).map(|n| format!("row-{n:03} 漢")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        slot.body = Body::Static(Arc::new(text(&refs)));
        s.previews.slots.insert(pane, slot);
        assert!(s.dispatch("preview.text.copy", &Params::Null).is_err());
        s.dispatch("preview.text.select_all", &Params::Null)
            .expect("select");
        s.dispatch("preview.text.copy", &Params::Null)
            .expect("copy");
        assert_eq!(s.clipboard_out.take().expect("clipboard"), lines.join("\n"));
        s.previews.slots.get_mut(&pane).expect("slot").pretty = true;
        assert!(!selected(&s, pane));
        assert!(s.dispatch("preview.text.copy", &Params::Null).is_err());
        s.previews.slots.get_mut(&pane).expect("slot").pretty = false;
        s.previews.slots.get_mut(&pane).expect("slot").body =
            Body::Static(Arc::new(text(&["new"])));
        assert!(!selected(&s, pane));
        assert!(s.dispatch("preview.text.copy", &Params::Null).is_err());
        for id in ["preview.text.select_all", "preview.text.copy"] {
            for bad in [
                serde_json::json!({}),
                serde_json::json!("x"),
                serde_json::json!([1]),
            ] {
                assert!(s.dispatch(id, &bad).is_err());
            }
        }
    }

    #[test]
    fn markdown_and_structured_copy_match_displayed_content() {
        let md = Preview::Markdown(fm_preview::markdown::MdDoc {
            blocks: fm_preview::markdown::parse(
                "# Title\n\nHello **world**.\n\n```rs\nlet x = 1;\n```",
            ),
            source: Default::default(),
            truncated: false,
            total_bytes: 0,
        });
        assert_eq!(
            contents(&md, false).expect("markdown"),
            "Title\nHello world.\nlet x = 1;"
        );
        let Preview::Markdown(mut doc) = md else {
            panic!("markdown")
        };
        doc.source = fm_preview::TextDoc {
            lines: vec![vec![fm_preview::Span {
                text: "# Title".into(),
                class: fm_preview::Class::Plain,
            }]],
            ..Default::default()
        };
        assert_eq!(
            contents(&Preview::Markdown(doc), true).expect("source"),
            "# Title"
        );
        let Preview::Text(raw) = text(&["{\"a\":1}"]) else {
            panic!("text")
        };
        let Preview::Text(pretty) = text(&["{", "  \"a\": 1", "}"]) else {
            panic!("text")
        };
        let structured = Preview::Structured(fm_preview::StructDoc {
            format: fm_preview::Format::Json,
            raw,
            pretty: Ok(pretty),
            pretty_note: None,
        });
        assert_eq!(contents(&structured, false).expect("raw"), "{\"a\":1}");
        assert_eq!(
            contents(&structured, true).expect("pretty"),
            "{\n  \"a\": 1\n}"
        );
    }

    #[test]
    fn rich_markdown_copy_preserves_tasks_tables_and_nested_blocks() {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(Arc::new(Preview::Markdown(fm_preview::markdown::MdDoc {
            blocks: fm_preview::markdown::parse(
                "- [x] **done**\n  - nested\n- [ ] pending\n\n> quoted\n\n| A | B |\n| - | - |\n| `one` | ~~two~~ |\n\n```rs\nlet x = 1;\n```",
            ),
            source: Default::default(),
            truncated: false,
            total_bytes: 0,
        })));
        s.previews.slots.insert(pane, slot);
        s.dispatch("preview.text.select_all", &Params::Null)
            .expect("select");
        s.dispatch("preview.scale.in", &Params::Null)
            .expect("zoom keeps selection");
        s.dispatch("preview.text.copy", &Params::Null)
            .expect("copy");
        assert_eq!(
            s.clipboard_out.take().expect("clipboard"),
            "[x] done\n  • nested\n[ ] pending\nquoted\nA\tB\none\ttwo\nlet x = 1;"
        );
    }
}
