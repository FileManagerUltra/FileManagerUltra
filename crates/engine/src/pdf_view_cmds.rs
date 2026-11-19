//! `preview.pdf.*` commands (design D14): navigation, fit and colours, outline, links,
//! the visible window the view reports, passwords, the signatures panel and text
//! selection. Every command rejects malformed or out-of-range parameters with an error
//! and leaves the view unchanged.

use std::sync::Arc;

use fm_layout::PaneId;
use fm_pdf::render::page_pixels;
use fm_pdf::{LinkTarget, MAX_PIXELS, Secret, Tile};
use fm_preview::Preview;
use fm_vfs::FmError;
use serde_json::{Map, Value};

use crate::file_cmds::object;
use crate::pdf_service::{RenderKey, TILE};
use crate::pdf_view::{PdfColors, PdfFit, PdfMsg, PdfSelection, Shown, push};
use crate::preview_cmds::Body;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Longest page label accepted by `preview.pdf.page`.
pub const MAX_LABEL: usize = 64;
/// Longest password accepted.
pub const MAX_PASSWORD: usize = 1024;
/// Pages a view may report as visible at once.
pub const MAX_VISIBLE: usize = 32;
/// Characters copied from a PDF at most.
pub const MAX_COPY_BYTES: usize = 8 << 20;

fn no_pdf() -> FmError {
    FmError::InvalidParams("the preview is not showing a PDF".into())
}

/// The focused (or named) Preview pane, which must show a PDF.
pub(crate) fn pdf_pane(s: &Session, params: &Params) -> Result<PaneId, FmError> {
    let pane = s.preview_pane(params)?;
    if s.pdf_view(pane).is_none() {
        return Err(no_pdf());
    }
    Ok(pane)
}

fn shown(s: &Session, pane: PaneId) -> Result<Shown, FmError> {
    s.pdf_shown(pane)
        .ok_or_else(|| FmError::InvalidParams("the PDF is locked or not loaded".into()))
}

/// Whether the focused pane shows an open PDF (menus and `enabled`).
pub fn pdf_focused(s: &Session) -> bool {
    let pane = s.layout.focused();
    s.focused_kind() == Some(fm_layout::PaneKind::Preview) && s.pdf_shown(pane).is_some()
}

/// Only `pane` (and the listed keys) may appear.
fn keys(params: &Params, allowed: &[&str]) -> Result<Option<Map<String, Value>>, FmError> {
    let Some(map) = object(params)? else {
        return Ok(None);
    };
    for k in map.keys() {
        if k != "pane" && !allowed.contains(&k.as_str()) {
            return Err(FmError::InvalidParams(format!("unknown parameter {k}")));
        }
    }
    Ok(Some(map.clone()))
}

fn index(map: &Map<String, Value>, key: &str, limit: usize) -> Result<usize, FmError> {
    map.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v < limit)
        .ok_or_else(|| FmError::InvalidParams(format!("{key} is missing or out of range")))
}

fn go(s: &mut Session, pane: PaneId, page: usize, top: Option<f32>) -> Result<(), FmError> {
    let n = shown(s, pane)?.doc.page_count();
    if page >= n {
        return Err(FmError::InvalidParams(format!(
            "page {} does not exist",
            page + 1
        )));
    }
    let view = s.pdf_view_mut(pane).ok_or_else(no_pdf)?;
    view.go(page, top);
    Ok(())
}

fn step(s: &mut Session, params: &Params, by: isize) -> Result<(), FmError> {
    keys(params, &[])?;
    let pane = pdf_pane(s, params)?;
    let n = shown(s, pane)?.doc.page_count();
    let cur = s.pdf_view(pane).map_or(0, |v| v.page);
    let next = match by {
        isize::MIN => 0,
        isize::MAX => n.saturating_sub(1),
        d => (cur as isize + d).clamp(0, n as isize - 1) as usize,
    };
    go(s, pane, next, None)
}

