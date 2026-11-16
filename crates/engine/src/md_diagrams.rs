//! Mermaid diagrams of the Markdown preview (change `markdown-mermaid`, capabilities
//! `preview-pane` and `security`).
//!
//! Like [`crate::md_images`]: the UI never draws anything itself, it asks
//! [`MdDiagramService::state`] what it may show, and `preview.md_diagrams.want` queues what
//! is missing. One worker thread draws at most one diagram at a time with at most
//! [`MAX_QUEUED`] waiting; work for a document a pane no longer shows is dropped (and a
//! running render cancelled); drawn diagrams are cached within [`MEMORY_BYTES`]. A finished
//! diagram wakes the UI through the waker set by the app.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crossbeam_channel::{Receiver, Sender};
use fm_layout::{PaneId, PaneKind};
use fm_preview::markdown::MdBlock;
use fm_preview::mermaid::{self, Cancel, Diagram, DiagramError, Palette};
use fm_preview::{Limits, Preview};
use fm_vfs::{FmError, VfsUri};
use serde_json::Value;

use crate::preview_cmds::Body;
use crate::registry::CommandSpec;

/// Drawn diagrams kept in memory (least recently drawn go first).
pub const MEMORY_BYTES: usize = 64 << 20;
const MAX_ENTRIES: usize = 256;
/// Diagrams waiting for the worker; further requests are refused until it catches up (the
/// preview asks again on a later frame).
pub const MAX_QUEUED: usize = 32;
/// Accepted drawing scales (preview zoom × pixels per point).
const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.25..=8.0;

/// Cache key: source, palette and scale in steps of 0.25.
type Key = (u64, u64, u32);

fn key(source: &str, palette: &Palette, scale: f32) -> Key {
    (
        mermaid::source_key(source),
        palette.key(),
        scale_step(scale),
    )
}

fn scale_step(scale: f32) -> u32 {
    (scale.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end()) * 4.0).round() as u32
}

/// What the preview may show for one diagram.
#[derive(Clone, Debug)]
pub enum MdDiagramState {
    Pending,
    Ready(Arc<Diagram>),
    /// The short reason shown after "Diagram not drawn: ".
    Failed(String),
}

#[derive(Debug)]
enum Slot {
    Pending,
    Ready(Arc<Diagram>),
    Failed(String),
}

#[derive(Debug)]
struct Item {
    slot: Slot,
    used: u64,
}

/// Which document a pane shows, and a counter that changes when it does.
type Shown = (VfsUri, Option<SystemTime>);

#[derive(Debug, Default)]
struct Store {
    map: HashMap<Key, Item>,
    bytes: usize,
    tick: u64,
    panes: HashMap<PaneId, (Shown, u64)>,
}

impl Store {
    fn touch(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    fn evict(&mut self) {
        while self.bytes > MEMORY_BYTES || self.map.len() > MAX_ENTRIES {
            let oldest = self
                .map
                .iter()
                .filter(|(_, i)| !matches!(i.slot, Slot::Pending))
                .min_by_key(|(_, i)| i.used)
                .map(|(k, _)| *k);
            let Some(key) = oldest else { break };
            if let Some(Item {
                slot: Slot::Ready(d),
                ..
            }) = self.map.remove(&key)
            {
                self.bytes = self.bytes.saturating_sub(d.thumb.bytes());
            }
        }
    }

    fn epoch(&self, pane: PaneId) -> u64 {
        self.panes.get(&pane).map_or(0, |(_, epoch)| *epoch)
    }
}

struct Job {
    key: Key,
    source: String,
    palette: Palette,
    scale: f32,
    pane: PaneId,
    epoch: u64,
}

type Waker = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    store: Mutex<Store>,
    queued: AtomicUsize,
    /// The job being drawn, so a document change can cancel it.
    running: Mutex<Option<(PaneId, u64, Cancel)>>,
    waker: Mutex<Option<Waker>>,
    limits: Limits,
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wake(&self) {
        let waker = self.waker.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(wake) = waker {
            wake();
        }
    }

