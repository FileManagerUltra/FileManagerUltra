//! Copy a code block from an already-loaded Markdown preview.

use fm_layout::PaneKind;
use fm_preview::{Preview, markdown::MdBlock};
use fm_vfs::FmError;

use crate::{CommandSpec, Params, preview_cmds::Body};

fn code_at<'a>(blocks: &'a [MdBlock], index: &mut usize) -> Option<&'a str> {
    for block in blocks {
        match block {
            MdBlock::Code(code) => {
                if *index == 0 {
                    return Some(&code.text);
                }

                *index -= 1;
            }
            MdBlock::Quote(children) => {
                if let Some(code) = code_at(children, index) {
                    return Some(code);
                }
            }
            MdBlock::List { items, .. } => {
                for item in items {
                    if let Some(code) = code_at(&item.blocks, index) {
                        return Some(code);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.code.copy",
        label: "Copy Code Block",
        menu: &["Preview", "Copy Code Block"],
        shortcut: None,
        params: "{pane?: id, block: zero-based code-block index, item?: grid-item index}",
        enabled: |s| s.pane_kind(s.layout.focused()) == Some(PaneKind::Preview),
        run: |s, params: &Params| {
            let map = crate::file_cmds::object(params)?;
            let index = |name: &str| -> Result<Option<usize>, FmError> {
                map.and_then(|m| m.get(name))
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| {
                                FmError::InvalidParams(format!(
                                    "{name}: non-negative integer expected"
                                ))
                            })
                    })
                    .transpose()
            };
            let pane = match map.and_then(|m| m.get("pane")) {
                Some(value) => value
                    .as_u64()
                    .ok_or_else(|| FmError::InvalidParams("pane: pane id expected".into()))?,
                None => s.layout.focused(),
            };
            let mut block = index("block")?
                .ok_or_else(|| FmError::InvalidParams("block: index required".into()))?;
            let item = index("item")?;
            if s.pane_kind(pane) != Some(PaneKind::Preview) {
                return Err(FmError::Unsupported("not a preview pane"));
            }
            let slot = s
                .preview_slot(pane)
                .ok_or(FmError::Unsupported("no Markdown preview"))?;
            let preview = match (&slot.body, item) {
                (Body::Static(preview), None) => preview,
                (Body::Grid(items), Some(item)) => {
                    &items
                        .get(item)
                        .ok_or_else(|| FmError::InvalidParams("item: out of range".into()))?
                        .1
                }
                _ => return Err(FmError::Unsupported("no matching Markdown preview")),
            };
            let Preview::Markdown(doc) = &**preview else {
                return Err(FmError::Unsupported("not a Markdown preview"));
            };
            let code = code_at(&doc.blocks, &mut block)
                .ok_or_else(|| FmError::InvalidParams("block: out of range".into()))?;
            s.clipboard_out = Some(code.to_owned());
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Session, preview_cmds::PreviewSlot};
    use fm_preview::markdown::{MdDoc, parse};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn copies_only_requested_code_including_nested_and_grid_blocks() {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        let doc = Arc::new(Preview::Markdown(MdDoc {
            blocks: parse(
                "# Title\n\n```rust\nfirst();\n```\n\n> ```\n> quoted();\n> ```\n\n- item\n\n  ```text\n  nested\n  ```",
            ),
            truncated: false,
            total_bytes: 0,
        }));
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(doc.clone());
        s.previews.slots.insert(pane, slot);
        for (block, expected) in [(0, "first();"), (1, "quoted();"), (2, "nested")] {
            s.dispatch("preview.code.copy", &json!({"block": block}))
                .expect("copy");
            assert_eq!(s.clipboard_out.take().as_deref(), Some(expected));
        }
        s.dispatch("preview.text.select_all", &Params::Null)
            .expect("select all");
        s.dispatch("preview.code.copy", &json!({"block": 0}))
            .expect("copy code");
        assert!(crate::preview_text_cmds::selected(&s, pane));
        s.dispatch("preview.text.copy", &Params::Null)
            .expect("copy document");
        let all = s.clipboard_out.take().expect("document");
        assert!(all.contains("Title") && all.contains("nested"));
        assert!(!all.contains("Copy") && !all.contains("rust"));
        let entry = fm_vfs::Entry::new(
            fm_vfs::VfsUri::parse("/code.md").expect("uri"),
            fm_vfs::EntryKind::File,
        );
        s.previews.slots.get_mut(&pane).expect("slot").body = Body::Grid(vec![(entry, doc)]);
        s.dispatch(
            "preview.code.copy",
            &json!({"pane": pane, "item": 0, "block": 2}),
        )
        .expect("grid copy");
        assert_eq!(s.clipboard_out.take().as_deref(), Some("nested"));
        for bad in [
            json!(null),
            json!({"block": -1}),
            json!({"block": "0"}),
            json!({"item": 0, "block": 100}),
            json!({"item": 100, "block": 0}),
            json!({"item": false, "block": 0}),
            json!({"pane": "bad", "block": 0}),
            json!({"pane": 999999, "block": 0}),
            json!({"block": 0}),
        ] {
            assert!(s.dispatch("preview.code.copy", &bad).is_err(), "{bad}");
            assert!(s.clipboard_out.is_none());
        }
    }
}