fn page(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["page"])?
        .ok_or_else(|| FmError::InvalidParams("page is required".into()))?;
    let pane = pdf_pane(s, params)?;
    let doc = shown(s, pane)?.doc;
    let target = match map.get("page") {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=doc.page_count()).contains(n))
            .map(|n| n - 1),
        Some(Value::String(t)) if t.chars().count() <= MAX_LABEL => doc.info().find_page(t),
        _ => {
            return Err(FmError::InvalidParams(
                "page must be a page number or label".into(),
            ));
        }
    };
    let target = target
        .ok_or_else(|| FmError::InvalidParams("There is no such page in this document.".into()))?;
    go(s, pane, target, None)
}

fn fit(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["mode"])?;
    let pane = pdf_pane(s, params)?;
    let current = s.pdf_view(pane).map_or(PdfFit::Width, |v| v.fit);
    let mode = match map.as_ref().and_then(|m| m.get("mode")) {
        None => {
            let i = PdfFit::ALL.iter().position(|f| *f == current).unwrap_or(0);
            PdfFit::ALL[(i + 1) % PdfFit::ALL.len()]
        }
        Some(v) => v
            .as_str()
            .and_then(PdfFit::from_id)
            .ok_or_else(|| FmError::InvalidParams("mode must be width, page or actual".into()))?,
    };
    if let Some(slot) = s.previews.slots.get_mut(&pane) {
        // Like images: choosing a fit mode drops extra magnification.
        slot.scale = 1.0;
        if let Some(v) = slot.pdf.as_mut() {
            v.fit = mode;
            let page = v.page;
            v.go(page, None);
        }
    }
    s.prefs.pdf_fit = mode;
    Ok(())
}

fn colors(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["mode"])?;
    let pane = pdf_pane(s, params)?;
    let current = s.pdf_view(pane).map_or(PdfColors::Original, |v| v.colors);
    let mode = match map.as_ref().and_then(|m| m.get("mode")) {
        None => {
            let i = PdfColors::ALL
                .iter()
                .position(|c| *c == current)
                .unwrap_or(0);
            PdfColors::ALL[(i + 1) % PdfColors::ALL.len()]
        }
        Some(v) => v.as_str().and_then(PdfColors::from_id).ok_or_else(|| {
            FmError::InvalidParams("mode must be original, dark or high_contrast".into())
        })?,
    };
    if let Some(v) = s.pdf_view_mut(pane) {
        v.colors = mode;
    }
    s.prefs.pdf_colors = mode;
    Ok(())
}

fn outline_go(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["index"])?
        .ok_or_else(|| FmError::InvalidParams("index is required".into()))?;
    let pane = pdf_pane(s, params)?;
    let doc = shown(s, pane)?.doc;
    let i = index(&map, "index", doc.info().outline.len())?;
    let dest = doc
        .info()
        .outline
        .get(i)
        .and_then(|o| o.dest)
        .ok_or_else(|| FmError::InvalidParams("this bookmark has no destination".into()))?;
    go(s, pane, dest.page, dest.top)
}

fn link(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["page", "index"])?
        .ok_or_else(|| FmError::InvalidParams("page and index are required".into()))?;
    let pane = pdf_pane(s, params)?;
    let sh = shown(s, pane)?;
    let page = index(&map, "page", sh.doc.page_count())?;
    let links = sh
        .text
        .links(page)
        .ok_or_else(|| FmError::InvalidParams("the page's links are not loaded yet".into()))?;
    let i = index(&map, "index", links.len())?;
    match links.get(i).map(|l| &l.target) {
        Some(LinkTarget::Dest(d)) => go(s, pane, d.page, d.top),
        Some(LinkTarget::Uri(uri)) => {
            let lower = uri.to_ascii_lowercase();
            if !(lower.starts_with("https://")
                || lower.starts_with("http://")
                || lower.starts_with("mailto:"))
            {
                return Err(FmError::InvalidParams(
                    "Only web and e-mail links open from a PDF.".into(),
                ));
            }
            s.dispatch(
                "preview.open_link",
                &serde_json::json!({ "url": uri, "pane": pane }),
            )
        }
        Some(LinkTarget::Unsupported(what)) => Err(FmError::InvalidParams(format!(
            "This link is not followed: it asks for {what}."
        ))),
        None => Err(FmError::InvalidParams("no such link".into())),
    }
}