    fn finish(&self, key: Key, result: Result<Diagram, DiagramError>) {
        let mut st = self.lock();
        let used = st.touch();
        match result {
            // Cancelled work is forgotten so the preview can ask for it again later.
            Err(DiagramError::Cancelled) => {
                st.map.remove(&key);
            }
            Ok(diagram) => {
                st.bytes += diagram.thumb.bytes();
                let slot = Slot::Ready(Arc::new(diagram));
                st.map.insert(key, Item { slot, used });
            }
            Err(why) => {
                let slot = Slot::Failed(why.to_string());
                st.map.insert(key, Item { slot, used });
            }
        }
        st.evict();
        drop(st);
        self.wake();
    }

    fn run(&self, job: Job) {
        self.queued.fetch_sub(1, Ordering::AcqRel);
        if self.lock().epoch(job.pane) != job.epoch {
            self.finish(job.key, Err(DiagramError::Cancelled));
            return;
        }
        let cancel = Cancel::default();
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((job.pane, job.epoch, cancel.clone()));
        // `mermaid::render` catches renderer panics; this guards the rest of the job.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mermaid::render(&job.source, &job.palette, job.scale, &self.limits, &cancel)
        }))
        .unwrap_or_else(|_| Err(DiagramError::Failed("the diagram renderer failed".into())));
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.finish(job.key, result);
    }
}

fn worker(inner: &Inner, rx: &Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        inner.run(job);
    }
}

pub struct MdDiagramService {
    inner: Arc<Inner>,
    sender: Mutex<Option<Sender<Job>>>,
}

impl std::fmt::Debug for MdDiagramService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdDiagramService")
            .field("queued", &self.inner.queued.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for MdDiagramService {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                store: Mutex::new(Store::default()),
                queued: AtomicUsize::new(0),
                running: Mutex::new(None),
                waker: Mutex::new(None),
                limits: Limits::default(),
            }),
            sender: Mutex::new(None),
        }
    }
}

