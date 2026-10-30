//! Open a pointed item in a preview without toggling an existing preview closed.

use fm_layout::{Axis, PaneKind, PreviewSource};
use fm_vfs::FmError;

use crate::{CommandSpec, Params, Session, preview_cmds::Body};

fn source(s: &Session) -> Option<fm_layout::PaneId> {
    let focused = s.layout.focused();
    if s.focused_is_files() {
        Some(focused)
    } else {
        s.grid_selection
            .as_ref()
            .filter(|grid| grid.pane == focused)
            .and_then(|_| s.layout.preview_target(focused))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.open",
        label: "Open in Preview Panel",
        menu: &["File", "Open in Preview Panel"],
        shortcut: None,
        params: "{uri: URI of an item in the focused listing or thumbnail grid}",
        enabled: |s| source(s).is_some(),
        run: |s, params: &Params| {
            let uri = crate::file_cmds::object(params)?
                .and_then(|m| m.get("uri"))
                .ok_or_else(|| FmError::InvalidParams("uri: required".into()))
                .and_then(crate::file_cmds::parse_uri)?;
            let origin = source(s).ok_or(FmError::Unsupported("no source Files pane"))?;
            let focused = s.layout.focused();
            let visible = if focused == origin {
                s.pane_view(origin).is_some_and(|(entries, ix)| {
                    ix.order
                        .iter()
                        .filter_map(|i| entries.get(*i as usize))
                        .any(|entry| entry.uri == uri)
                })
            } else {
                s.preview_slot(focused)
                    .is_some_and(|slot| match &slot.body {
                        Body::Static(doc) => match &**doc {
                            fm_preview::Preview::Dir(dir) => dir.items.iter().any(|e| e.uri == uri),
                            _ => false,
                        },
                        _ => false,
                    })
            };
            if !visible {
                return Err(FmError::NotFound(
                    "the item is no longer in this view; refresh and try again".into(),
                ));
            }
            let parent = uri
                .parent()
                .ok_or(FmError::Unsupported("cannot preview this location"))?;
            let name = uri
                .file_name()
                .ok_or(FmError::Unsupported("cannot preview this location"))?;
            s.backends.require(&uri)?;
            let existing = s
                .layout
                .panes()
                .into_iter()
                .filter(|id| s.pane_kind(*id) == Some(PaneKind::Preview))
                .find(|id| s.layout.preview_target(*id) == Some(origin))
                .or_else(|| {
                    s.layout
                        .panes()
                        .into_iter()
                        .find(|id| s.pane_kind(*id) == Some(PaneKind::Preview))
                });
            let preview = match existing {
                Some(id) => id,
                None => s.layout.split(origin, Axis::Horizontal, 0.55)?,
            };
            if let Some(state) = s.layout.state_mut(preview) {
                state.kind = PaneKind::Preview;
                state.preview_source = PreviewSource::Pane(origin);
            }
            if let Some(state) = s.layout.state_mut(origin) {
                if state.location != parent {
                    state.navigate(parent.clone());
                    state.listing.filter = None;
                }
                state.tree.active = false;
                state.listing.select_only(Some(name), 0);
            }
            s.invalidate_view(origin);
            if let Some((entries, ix)) = s.pane_view(origin)
                && let Some(row) = ix
                    .order
                    .iter()
                    .position(|i| entries.get(*i as usize).is_some_and(|e| e.uri == uri))
                && let Some(state) = s.layout.state_mut(origin)
            {
                state.listing.select_only(Some(name), row);
            }
            s.layout.focus_pane(origin)?;
            s.layout.zoomed = None;
            s.layout.touch_files_focus();
            s.ensure_read(parent)?;
            Ok(())
        },
    }]
}