fn rgb(v: &Value) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let mut out = [0u8; 3];
    for (i, c) in a.iter().enumerate() {
        out[i] = u8::try_from(c.as_u64()?).ok()?;
    }
    Some(out)
}

/// `preview.pdf.visible`: the pages on screen, the scale they are drawn at, tiles for
/// pages above the pixel cap, and the theme's colours for High contrast.
fn visible(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["pages", "scale", "current", "tiles", "contrast"])?
        .ok_or_else(|| FmError::InvalidParams("pages and scale are required".into()))?;
    let pane = pdf_pane(s, params)?;
    let sh = shown(s, pane)?;
    let n = sh.doc.page_count();
    let pages: Vec<usize> = map
        .get("pages")
        .and_then(Value::as_array)
        .filter(|a| a.len() <= MAX_VISIBLE)
        .ok_or_else(|| FmError::InvalidParams("pages must be a short list".into()))?
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p < n)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| FmError::InvalidParams("a page is out of range".into()))?;
    let scale = map
        .get("scale")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 64.0)
        .ok_or_else(|| FmError::InvalidParams("scale must be between 0 and 64".into()))?
        as f32;
    let current = match map.get("current") {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|p| usize::try_from(p).ok())
                .filter(|p| *p < n)
                .ok_or_else(|| FmError::InvalidParams("current is out of range".into()))?,
        ),
    };
    let contrast = match map.get("contrast") {
        None => None,
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() == 2);
            let pair = a.and_then(|a| Some((rgb(&a[0])?, rgb(&a[1])?)));
            Some(pair.ok_or_else(|| {
                FmError::InvalidParams("contrast must be two [r, g, b] colours".into())
            })?)
        }
    };
    let step = RenderKey::step_for(scale);
    let mut tiles: Vec<(usize, Tile)> = Vec::new();
    if let Some(list) = map.get("tiles") {
        let list = list
            .as_array()
            .filter(|a| a.len() <= 64)
            .ok_or_else(|| FmError::InvalidParams("tiles must be a short list".into()))?;
        for t in list {
            let a = t
                .as_array()
                .filter(|a| a.len() == 5)
                .ok_or_else(|| FmError::InvalidParams("a tile is [page, x, y, w, h]".into()))?;
            let num = |i: usize| a.get(i).and_then(Value::as_u64);
            let (Some(p), Some(x), Some(y), Some(w), Some(h)) =
                (num(0), num(1), num(2), num(3), num(4))
            else {
                return Err(FmError::InvalidParams(
                    "a tile is [page, x, y, w, h]".into(),
                ));
            };
            let p = usize::try_from(p).ok().filter(|p| *p < n);
            let (Some(p), Some((pw, ph))) = (
                p,
                p.and_then(|p| page_pixels(&sh.doc, p, step as f32 / 4.0)),
            ) else {
                return Err(FmError::InvalidParams(
                    "a tile's page is out of range".into(),
                ));
            };
            let fits = |v: u64, lim: u32| u32::try_from(v).ok().filter(|v| *v < lim);
            let (Some(x), Some(y)) = (fits(x, pw), fits(y, ph)) else {
                return Err(FmError::InvalidParams("a tile is outside its page".into()));
            };
            let (w, h) = (
                w.clamp(1, u64::from(TILE)) as u32,
                h.clamp(1, u64::from(TILE)) as u32,
            );
            tiles.push((
                p,
                Tile {
                    x,
                    y,
                    width: w,
                    height: h,
                },
            ));
        }
    }
    // Commit the view state, then queue renders: visible first, then one either side.
    let colors = {
        let view = s.pdf_view_mut(pane).ok_or_else(no_pdf)?;
        if let Some(c) = contrast {
            view.contrast = c;
        }
        view.visible = pages.clone();
        if let Some(c) = current {
            view.page = c;
        }
        view.color_mode()
    };
    let mut order = pages.clone();
    if let (Some(first), Some(last)) = (pages.iter().min(), pages.iter().max()) {
        if *first > 0 {
            order.push(first - 1);
        }
        if last + 1 < n {
            order.push(last + 1);
        }
    }
    let mut keys_out = Vec::new();
    for p in order {
        let whole = page_pixels(&sh.doc, p, step as f32 / 4.0)
            .is_some_and(|(w, h)| u64::from(w) * u64::from(h) <= MAX_PIXELS);
        if whole {
            keys_out.push(RenderKey {
                doc: sh.id,
                page: p,
                step,
                colors,
                tile: None,
            });
        } else {
            for (tp, t) in tiles.iter().filter(|(tp, _)| *tp == p) {
                keys_out.push(RenderKey {
                    doc: sh.id,
                    page: *tp,
                    step,
                    colors,
                    tile: Some(*t),
                });
            }
        }
    }
    s.pdf.want(pane, &sh.doc, &keys_out);
    for p in &pages {
        s.pdf.want_text(sh.id, &sh.doc, &sh.text, *p);
    }
    Ok(())
}