impl MdDiagramService {
    /// Called from the worker when a diagram is ready, so an idle UI wakes.
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.inner.waker.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(wake));
    }

    /// What may be shown for `source`; `None` when it was not requested yet.
    pub fn state(&self, source: &str, palette: &Palette, scale: f32) -> Option<MdDiagramState> {
        if source.len() > mermaid::MAX_SOURCE {
            return Some(MdDiagramState::Failed(DiagramError::TooLarge.to_string()));
        }
        let mut st = self.inner.lock();
        let tick = st.touch();
        let item = st.map.get_mut(&key(source, palette, scale))?;
        item.used = tick;
        Some(match &item.slot {
            Slot::Pending => MdDiagramState::Pending,
            Slot::Ready(d) => MdDiagramState::Ready(d.clone()),
            Slot::Failed(why) => MdDiagramState::Failed(why.clone()),
        })
    }

    /// Whether `source` failed to draw (at any palette or scale): it is then shown as code.
    pub fn has_failed(&self, source: &str) -> bool {
        if source.len() > mermaid::MAX_SOURCE {
            return true;
        }
        let wanted = mermaid::source_key(source);
        self.inner
            .lock()
            .map
            .iter()
            .any(|((k, _, _), item)| *k == wanted && matches!(item.slot, Slot::Failed(_)))
    }

    /// `pane` now shows `shown`: queued work for its previous document is dropped and a
    /// running render for it is cancelled.
    fn note_shown(&self, pane: PaneId, shown: &Shown) -> u64 {
        let mut st = self.inner.lock();
        let epoch = match st.panes.get(&pane) {
            Some((old, epoch)) if old == shown => return *epoch,
            Some((_, epoch)) => epoch + 1,
            None => 1,
        };
        st.panes.insert(pane, (shown.clone(), epoch));
        drop(st);
        if let Some((running_pane, running_epoch, cancel)) =
            &*self.inner.running.lock().unwrap_or_else(|p| p.into_inner())
            && *running_pane == pane
            && *running_epoch != epoch
        {
            cancel.cancel();
        }
        epoch
    }

    /// Queue the diagrams not known yet. Returns how many were queued.
    fn want(
        &self,
        pane: PaneId,
        shown: &Shown,
        sources: &[&str],
        palette: &Palette,
        scale: f32,
    ) -> usize {
        let epoch = self.note_shown(pane, shown);
        let mut queued = 0;
        for source in sources {
            if source.len() > mermaid::MAX_SOURCE
                || self.inner.queued.load(Ordering::Acquire) >= MAX_QUEUED
            {
                continue;
            }
            let key = key(source, palette, scale);
            {
                let mut st = self.inner.lock();
                if st.map.contains_key(&key) {
                    continue;
                }
                let used = st.touch();
                st.map.insert(
                    key,
                    Item {
                        slot: Slot::Pending,
                        used,
                    },
                );
            }
            self.inner.queued.fetch_add(1, Ordering::AcqRel);
            let job = Job {
                key,
                source: (*source).to_owned(),
                palette: palette.clone(),
                scale,
                pane,
                epoch,
            };
            if self.sender().is_some_and(|tx| tx.send(job).is_ok()) {
                queued += 1;
            } else {
                self.inner.queued.fetch_sub(1, Ordering::AcqRel);
                self.inner.finish(
                    key,
                    Err(DiagramError::Failed(
                        "the diagram worker is not running".into(),
                    )),
                );
            }
        }
        queued
    }

    pub fn is_busy(&self) -> bool {
        self.inner.queued.load(Ordering::Acquire) > 0
            || self
                .inner
                .running
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_some()
    }

    fn sender(&self) -> Option<Sender<Job>> {
        let mut slot = self.sender.lock().ok()?;
        if slot.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded::<Job>();
            let inner = self.inner.clone();
            std::thread::Builder::new()
                .name("fm-mermaid".into())
                .spawn(move || worker(&inner, &rx))
                .ok()?;
            *slot = Some(tx);
        }
        slot.clone()
    }
}

