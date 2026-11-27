//! Live preview (change `add-editor-folding-live-preview`): an ordinary Preview pane
//! whose source is a Preview pane in Edit mode renders that editor's unsaved text. The
//! editor's buffer is snapshotted after a short pause in typing and rendered through
//! the usual preview pipeline off the UI thread; links and images resolve against the
//! file's folder because the snapshot keeps the file's uri.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fm_layout::{Axis, PaneId, PaneKind, PreviewSource};
use fm_preview::snapshot::SnapshotVfs;
use fm_preview::{Limits, Preview};
use fm_vfs::{Entry, FmError, Vfs, VfsUri};

use crate::editor_doc::Phase;
use crate::preview_cmds::{Body, Msg, PreviewHub, PreviewSlot};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Buffers above this size are not live-previewed (rendering would lag typing).
pub const LIVE_MAX_BYTES: usize = 8 << 20;
const BASE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Live-preview bookkeeping of one follower slot.
#[derive(Default)]
pub(crate) struct LiveState {
    /// File and buffer version shown (or being rendered).
    shown: Option<(VfsUri, u64)>,
    /// File and version waiting for the typing pause to end.
    wanted: Option<(VfsUri, u64)>,
    /// When the wanted version renders.
    pub(crate) due: Option<Instant>,
    vfs: Option<Arc<SnapshotVfs>>,
}

impl std::fmt::Debug for LiveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveState")
            .field("shown", &self.shown)
            .field("due", &self.due)
            .finish()
    }
}

impl LiveState {
    /// The text being shown, as a backend (tables read their rows from it).
    pub(crate) fn snapshot(&self) -> Option<(Arc<dyn Vfs>, Entry)> {
        let vfs = self.vfs.clone()?;
        let entry = vfs.entry();
        Some((vfs, entry))
    }
}

/// The pause after the last keystroke before a re-render: longer for bigger texts.
fn debounce(bytes: usize) -> Duration {
    BASE_DEBOUNCE + Duration::from_millis(((bytes >> 20) as u64).saturating_mul(100))
}

impl Session {
    /// Whether `src` can be followed live by `follower`: another Preview pane, in Edit
    /// mode, that is not itself a live follower (no chains).
    fn live_source_ok(&self, follower: PaneId, src: PaneId) -> bool {
        src != follower
            && self.layout.state(src).is_some_and(|st| {
                st.kind == PaneKind::Preview
                    && !matches!(st.preview_source, PreviewSource::Editor(_))
            })
            && !self.editors.docs.contains_key(&follower)
    }

    /// One poll for a live follower: re-render when the editor's text changed and the
    /// typing pause is over. An editor that left Edit mode shows its file from disk.
    pub(crate) fn live_preview_tick(&mut self, pane: PaneId, src: PaneId, now: Instant) -> bool {
        if !self.live_source_ok(pane, src) {
            if let Some(st) = self.layout.state_mut(pane) {
                st.preview_source = PreviewSource::Follow;
            }
            if let Some(slot) = self.previews.slots.get_mut(&pane) {
                slot.live = None;
            }
            return true;
        }
        let slot = self
            .previews
            .slots
            .entry(pane)
            .or_insert_with(PreviewSlot::new);
        slot.target = None;
        let snapshot = self.editors.docs.get(&src).map(|doc| {
            (
                doc.phase != Phase::Loading && doc.blocked.is_none() && doc.load_error.is_none(),
                doc.uri.clone(),
                doc.buffer.version(),
                doc.buffer.len_bytes(),
            )
        });
        let Some((ready, uri, version, bytes)) = snapshot else {
            // The source pane shows its file as a preview: so does the follower.
            if let Some(slot) = self.previews.slots.get_mut(&pane) {
                slot.live = None;
            }
            let desired: Vec<Entry> = self.preview_subject(src).into_iter().collect();
            return self.advance_slot(pane, desired, now);
        };
        if !ready {
            return false;
        }
        let key = (uri.clone(), version);
        let live = slot.live.get_or_insert_with(LiveState::default);
        if live.shown.as_ref() == Some(&key) {
            return false;
        }
        if bytes > LIVE_MAX_BYTES {
            live.shown = Some(key);
            live.due = None;
            slot.body = Body::Static(Arc::new(Preview::Error(format!(
                "Live preview is off for files over {} MiB",
                LIVE_MAX_BYTES >> 20
            ))));
            return true;
        }
        if live.wanted.as_ref() != Some(&key) {
            // The first render of a file comes at once; later ones wait for a pause.
            let first = live.shown.as_ref().is_none_or(|(u, _)| *u != uri);
            live.wanted = Some(key);
            live.due = Some(if first { now } else { now + debounce(bytes) });
        }
        if live.due.is_some_and(|due| due > now) {
            return false;
        }
        self.start_live_render(pane, src)
    }