fn password(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["password"])?
        .ok_or_else(|| FmError::InvalidParams("password is required".into()))?;
    let pane = pdf_pane(s, params)?;
    let pw = map
        .get("password")
        .and_then(Value::as_str)
        .filter(|p| p.len() <= MAX_PASSWORD * 4 && p.chars().count() <= MAX_PASSWORD)
        .ok_or_else(|| FmError::InvalidParams("password must be text".into()))?;
    let doc = s.pdf_doc(pane).cloned().ok_or_else(no_pdf)?;
    if doc.document().is_some() {
        return Err(FmError::InvalidParams("this PDF is not locked".into()));
    }
    let secret = Secret::new(pw);
    let view = s.pdf_view_mut(pane).ok_or_else(no_pdf)?;
    if view.unlocking {
        return Ok(());
    }
    view.unlocking = true;
    view.password_error = None;
    view.password = Some(secret.clone());
    let source_id = doc.id;
    let inbox = s.pdf_inbox.clone();
    s.pdf.run(move || {
        let result = doc.unlock(&secret);
        push(
            &inbox,
            PdfMsg::Unlocked {
                pane,
                source_id,
                result,
            },
        );
    });
    Ok(())
}

fn toggle(s: &mut Session, params: &Params, which: &str) -> Result<(), FmError> {
    keys(params, &[])?;
    let pane = pdf_pane(s, params)?;
    let sh = shown(s, pane)?;
    let view = s.pdf_view_mut(pane).ok_or_else(no_pdf)?;
    match which {
        "outline" => {
            if sh.doc.info().outline.is_empty() {
                return Err(FmError::InvalidParams("this PDF has no bookmarks".into()));
            }
            view.outline_open = !view.outline_open;
        }
        _ => {
            if sh.doc.info().forms.signature_fields == 0 {
                return Err(FmError::InvalidParams("this PDF has no signatures".into()));
            }
            view.signatures_open = !view.signatures_open;
            s.pdf.want_verify(sh.id, &sh.doc);
        }
    }
    Ok(())
}

fn position(v: Option<&Value>, n: usize) -> Option<(usize, usize)> {
    let m = v?.as_object()?;
    let page = usize::try_from(m.get("page")?.as_u64()?)
        .ok()
        .filter(|p| *p < n)?;
    let offset = usize::try_from(m.get("offset")?.as_u64()?).ok()?;
    Some((page, offset))
}