/// The Mermaid sources of a document, in order, as (code-block index, source). Only the
/// first [`mermaid::MAX_PER_DOCUMENT`] are drawn; the preview shows the rest as code.
pub fn mermaid_blocks(blocks: &[MdBlock]) -> Vec<(usize, &str)> {
    fn walk<'a>(blocks: &'a [MdBlock], index: &mut usize, out: &mut Vec<(usize, &'a str)>) {
        for block in blocks {
            match block {
                MdBlock::Code(code) => {
                    if mermaid::is_mermaid(&code.language) {
                        out.push((*index, code.text.as_str()));
                    }
                    *index += 1;
                }
                MdBlock::Quote(children) | MdBlock::Centered(children) => {
                    walk(children, index, out);
                }
                MdBlock::List { items, .. } => {
                    for item in items {
                        walk(&item.blocks, index, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, &mut 0, &mut out);
    out
}

fn palette_param(value: Option<&Value>) -> Result<Palette, FmError> {
    let map = value
        .and_then(Value::as_object)
        .ok_or_else(|| FmError::InvalidParams("palette: object expected".into()))?;
    let colour = |name: &str| -> Result<String, FmError> {
        map.get(name)
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 16)
            .map(str::to_owned)
            .ok_or_else(|| FmError::InvalidParams(format!("palette.{name}: colour expected")))
    };
    let palette = Palette {
        dark: map
            .get("dark")
            .and_then(Value::as_bool)
            .ok_or_else(|| FmError::InvalidParams("palette.dark: true or false expected".into()))?,
        background: colour("background")?,
        node_fill: colour("node_fill")?,
        node_border: colour("node_border")?,
        text: colour("text")?,
        line: colour("line")?,
        cluster_fill: colour("cluster_fill")?,
        cluster_border: colour("cluster_border")?,
        label_background: colour("label_background")?,
        note_fill: colour("note_fill")?,
        note_border: colour("note_border")?,
    };
    palette.validate().map_err(FmError::InvalidParams)?;
    Ok(palette)
}

fn scale_param(value: Option<&Value>) -> Result<f32, FmError> {
    value
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .filter(|v| SCALE_RANGE.contains(v))
        .ok_or_else(|| FmError::InvalidParams("scale: number 0.25-8 expected".into()))
}

/// A palette as command parameters (what the UI sends).
pub fn palette_json(palette: &Palette) -> Value {
    serde_json::json!({
        "dark": palette.dark,
        "background": palette.background,
        "node_fill": palette.node_fill,
        "node_border": palette.node_border,
        "text": palette.text,
        "line": palette.line,
        "cluster_fill": palette.cluster_fill,
        "cluster_border": palette.cluster_border,
        "label_background": palette.label_background,
        "note_fill": palette.note_fill,
        "note_border": palette.note_border,
    })
}

/// The pane of `params` (default: focused) and the Mermaid blocks of the Markdown document
/// it shows that may be drawn (the first [`mermaid::MAX_PER_DOCUMENT`]).
fn previewed_blocks(
    s: &crate::Session,
    map: &serde_json::Map<String, Value>,
) -> Result<(PaneId, Vec<(usize, String)>), FmError> {
    let pane = match map.get("pane") {
        Some(v) => v
            .as_u64()
            .ok_or_else(|| FmError::InvalidParams("pane: pane id expected".into()))?,
        None => s.layout.focused(),
    };
    if s.pane_kind(pane) != Some(PaneKind::Preview) {
        return Err(FmError::InvalidParams("pane: not a preview pane".into()));
    }
    let slot = s
        .preview_slot(pane)
        .ok_or(FmError::Unsupported("nothing is previewed"))?;
    let (Body::Static(preview), Some(_)) = (&slot.body, &slot.subject) else {
        return Err(FmError::Unsupported("not a Markdown preview"));
    };
    let Preview::Markdown(doc) = &**preview else {
        return Err(FmError::Unsupported("not a Markdown preview"));
    };
    let blocks = mermaid_blocks(&doc.blocks)
        .into_iter()
        .take(mermaid::MAX_PER_DOCUMENT)
        .map(|(i, source)| (i, source.to_owned()))
        .collect();
    Ok((pane, blocks))
}

/// `preview.md_diagrams.toggle_source {pane?, block}`: show one diagram as its source code,
/// or draw it again.
fn toggle_source() -> CommandSpec {
    CommandSpec {
        id: "preview.md_diagrams.toggle_source",
        label: "Show Diagram Source",
        menu: &["Preview", "Show Diagram Source"],
        shortcut: None,
        params: "{pane?: number, block: code-block index of a drawable Mermaid block}: switch that block between diagram and source code",
        enabled: |s| s.prefs.preview_mermaid,
        run: |s, params| {
            let map = crate::file_cmds::object(params)?
                .ok_or_else(|| FmError::InvalidParams("parameters object expected".into()))?;
            let block = map
                .get("block")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| FmError::InvalidParams("block: index expected".into()))?;
            if !s.prefs.preview_mermaid {
                return Err(FmError::Unsupported("Mermaid diagrams are turned off"));
            }
            let (pane, drawable) = previewed_blocks(s, map)?;
            if !drawable.iter().any(|(i, _)| *i == block) {
                return Err(FmError::InvalidParams(format!(
                    "block: {block} is not a drawable Mermaid block"
                )));
            }
            let slot = s
                .previews
                .slots
                .get_mut(&pane)
                .ok_or(FmError::Unsupported("nothing is previewed"))?;
            let uri = slot
                .subject
                .as_ref()
                .map(|e| e.uri.clone())
                .ok_or(FmError::Unsupported("nothing is previewed"))?;
            let mut blocks = slot.diagram_sources().to_vec();
            if let Some(at) = blocks.iter().position(|b| *b == block) {
                blocks.remove(at);
            } else {
                blocks.push(block);
            }
            slot.diagram_source = Some((uri, blocks));
            Ok(())
        },
    }
}

/// `preview.md_diagrams.copy_image {pane?, block, palette, scale}`: put a drawn diagram on
/// the clipboard as an opaque picture with a margin of its background colour.
fn copy_image() -> CommandSpec {
    CommandSpec {
        id: "preview.md_diagrams.copy_image",
        label: "Copy Diagram as Image",
        menu: &["Preview", "Copy Diagram as Image"],
        shortcut: None,
        params: "{pane?: number, block: code-block index of a drawn Mermaid block, palette: as for preview.md_diagrams.want, scale: number 0.25-8}",
        enabled: |s| s.prefs.preview_mermaid,
        run: |s, params| {
            let map = crate::file_cmds::object(params)?
                .ok_or_else(|| FmError::InvalidParams("parameters object expected".into()))?;
            let block = map
                .get("block")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| FmError::InvalidParams("block: index expected".into()))?;
            let palette = palette_param(map.get("palette"))?;
            let scale = scale_param(map.get("scale"))?;
            if !s.prefs.preview_mermaid {
                return Err(FmError::Unsupported("Mermaid diagrams are turned off"));
            }
            let (_, drawable) = previewed_blocks(s, map)?;
            let (_, source) = drawable.iter().find(|(i, _)| *i == block).ok_or_else(|| {
                FmError::InvalidParams(format!("block: {block} is not a drawable Mermaid block"))
            })?;
            let Some(MdDiagramState::Ready(drawn)) = s.md_diagrams.state(source, &palette, scale)
            else {
                return Err(FmError::Unsupported("the diagram is not drawn yet"));
            };
            let pad = (16.0 * scale).round() as u32;
            let flat = mermaid::flatten(&drawn.thumb, &palette.background, pad)
                .map_err(|_| FmError::Unsupported("the diagram could not be copied"))?;
            s.clipboard_image = Some(crate::image_clip_cmds::ClipImage {
                width: flat.width as usize,
                height: flat.height as usize,
                rgba: flat.rgba,
            });
            s.info = Some((
                "Copied diagram to the clipboard as an image".to_string(),
                std::time::Instant::now(),
            ));
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        toggle_source(),
        copy_image(),
        CommandSpec {
            id: "preview.md_diagrams.want",
            label: "Draw Mermaid Diagrams",
            menu: &["Preview", "Draw Mermaid Diagrams"],
            shortcut: None,
            params: "{pane?: number, blocks: [code-block index] (Mermaid blocks, at most 50), palette: {dark, background, node_fill, node_border, text, line, cluster_fill, cluster_border, label_background, note_fill, note_border} (#rrggbb), scale: number 0.25-8}: queue the diagrams of the previewed Markdown file",
            enabled: |s| s.prefs.preview_mermaid,
            run: |s, params| {
                let map = crate::file_cmds::object(params)?
                    .ok_or_else(|| FmError::InvalidParams("parameters object expected".into()))?;
                if !s.prefs.preview_mermaid {
                    return Err(FmError::Unsupported("Mermaid diagrams are turned off"));
                }
                let indices = map
                    .get("blocks")
                    .and_then(Value::as_array)
                    .filter(|a| a.len() <= mermaid::MAX_PER_DOCUMENT)
                    .ok_or_else(|| {
                        FmError::InvalidParams(
                            "blocks: array of at most 50 indices expected".into(),
                        )
                    })?
                    .iter()
                    .map(|v| {
                        v.as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| FmError::InvalidParams("blocks: index expected".into()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let palette = palette_param(map.get("palette"))?;
                let scale = scale_param(map.get("scale"))?;
                let (pane, drawable) = previewed_blocks(s, map)?;
                let mut sources = Vec::with_capacity(indices.len());
                for index in indices {
                    let (_, source) =
                        drawable.iter().find(|(i, _)| *i == index).ok_or_else(|| {
                            FmError::InvalidParams(format!(
                                "blocks: {index} is not a drawable Mermaid block"
                            ))
                        })?;
                    sources.push(source.as_str());
                }
                let subject = s
                    .preview_slot(pane)
                    .and_then(|slot| slot.subject.as_ref())
                    .ok_or(FmError::Unsupported("nothing is previewed"))?;
                let shown = (subject.uri.clone(), subject.modified);
                s.md_diagrams.want(pane, &shown, &sources, &palette, scale);
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use crate::Session;
    use crate::preview_cmds::PreviewSlot;
    use fm_preview::markdown::{MdDoc, parse};
    use fm_vfs::{Entry, EntryKind};
    use serde_json::json;
    use std::time::{Duration, Instant};

    pub(crate) fn palette(dark: bool) -> Palette {
        Palette {
            dark,
            background: "#ffffff".into(),
            node_fill: "#eef2f8".into(),
            node_border: "#5b8def".into(),
            text: "#1b1f24".into(),
            line: "#8a94a6".into(),
            cluster_fill: "#f6f8fb".into(),
            cluster_border: "#7a8597".into(),
            label_background: "#ffffff".into(),
            note_fill: "#fff5c2".into(),
            note_border: "#c9a400".into(),
        }
    }

    const DOC: &str = "# Title\n\n```rust\nfn x() {}\n```\n\n```mermaid\nflowchart LR\nA-->B\n```\n\n> ```Mermaid\n> sequenceDiagram\n> A->>B: hi\n> ```\n\n```mermaid\nflowchart LR\n A[[[ --> }}}\n```";

    fn previewing(markdown: &str, name: &str) -> (Session, PaneId) {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        let doc = Arc::new(Preview::Markdown(MdDoc {
            blocks: parse(markdown),
            source: Default::default(),
            truncated: false,
            total_bytes: 0,
        }));
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(doc);
        slot.subject = Some(Entry::new(
            VfsUri::parse(&format!("/tmp/{name}.md")).expect("uri"),
            EntryKind::File,
        ));
        s.previews.slots.insert(pane, slot);
        (s, pane)
    }

    fn want(s: &mut Session, blocks: Value) -> Result<(), FmError> {
        s.dispatch(
            "preview.md_diagrams.want",
            &json!({"blocks": blocks, "palette": palette_json(&palette(false)), "scale": 1.0}),
        )
    }

    fn settle(s: &Session) {
        let end = Instant::now() + Duration::from_secs(20);
        while s.md_diagrams.is_busy() {
            assert!(Instant::now() < end, "diagrams never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn source_of(s: &Session, pane: PaneId, n: usize) -> String {
        let Body::Static(p) = &s.preview_slot(pane).expect("slot").body else {
            panic!("static")
        };
        let Preview::Markdown(doc) = &**p else {
            panic!("md")
        };
        mermaid_blocks(&doc.blocks)[n].1.to_owned()
    }

    #[test]
    fn mermaid_blocks_are_found_with_their_code_block_index() {
        let blocks = parse(DOC);
        let found = mermaid_blocks(&blocks);
        assert_eq!(found.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(found[1].1.starts_with("sequenceDiagram"));
    }

    #[test]
    fn diagrams_go_from_pending_to_ready_or_failed_and_wake_the_ui() {
        let (mut s, pane) = previewing(DOC, "a");
        let woke = Arc::new(AtomicUsize::new(0));
        let counter = woke.clone();
        s.md_diagrams.set_waker(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let p = palette(false);
        let good = source_of(&s, pane, 0);
        let bad = source_of(&s, pane, 2);
        assert!(
            s.md_diagrams.state(&good, &p, 1.0).is_none(),
            "not requested yet"
        );
        want(&mut s, json!([1, 3])).expect("want");
        assert!(matches!(
            s.md_diagrams.state(&good, &p, 1.0),
            Some(MdDiagramState::Pending | MdDiagramState::Ready(_))
        ));
        settle(&s);
        assert!(matches!(
            s.md_diagrams.state(&good, &p, 1.0),
            Some(MdDiagramState::Ready(_))
        ));
        let Some(MdDiagramState::Failed(why)) = s.md_diagrams.state(&bad, &p, 1.0) else {
            panic!("broken diagram fails")
        };
        assert!(!why.is_empty());
        assert!(
            woke.load(Ordering::Relaxed) >= 2,
            "one wake per finished diagram"
        );
        // Another scale step is another bitmap; the same step is cached.
        assert!(
            s.md_diagrams.state(&good, &p, 1.1).is_some(),
            "1.1 rounds to 1.0"
        );
        assert!(s.md_diagrams.state(&good, &p, 2.0).is_none());
    }

    #[test]
    fn bad_params_are_errors_and_queue_nothing() {
        let (mut s, _) = previewing(DOC, "b");
        let base =
            || json!({"blocks": [1], "palette": palette_json(&palette(false)), "scale": 1.0});
        let mut cases = Vec::new();
        for (field, value) in [
            ("blocks", json!([0])),
            ("blocks", json!([99])),
            ("blocks", json!("1")),
            ("blocks", json!((0..51).collect::<Vec<_>>())),
            ("scale", json!(0.0)),
            ("scale", json!(100.0)),
            ("scale", json!("big")),
            ("pane", json!(999_999)),
        ] {
            let mut p = base();
            p[field] = value;
            cases.push(p);
        }
        for colour in ["red; url(x)", "#12345", "", "#1234567"] {
            let mut p = base();
            p["palette"]["line"] = json!(colour);
            cases.push(p);
        }
        let mut no_dark = base();
        no_dark["palette"]
            .as_object_mut()
            .expect("obj")
            .remove("dark");
        cases.push(no_dark);
        cases.push(json!(null));
        for params in cases {
            assert!(
                s.dispatch("preview.md_diagrams.want", &params).is_err(),
                "{params}"
            );
        }
        assert!(!s.md_diagrams.is_busy(), "nothing queued");
    }

    #[test]
    fn show_source_toggles_one_block_per_document() {
        let (mut s, pane) = previewing(DOC, "t");
        let toggle = |s: &mut Session, params: Value| {
            s.dispatch("preview.md_diagrams.toggle_source", &params)
        };
        toggle(&mut s, json!({"block": 3})).expect("show source");
        toggle(&mut s, json!({"block": 1, "pane": pane})).expect("show source");
        let slot = s.preview_slot(pane).expect("slot");
        assert_eq!(slot.diagram_sources(), [3, 1]);
        toggle(&mut s, json!({"block": 3})).expect("draw again");
        assert_eq!(s.preview_slot(pane).expect("slot").diagram_sources(), [1]);
        for bad in [
            json!({"block": 0}),
            json!({"block": 99}),
            json!({"block": "1"}),
            json!({}),
            json!({"block": 1, "pane": 999_999}),
            json!(null),
        ] {
            assert!(toggle(&mut s, bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(s.preview_slot(pane).expect("slot").diagram_sources(), [1]);
        // Another file starts with every diagram drawn.
        s.previews.slots.get_mut(&pane).expect("slot").subject = Some(Entry::new(
            VfsUri::parse("/tmp/other.md").expect("uri"),
            EntryKind::File,
        ));
        assert!(
            s.preview_slot(pane)
                .expect("slot")
                .diagram_sources()
                .is_empty()
        );
        s.prefs.preview_mermaid = false;
        assert!(toggle(&mut s, json!({"block": 1})).is_err());
    }

    #[test]
    fn a_drawn_diagram_is_copied_as_an_opaque_image() {
        let (mut s, _) = previewing(DOC, "img");
        let params = |block: usize, scale: f64| json!({"block": block, "palette": palette_json(&palette(true)), "scale": scale});
        let copy = |s: &mut Session, p: Value| s.dispatch("preview.md_diagrams.copy_image", &p);
        assert!(copy(&mut s, params(1, 1.0)).is_err(), "not drawn yet");
        s.dispatch(
            "preview.md_diagrams.want",
            &json!({"blocks": [1, 3], "palette": palette_json(&palette(true)), "scale": 1.0}),
        )
        .expect("want");
        settle(&s);
        copy(&mut s, params(1, 1.0)).expect("copy");
        let image = s.clipboard_image.take().expect("image on the clipboard");
        assert!(image.width > 32 && image.height > 32);
        assert_eq!(image.rgba.len(), image.width * image.height * 4);
        assert!(
            image.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255),
            "opaque"
        );
        assert_eq!(
            &image.rgba[..3],
            &[0xff, 0xff, 0xff],
            "margin in the background colour"
        );
        assert!(s.info.is_some(), "confirmation shown");
        let no_palette = json!({"block": 1, "scale": 1.0});
        let no_scale = json!({"block": 1, "palette": palette_json(&palette(true))});
        // A failed diagram, another scale, a non-Mermaid block, missing fields, garbage.
        for bad in [
            params(3, 1.0),
            params(1, 2.0),
            params(0, 1.0),
            no_palette,
            no_scale,
            json!(null),
        ] {
            assert!(copy(&mut s, bad.clone()).is_err(), "{bad}");
        }
        assert!(s.clipboard_image.is_none(), "nothing copied on errors");
    }

    #[test]
    fn turned_off_means_no_drawing() {
        let (mut s, _) = previewing(DOC, "c");
        s.prefs.preview_mermaid = false;
        assert!(!s.command_enabled("preview.md_diagrams.want"));
        assert!(want(&mut s, json!([1])).is_err());
    }

    #[test]
    fn only_the_first_fifty_diagrams_are_drawable() {
        let doc = "```mermaid\nflowchart LR\nA-->B\n```\n\n".repeat(55);
        let (mut s, _) = previewing(&doc, "d");
        assert!(want(&mut s, json!([49])).is_ok());
        assert!(want(&mut s, json!([50])).is_err(), "the 51st stays code");
        settle(&s);
    }

    #[test]
    fn a_new_document_drops_queued_work_and_the_cache_stays_bounded() {
        let svc = MdDiagramService::default();
        let p = palette(false);
        let doc = |n: u32| -> Shown {
            (
                VfsUri::parse(&format!("/tmp/doc{n}.md")).expect("uri"),
                None,
            )
        };
        let sources: Vec<String> = (0..40)
            .map(|n| format!("flowchart LR\nA{n}-->B{n}"))
            .collect();
        let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
        let queued = svc.want(1, &doc(1), &refs, &p, 1.0);
        let waiting = svc.inner.queued.load(Ordering::Acquire);
        assert!(
            waiting <= MAX_QUEUED,
            "at most {MAX_QUEUED} waiting: {waiting}"
        );
        assert!(queued < refs.len(), "requests beyond the queue are refused");
        // The pane moves on: its old work is dropped, not drawn.
        svc.want(1, &doc(2), &[], &p, 1.0);
        let end = Instant::now() + Duration::from_secs(20);
        while svc.is_busy() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        let drawn = refs
            .iter()
            .filter(|src| matches!(svc.state(src, &p, 1.0), Some(MdDiagramState::Ready(_))))
            .count();
        assert!(
            drawn < queued,
            "dropped work was not drawn ({drawn} of {queued})"
        );
        assert!(svc.inner.lock().bytes <= MEMORY_BYTES);
        assert!(svc.inner.lock().map.len() <= MAX_ENTRIES);
    }

    #[test]
    fn oversized_sources_fail_without_queueing() {
        let svc = MdDiagramService::default();
        let huge = format!(
            "flowchart LR\n{}",
            "A-->B\n".repeat(mermaid::MAX_SOURCE / 6 + 1)
        );
        let p = palette(true);
        assert!(matches!(
            svc.state(&huge, &p, 1.0),
            Some(MdDiagramState::Failed(_))
        ));
        assert_eq!(
            svc.want(
                1,
                &(VfsUri::parse("/tmp/x.md").expect("uri"), None),
                &[&huge],
                &p,
                1.0
            ),
            0
        );
        assert!(!svc.is_busy());
    }
}