    fn start_live_render(&mut self, pane: PaneId, src: PaneId) -> bool {
        let Some(doc) = self.editors.docs.get(&src) else {
            return false;
        };
        let text = doc.buffer.to_text();
        let (uri, version) = (doc.uri.clone(), doc.buffer.version());
        let vfs = Arc::new(SnapshotVfs::new(uri.clone(), &text));
        drop(text);
        let entry = vfs.entry();
        let token = self.previews.token();
        let (log_limits, filters, budget) = (
            self.prefs.log_limits(),
            self.prefs.preview_filters.clone(),
            self.prefs.preview_budget(),
        );
        let Some(slot) = self.previews.slots.get_mut(&pane) else {
            return false;
        };
        let live = slot.live.get_or_insert_with(LiveState::default);
        live.shown = Some((uri, version));
        live.wanted = None;
        live.due = None;
        live.vfs = Some(vfs.clone());
        slot.token = token;
        slot.subject = Some(entry.clone());
        slot.subjects = vec![entry.clone()];
        if matches!(slot.body, Body::Empty) {
            slot.body = Body::Loading;
        }
        let inbox = self.previews.inbox.clone();
        let spawned = std::thread::Builder::new()
            .name("fm-live-preview".into())
            .spawn(move || {
                let vfs: Arc<dyn Vfs> = vfs;
                let preview = crate::preview_cmds::render_budgeted(
                    vfs,
                    entry,
                    Limits::default(),
                    false,
                    false,
                    log_limits,
                    filters,
                    budget,
                );
                PreviewHub::push(
                    &inbox,
                    Msg::Rendered {
                        pane,
                        token,
                        preview: Box::new(preview),
                    },
                );
            });
        if spawned.is_err()
            && let Some(slot) = self.previews.slots.get_mut(&pane)
        {
            slot.body = Body::Static(Arc::new(Preview::Error(
                "could not start the live preview".into(),
            )));
        }
        true
    }