fn select(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = keys(params, &["start", "end"])?;
    let pane = pdf_pane(s, params)?;
    let n = shown(s, pane)?.doc.page_count();
    let selection = match map {
        None => None,
        Some(m) if m.get("start").is_none() && m.get("end").is_none() => None,
        Some(m) => {
            let a = position(m.get("start"), n)
                .ok_or_else(|| FmError::InvalidParams("start is {page, offset}".into()))?;
            let b = position(m.get("end"), n)
                .ok_or_else(|| FmError::InvalidParams("end is {page, offset}".into()))?;
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            Some(PdfSelection { start, end })
        }
    };
    let view = s.pdf_view_mut(pane).ok_or_else(no_pdf)?;
    view.selection = selection;
    Ok(())
}

/// `preview.text.select_all` on a PDF: the whole document, text extracted off-thread.
pub(crate) fn select_all(s: &mut Session, pane: PaneId) -> Result<(), FmError> {
    let sh = shown(s, pane)?;
    let n = sh.doc.page_count();
    for p in 0..n {
        s.pdf.want_text(sh.id, &sh.doc, &sh.text, p);
    }
    let view = s.pdf_view_mut(pane).ok_or_else(no_pdf)?;
    view.selection = Some(PdfSelection {
        start: (0, 0),
        end: (n.saturating_sub(1), usize::MAX),
    });
    Ok(())
}

/// `preview.text.copy` on a PDF: now when the text is there, else as soon as it is.
pub(crate) fn copy(s: &mut Session, pane: PaneId) -> Result<(), FmError> {
    let sh = shown(s, pane)?;
    let selection = s
        .pdf_view(pane)
        .and_then(|v| v.selection)
        .ok_or(FmError::Unsupported("no PDF text selection"))?;
    let pages = selection.start.0..=selection.end.0;
    let ready = pages.clone().all(|p| s.pdf.text_done(sh.id, &sh.text, p));
    if ready {
        s.clipboard_out = Some(selected_text(&sh, Some(selection)));
        return Ok(());
    }
    for p in pages {
        s.pdf.want_text(sh.id, &sh.doc, &sh.text, p);
    }
    if let Some(v) = s.pdf_view_mut(pane) {
        v.copy_pending = true;
    }
    Ok(())
}

/// The selected text, pages joined by a blank line, capped at [`MAX_COPY_BYTES`].
pub(crate) fn selected_text(sh: &Shown, selection: Option<PdfSelection>) -> String {
    let Some(sel) = selection else {
        return String::new();
    };
    let mut out = String::new();
    for p in sel.start.0..=sel.end.0 {
        let Some(t) = sh.text.get(p) else {
            continue;
        };
        let from = if p == sel.start.0 { sel.start.1 } else { 0 };
        let to = if p == sel.end.0 {
            sel.end.1
        } else {
            usize::MAX
        };
        let to = to.min(t.text.len());
        let from = from.min(to);
        let (from, to) = (floor_char(&t.text, from), floor_char(&t.text, to));
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(t.text.get(from..to).unwrap_or(""));
        if out.len() > MAX_COPY_BYTES {
            let cut = floor_char(&out, MAX_COPY_BYTES);
            out.truncate(cut);
            break;
        }
    }
    out
}

fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Body of `Preview::Pdf` replaced by the unlocked document (used by `poll`).
pub(crate) fn replace_body(s: &mut Session, pane: PaneId, doc: fm_preview::pdf_prev::PdfDoc) {
    if let Some(slot) = s.previews.slots.get_mut(&pane) {
        let id = doc.id;
        slot.body = Body::Static(Arc::new(Preview::Pdf(doc)));
        if let Some(v) = slot.pdf.as_mut() {
            v.source_id = id;
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "preview.pdf.page",
            label: "Go to Page",
            menu: &["Preview", "PDF", "Go to Page"],
            shortcut: None,
            params: "{page: number (1-based) | string (page number or label), pane?: number}",
            enabled: pdf_focused,
            run: page,
        },
        CommandSpec {
            id: "preview.pdf.next",
            label: "Next Page",
            menu: &["Preview", "PDF", "Next Page"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: |s, p| step(s, p, 1),
        },
        CommandSpec {
            id: "preview.pdf.prev",
            label: "Previous Page",
            menu: &["Preview", "PDF", "Previous Page"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: |s, p| step(s, p, -1),
        },
        CommandSpec {
            id: "preview.pdf.first",
            label: "First Page",
            menu: &["Preview", "PDF", "First Page"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: |s, p| step(s, p, isize::MIN),
        },
        CommandSpec {
            id: "preview.pdf.last",
            label: "Last Page",
            menu: &["Preview", "PDF", "Last Page"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: |s, p| step(s, p, isize::MAX),
        },
        CommandSpec {
            id: "preview.pdf.fit",
            label: "Page Fit",
            menu: &["Preview", "PDF", "Page Fit"],
            shortcut: None,
            params: "{mode?: \"width\"|\"page\"|\"actual\", pane?: number} (omitted: the next mode)",
            enabled: pdf_focused,
            run: fit,
        },
        CommandSpec {
            id: "preview.pdf.colors",
            label: "Page Colours",
            menu: &["Preview", "PDF", "Page Colours"],
            shortcut: None,
            params: "{mode?: \"original\"|\"dark\"|\"high_contrast\", pane?: number} (omitted: the next mode)",
            enabled: pdf_focused,
            run: colors,
        },
        CommandSpec {
            id: "preview.pdf.outline.toggle",
            label: "Show Bookmarks",
            menu: &["Preview", "PDF", "Bookmarks"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: |s, p| toggle(s, p, "outline"),
        },
        CommandSpec {
            id: "preview.pdf.outline.go",
            label: "Go to Bookmark",
            menu: &["Preview", "PDF", "Go to Bookmark"],
            shortcut: None,
            params: "{index: number (outline entry), pane?: number}",
            enabled: pdf_focused,
            run: outline_go,
        },
        CommandSpec {
            id: "preview.pdf.link",
            label: "Follow PDF Link",
            menu: &["Preview", "PDF", "Follow Link"],
            shortcut: None,
            params: "{page: number (0-based), index: number (link on that page), pane?: number}",
            enabled: pdf_focused,
            run: link,
        },
        CommandSpec {
            id: "preview.pdf.visible",
            label: "Report Visible PDF Pages",
            menu: &["Preview", "PDF", "Visible Pages"],
            shortcut: None,
            params: "{pages: [number ≤ 32], scale: number (pixels per point), current?: number, tiles?: [[page, x, y, w, h]], contrast?: [[r,g,b],[r,g,b]], pane?: number}",
            enabled: pdf_focused,
            run: visible,
        },
        CommandSpec {
            id: "preview.pdf.password",
            label: "Unlock PDF",
            menu: &["Preview", "PDF", "Unlock"],
            shortcut: None,
            params: "{password: string, pane?: number} (held in memory for this document only)",
            enabled: |s| {
                let pane = s.layout.focused();
                s.pdf_doc(pane).is_some_and(|d| d.document().is_none())
            },
            run: password,
        },
        CommandSpec {
            id: "preview.pdf.signatures.toggle",
            label: "Show Signatures",
            menu: &["Preview", "PDF", "Signatures"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: |s, p| toggle(s, p, "signatures"),
        },
        CommandSpec {
            id: "preview.pdf.select",
            label: "Select PDF Text",
            menu: &["Preview", "PDF", "Select Text"],
            shortcut: None,
            params: "{start: {page, offset}, end: {page, offset}, pane?: number} (none clears)",
            enabled: pdf_focused,
            run: select,
        },
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// A session whose focused pane is a Preview showing `bytes` as a PDF.
    pub(crate) fn pdf_session(bytes: Vec<u8>) -> (Session, PaneId) {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = fm_layout::PaneKind::Preview;
        let mut slot = crate::preview_cmds::PreviewSlot::new();
        slot.body = Body::Static(Arc::new(fm_preview::pdf_prev::open_bytes(
            Vec::new(),
            bytes.into(),
        )));
        s.previews.slots.insert(pane, slot);
        s.sync_pdf_views();
        (s, pane)
    }

    #[test]
    fn navigation_and_bad_params() {
        let (mut s, pane) = pdf_session(fm_pdf::fixtures::text_pages(5));
        let p = |v: Value| {
            let mut v = v;
            v["pane"] = json!(pane);
            v
        };
        s.dispatch("preview.pdf.page", &p(json!({"page": 3})))
            .expect("page 3");
        assert_eq!(s.pdf_view(pane).map(|v| v.page), Some(2));
        s.dispatch("preview.pdf.page", &p(json!({"page": "ii"})))
            .expect("label");
        assert_eq!(s.pdf_view(pane).map(|v| v.page), Some(1));
        s.dispatch("preview.pdf.next", &p(json!({}))).expect("next");
        s.dispatch("preview.pdf.last", &p(json!({}))).expect("last");
        assert_eq!(s.pdf_view(pane).map(|v| v.page), Some(4));
        s.dispatch("preview.pdf.next", &p(json!({})))
            .expect("clamped");
        assert_eq!(s.pdf_view(pane).map(|v| v.page), Some(4));
        for bad in [
            json!({"page": -1}),
            json!({"page": 0}),
            json!({"page": 6}),
            json!({"page": "x".repeat(MAX_LABEL + 1)}),
            json!({"page": "nope"}),
            json!({"page": true}),
            json!({"bogus": 1}),
        ] {
            assert!(
                s.dispatch("preview.pdf.page", &p(bad.clone())).is_err(),
                "{bad}"
            );
            assert_eq!(
                s.pdf_view(pane).map(|v| v.page),
                Some(4),
                "unchanged after {bad}"
            );
        }
        s.dispatch("preview.pdf.outline.go", &p(json!({"index": 2})))
            .expect("bookmark");
        assert_eq!(s.pdf_view(pane).map(|v| v.page), Some(4));
        assert!(
            s.dispatch("preview.pdf.outline.go", &p(json!({"index": 9})))
                .is_err()
        );
    }

    #[test]
    fn fit_and_colours_are_remembered() {
        let (mut s, pane) = pdf_session(fm_pdf::fixtures::text_pages(1));
        if let Some(slot) = s.previews.slots.get_mut(&pane) {
            slot.scale = 2.0;
        }
        s.dispatch("preview.pdf.fit", &json!({"mode": "page", "pane": pane}))
            .expect("fit");
        assert_eq!(s.prefs.pdf_fit, PdfFit::Page);
        assert_eq!(
            s.preview_slot(pane).map(|sl| sl.scale),
            Some(1.0),
            "zoom reset"
        );
        s.dispatch("preview.pdf.colors", &json!({"mode": "dark", "pane": pane}))
            .expect("colors");
        assert_eq!(s.prefs.pdf_colors, PdfColors::Dark);
        assert!(
            s.dispatch("preview.pdf.fit", &json!({"mode": "huge", "pane": pane}))
                .is_err()
        );
        assert!(
            s.dispatch("preview.pdf.colors", &json!({"mode": 3, "pane": pane}))
                .is_err()
        );
    }

    #[test]
    fn visible_validates_and_renders() {
        let (mut s, pane) = pdf_session(fm_pdf::fixtures::text_pages(3));
        for bad in [
            json!({"pages": [9], "scale": 1.0}),
            json!({"pages": [0], "scale": 0}),
            json!({"pages": [0], "scale": 1e9}),
            json!({"pages": (0..40).collect::<Vec<_>>(), "scale": 1.0}),
            json!({"pages": [0], "scale": 1.0, "tiles": [[0, 99999, 0, 10, 10]]}),
            json!({"pages": [0], "scale": 1.0, "contrast": [[1, 2], [3, 4, 5]]}),
        ] {
            let mut b = bad.clone();
            b["pane"] = json!(pane);
            assert!(s.dispatch("preview.pdf.visible", &b).is_err(), "{bad}");
        }
        s.dispatch(
            "preview.pdf.visible",
            &json!({"pages": [0, 1], "scale": 0.5, "current": 1, "pane": pane}),
        )
        .expect("visible");
        assert_eq!(s.pdf_view(pane).map(|v| v.page), Some(1));
        let shown = s.pdf_shown(pane).expect("shown");
        let key = RenderKey {
            doc: shown.id,
            page: 0,
            step: 2,
            colors: fm_pdf::ColorMode::Original,
            tile: None,
        };
        let start = std::time::Instant::now();
        while !matches!(
            s.pdf.get(&key),
            Some(crate::pdf_service::RenderState::Ready(_))
        ) {
            assert!(start.elapsed().as_secs() < 20);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn password_unlocks_and_wrong_ones_are_reported() {
        let (mut s, pane) = pdf_session(fm_pdf::fixtures::encrypted("pw"));
        assert!(s.pdf_shown(pane).is_none(), "locked");
        assert!(s.command_enabled("preview.pdf.password") || s.layout.focused() != pane);
        let wait = |s: &mut Session| {
            let start = std::time::Instant::now();
            while s.pdf_view(pane).is_some_and(|v| v.unlocking) {
                assert!(start.elapsed().as_secs() < 20);
                s.pump_pdf();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        s.dispatch(
            "preview.pdf.password",
            &json!({"password": "nope", "pane": pane}),
        )
        .expect("try");
        wait(&mut s);
        let err = s
            .pdf_view(pane)
            .and_then(|v| v.password_error.clone())
            .expect("error");
        assert!(!err.contains("nope"));
        s.dispatch(
            "preview.pdf.password",
            &json!({"password": "pw", "pane": pane}),
        )
        .expect("try again");
        wait(&mut s);
        assert!(s.pdf_shown(pane).is_some(), "unlocked");
        assert!(
            s.dispatch(
                "preview.pdf.password",
                &json!({"password": "pw", "pane": pane})
            )
            .is_err(),
            "not locked any more"
        );
        assert!(s.pdf_view(pane).is_some_and(|v| v.password.is_some()));
        // Another file in the pane: the password is gone with the old view.
        if let Some(slot) = s.previews.slots.get_mut(&pane) {
            slot.body = Body::Static(Arc::new(fm_preview::pdf_prev::open_bytes(
                Vec::new(),
                fm_pdf::fixtures::encrypted("pw").into(),
            )));
        }
        s.sync_pdf_views();
        assert!(s.pdf_view(pane).is_some_and(|v| v.password.is_none()));
        assert!(s.pdf_shown(pane).is_none(), "the new file asks again");
    }

    #[test]
    fn selection_and_copy() {
        let (mut s, pane) = pdf_session(fm_pdf::fixtures::text_pages(2));
        select_all(&mut s, pane).expect("select all");
        copy(&mut s, pane).expect("copy");
        let start = std::time::Instant::now();
        while s.clipboard_out.is_none() {
            assert!(start.elapsed().as_secs() < 20);
            s.pump_pdf();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let text = s.clipboard_out.take().unwrap_or_default();
        assert!(text.contains("Page 1") && text.contains("Page 2"), "{text}");
        assert!(
            s.dispatch(
                "preview.pdf.select",
                &json!({"start": {"page": 9, "offset": 0}, "end": {"page": 0, "offset": 1}, "pane": pane})
            )
            .is_err()
        );
        s.dispatch(
            "preview.pdf.select",
            &json!({"start": {"page": 0, "offset": 5}, "end": {"page": 0, "offset": 0}, "pane": pane}),
        )
        .expect("select");
        assert_eq!(
            s.pdf_view(pane).and_then(|v| v.selection),
            Some(PdfSelection {
                start: (0, 0),
                end: (0, 5)
            })
        );
    }
}