    /// Opens (or reuses) a Preview pane that follows editor `src` live; focus stays in
    /// the editor.
    pub(crate) fn open_live_preview(&mut self, src: PaneId) -> Result<PaneId, FmError> {
        let panes = self.layout.panes();
        let follower = panes
            .iter()
            .copied()
            .find(|p| {
                self.layout
                    .state(*p)
                    .is_some_and(|st| st.preview_source == PreviewSource::Editor(src))
            })
            .or_else(|| {
                panes.iter().copied().find(|p| {
                    *p != src
                        && !self.editors.docs.contains_key(p)
                        && self.layout.state(*p).is_some_and(|st| {
                            st.kind == PaneKind::Preview
                                && !matches!(st.preview_source, PreviewSource::Editor(_))
                        })
                })
            });
        let follower = match follower {
            Some(p) => p,
            None => {
                let p = self.layout.split(src, Axis::Horizontal, 0.5)?;
                if let Some(st) = self.layout.state_mut(p) {
                    st.kind = PaneKind::Preview;
                    st.edit = None;
                }
                p
            }
        };
        if !self.live_source_ok(follower, src) {
            return Err(FmError::InvalidParams(
                "this pane cannot show a live preview of that editor".into(),
            ));
        }
        if let Some(st) = self.layout.state_mut(follower) {
            st.preview_source = PreviewSource::Editor(src);
        }
        if let Some(slot) = self.previews.slots.get_mut(&follower) {
            slot.live = None;
        }
        self.layout.focus_pane(src)?;
        Ok(follower)
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "editor.open_live_preview",
        label: "Live Preview",
        menu: &["File", "Editor", "Live Preview"],
        shortcut: None,
        params: "{pane?: number}: show the editor's unsaved text rendered in another Preview pane",
        enabled: |s| {
            s.editor(s.layout.focused())
                .is_some_and(|d| d.blocked.is_none())
        },
        run: |s, p: &Params| {
            let map = crate::file_cmds::object(p)?;
            if let Some(key) = map.and_then(|m| m.keys().find(|k| *k != "pane")) {
                return Err(FmError::InvalidParams(format!("unknown parameter {key:?}")));
            }
            let pane = match map.and_then(|m| m.get("pane")) {
                None => s.layout.focused(),
                Some(v) => v
                    .as_u64()
                    .ok_or_else(|| FmError::InvalidParams("pane: number".into()))?,
            };
            if !s.editors.docs.contains_key(&pane) {
                return Err(FmError::Unsupported(
                    "the focused pane is not editing a file",
                ));
            }
            s.open_live_preview(pane).map(|_| ())
        },
    }]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use fm_testkit::MemFs;
    use serde_json::json;

    fn settle(s: &mut Session, what: &str, until: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !until(s) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn markdown_heading(s: &Session, pane: PaneId) -> Option<String> {
        match &s.previews.slot(pane)?.body {
            Body::Static(p) => match &**p {
                Preview::Markdown(doc) => Some(format!("{doc:?}")),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn a_follower_renders_the_unsaved_text_and_reverts_when_the_editor_closes() {
        let mem = MemFs::new();
        let file = VfsUri::parse("/w/a.md").expect("uri");
        mem.put_file(&file, b"# Saved\n").expect("put");
        let mut s = Session::new().expect("session");
        s.backends.register(Arc::new(mem));
        s.dispatch("editor.open", &json!({ "uri": file.to_string() }))
            .expect("open");
        let editor = s.layout.focused();
        settle(&mut s, "load", |s| {
            s.editor(editor).is_some_and(|d| d.phase == Phase::Ready)
        });
        s.dispatch("editor.open_live_preview", &json!(null))
            .expect("live");
        assert_eq!(s.layout.focused(), editor, "focus stays in the editor");
        let follower = s
            .layout
            .panes()
            .into_iter()
            .find(|p| {
                s.layout.state(*p).map(|st| st.preview_source)
                    == Some(PreviewSource::Editor(editor))
            })
            .expect("follower");
        s.dispatch(
            "editor.input",
            &json!({"op": "insert", "text": "# Unsaved draft\n"}),
        )
        .expect("type");
        settle(&mut s, "live render", |s| {
            markdown_heading(s, follower).is_some_and(|d| d.contains("Unsaved draft"))
        });
        assert!(s.editor(editor).expect("doc").dirty(), "nothing was saved");
        // A chain is refused; following itself is refused.
        assert!(!s.live_source_ok(follower, follower));
        s.layout.close(editor).expect("close editor pane");
        s.poll();
        assert_eq!(
            s.layout.state(follower).expect("follower").preview_source,
            PreviewSource::Follow
        );
    }

    #[test]
    fn the_command_needs_an_editor_and_takes_only_pane() {
        let mut s = Session::new().expect("session");
        assert!(
            s.dispatch("editor.open_live_preview", &json!(null))
                .is_err()
        );
        assert!(matches!(
            s.dispatch("editor.open_live_preview", &json!({"bogus": 1})),
            Err(FmError::InvalidParams(_))
        ));
        assert_eq!(debounce(0), BASE_DEBOUNCE);
        assert!(debounce(4 << 20) > debounce(1 << 20));
    }
}
