//! PDF pages in Preview (change `add-pdf-preview`): a virtualized column of pages drawn
//! from the engine's render cache, with links, Find highlights, text selection, the
//! bookmarks and signatures panels and the Fill & Sign overlays. Everything the user does
//! becomes a `preview.pdf.*` / `pdf.*` command; nothing here reads files or renders.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Id, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui, Vec2, pos2, vec2};
use fm_engine::Session;
use fm_engine::pdf_form_cmds::{FillState, PdfSession, SaveState, SaveTarget};
use fm_engine::pdf_service::{RenderKey, RenderState, TILE, VerifyState};
use fm_engine::pdf_view::{PdfColors, PdfFit, PdfView, Shown};
use fm_layout::PaneId;
use fm_pdf::forms::{Field, FieldKind, FieldValue};
use fm_pdf::render::{Pixels, Tile, page_pixels};
use fm_pdf::verify::SigStatus;
use fm_pdf::{LinkTarget, MAX_PIXELS};
use serde_json::{Value, json};

use super::Action;
use crate::theme::Tokens;

/// Space between pages and around the column, in points.
const GAP: f32 = 12.0;
/// Width of the bookmarks and signatures panels.
const PANEL_W: f32 = 240.0;
/// "100%": one PDF point (1/72 in) at 96 dpi.
const ACTUAL: f32 = 96.0 / 72.0;
/// Default width of a placed visual signature, in page points.
const PLACE_W: f32 = 150.0;

fn id(pane: PaneId, what: &str) -> Id {
    Id::NULL.with(("pv-pdf", pane, what))
}

fn temp<T: Clone + Default + Send + Sync + 'static>(ui: &Ui, key: Id) -> T {
    ui.data(|d| d.get_temp::<T>(key)).unwrap_or_default()
}

fn set_temp<T: Clone + Send + Sync + 'static>(ui: &Ui, key: Id, v: T) {
    ui.data_mut(|d| d.insert_temp(key, v));
}

/// Text field state in the toolbar and dialogs.
#[derive(Clone, Default)]
struct PageField {
    text: String,
    editing: bool,
}

/// A form field being edited in place.
#[derive(Clone)]
struct FieldEdit {
    field: String,
    text: String,
    multiline: bool,
    focused: bool,
}

/// A selection drag in progress: anchor and current position.
#[derive(Clone, Copy, Default)]
struct Drag {
    anchor: Option<(usize, usize)>,
    head: Option<(usize, usize)>,
}

/// A placed signature being dragged: id, page and current rectangle.
#[derive(Clone, Copy)]
struct Moving {
    id: u64,
    page: usize,
    rect: [f32; 4],
}

/// What the Fill & Sign bar is asking about.
#[derive(Clone, Default, PartialEq)]
enum Confirm {
    #[default]
    None,
    Overwrite,
    SaveAs(String),
}

/// The toolbar's PDF controls (folded behind the view-options button on narrow panes).
pub(super) fn toolbar_options(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(view) = s.pdf_view(pane) else {
        return;
    };
    let Some(shown) = s.pdf_shown(pane) else {
        return;
    };
    let info = shown.doc.info();
    let n = shown.doc.page_count();
    let page = view.page.min(n.saturating_sub(1));
    if ui
        .add_enabled(page > 0, egui::Button::new("‹"))
        .on_hover_text("Previous page")
        .clicked()
    {
        actions.push(("preview.pdf.prev", json!({})));
    }
    let key = id(pane, "page-field");
    let mut field: PageField = temp(ui, key);
    let label = info
        .pages
        .get(page)
        .and_then(|p| p.label.clone())
        .unwrap_or_else(|| (page + 1).to_string());
    if !field.editing {
        field.text = label;
    }
    let resp = ui.add(egui::TextEdit::singleline(&mut field.text).desired_width(44.0));
    let resp = resp.on_hover_text("Page number or label; Enter goes there");
    if resp.gained_focus() {
        field.editing = true;
    }
    if resp.lost_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !field.text.trim().is_empty() {
            let text = field.text.trim().to_string();
            let target = text
                .parse::<u64>()
                .ok()
                .filter(|_| info.find_page(&text).is_none())
                .map_or(json!(text), |n| json!(n));
            actions.push(("preview.pdf.page", json!({ "page": target })));
        }
        field.editing = false;
    }
    set_temp(ui, key, field);
    let total = if info.pages_truncated {
        format!("of {n}+")
    } else {
        format!("of {n}")
    };
    ui.label(RichText::new(total).color(tokens.text_muted));
    if ui
        .add_enabled(page + 1 < n, egui::Button::new("›"))
        .on_hover_text("Next page")
        .clicked()
    {
        actions.push(("preview.pdf.next", json!({})));
    }
    egui::ComboBox::from_id_salt(id(pane, "fit"))
        .selected_text(view.fit.label())
        .width(84.0)
        .show_ui(ui, |ui| {
            for f in PdfFit::ALL {
                if ui.selectable_label(view.fit == f, f.label()).clicked() {
                    actions.push(("preview.pdf.fit", json!({ "mode": f.id() })));
                }
            }
        })
        .response
        .on_hover_text("How pages are sized; the pane zoom multiplies it");
    egui::ComboBox::from_id_salt(id(pane, "colors"))
        .selected_text(view.colors.label())
        .width(104.0)
        .show_ui(ui, |ui| {
            for c in PdfColors::ALL {
                if ui.selectable_label(view.colors == c, c.label()).clicked() {
                    actions.push(("preview.pdf.colors", json!({ "mode": c.id() })));
                }
            }
        })
        .response
        .on_hover_text("Page colours for easier reading; saved files are not changed");
    if !info.outline.is_empty()
        && ui
            .add(egui::Button::selectable(view.outline_open, "Bookmarks"))
            .clicked()
    {
        actions.push(("preview.pdf.outline.toggle", json!({})));
    }
    if info.forms.signature_fields > 0 {
        let source_id = s.pdf_doc(pane).map_or(shown.id, |d| d.id);
        let (text, color) = badge(s, source_id, info.forms.signed, tokens);
        if ui
            .add(egui::Button::selectable(
                view.signatures_open,
                RichText::new(text).color(color),
            ))
            .on_hover_text("Show the document's signatures and whether they are intact")
            .clicked()
        {
            actions.push(("preview.pdf.signatures.toggle", json!({})));
        }
    }
    // Fill & Sign is on the pages (forms) or in the always-visible part of the bar.
}

/// The signature state shown as a square on the pages: broken, changed after signing,
/// or all intact. Nothing while the check is running or when it timed out.
fn sig_notice(s: &Session, doc: u64, signed: usize, tokens: &Tokens) -> Option<SigNotice> {
    if signed == 0 {
        return None;
    }
    let Some(VerifyState::Done(reports)) = s.pdf.verify_state(doc) else {
        return None;
    };
    if reports
        .iter()
        .any(|r| matches!(r.status, SigStatus::Broken(_)))
    {
        Some(SigNotice {
            word: "broken",
            tip: "A signature in this PDF is broken: it does not match the document, or its signer cannot be checked. Click to show the signatures.",
            fill: tokens.sig_broken,
            ok: false,
        })
    } else if reports
        .iter()
        .any(|r| matches!(r.status, SigStatus::ChangedAfter(_)))
    {
        Some(SigNotice {
            word: "changed",
            tip: "This PDF was changed after it was signed. Click to show the signatures and what they cover.",
            fill: tokens.sig_changed,
            ok: false,
        })
    } else if !reports.is_empty() && reports.iter().all(|r| r.status == SigStatus::Valid) {
        Some(SigNotice {
            word: "intact",
            tip: "The signed document is unchanged since it was signed. Who the signer is was not checked against trusted authorities. Click to show the signatures.",
            fill: tokens.sig_valid,
            ok: true,
        })
    } else {
        None
    }
}

struct SigNotice {
    /// Second line under "Signature".
    word: &'static str,
    tip: &'static str,
    fill: Color32,
    /// Intact (a check mark) rather than a problem (a warning triangle).
    ok: bool,
}

fn badge(s: &Session, doc: u64, signed: usize, tokens: &Tokens) -> (String, Color32) {
    if signed == 0 {
        return ("Signatures".into(), tokens.text);
    }
    match s.pdf.verify_state(doc) {
        Some(VerifyState::Done(reports)) => {
            if reports
                .iter()
                .any(|r| matches!(r.status, SigStatus::Broken(_)))
            {
                ("Signature problem".into(), tokens.sig_broken)
            } else if reports
                .iter()
                .any(|r| matches!(r.status, SigStatus::ChangedAfter(_)))
            {
                ("Changed after signing".into(), tokens.sig_changed)
            } else {
                ("Signed".into(), tokens.sig_valid)
            }
        }
        Some(VerifyState::TimedOut) => ("Signatures not checked".into(), tokens.sig_changed),
        _ => ("Checking signatures…".into(), tokens.text_muted),
    }
}

/// The PDF body: password card, or panels and pages.
pub(super) fn body(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(view) = s.pdf_view(pane) else {
        ui.centered_and_justified(|ui| ui.add(crate::spinner::Spinner::new()));
        return;
    };
    let Some(shown) = s.pdf_shown(pane) else {
        password_card(ui, pane, tokens, view, actions);
        return;
    };
    keys(ui, s, pane, view, actions);
    if let Some(fill) = &view.fill
        && needs_strip(ui, pane, fill)
    {
        fill_strip(ui, pane, tokens, fill, actions);
        ui.separator();
    }
    let full = ui.available_rect_before_wrap();
    let mut pages_rect = full;
    if view.outline_open && !shown.doc.info().outline.is_empty() && full.width() > PANEL_W * 2.0 {
        let left = Rect::from_min_size(full.min, vec2(PANEL_W, full.height()));
        pages_rect.min.x = left.max.x + 4.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            outline(ui, pane, tokens, &shown, actions);
        });
        ui.painter().vline(
            left.max.x + 2.0,
            full.y_range(),
            Stroke::new(tokens.border_width, tokens.border),
        );
    }
    if view.signatures_open && pages_rect.width() > PANEL_W * 2.0 {
        let right = Rect::from_min_max(pos2(pages_rect.max.x - PANEL_W, full.min.y), full.max);
        pages_rect.max.x = right.min.x - 4.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            signatures(ui, s, pane, tokens, &shown, actions);
        });
        ui.painter().vline(
            right.min.x - 2.0,
            full.y_range(),
            Stroke::new(tokens.border_width, tokens.border),
        );
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(pages_rect), |ui| {
        pages(ui, s, pane, tokens, view, &shown, actions);
    });
    ui.allocate_rect(full, Sense::hover());
}

/// Pane-local keys: Home/End jump to the first/last page, Ctrl+C copies a PDF selection,
/// Escape clears it.
fn keys(ui: &mut Ui, s: &Session, pane: PaneId, view: &PdfView, actions: &mut Vec<Action>) {
    let can = s.layout.focused() == pane
        && !crate::dialogs::window_owns_keyboard(s)
        && !ui.ctx().text_edit_focused()
        && ui.is_enabled()
        && ui.ctx().memory(|m| m.is_above_modal_layer(ui.layer_id()))
        && !ui.any_popup_open();
    if !can {
        return;
    }
    ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
            actions.push(("preview.pdf.first", json!({})));
        }
        if i.consume_key(egui::Modifiers::NONE, egui::Key::End) {
            actions.push(("preview.pdf.last", json!({})));
        }
        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::PageDown) {
            actions.push(("preview.pdf.next", json!({})));
        }
        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::PageUp) {
            actions.push(("preview.pdf.prev", json!({})));
        }
    });
    if view.selection.is_some() {
        let copy = ui.input_mut(|i| {
            let mut copy = false;
            i.events.retain(|e| {
                if matches!(e, egui::Event::Copy) {
                    copy = true;
                    false
                } else {
                    true
                }
            });
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::C) || copy
        });
        if copy {
            actions.push(("preview.text.copy", Value::Null));
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            actions.push(("preview.pdf.select", json!({})));
        }
    }
}

fn password_card(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    view: &PdfView,
    actions: &mut Vec<Action>,
) {
    let key = id(pane, "password");
    let mut text: String = temp(ui, key);
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.label(
            RichText::new("This PDF is protected with a password.")
                .color(tokens.text)
                .strong(),
        );
        ui.label(
            RichText::new("The password is kept in memory only while this document is shown.")
                .color(tokens.text_muted),
        );
        ui.add_space(8.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut text)
                .password(true)
                .hint_text("Password")
                .desired_width(220.0),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(4.0);
        let submit = ui
            .add_enabled(
                !view.unlocking && !text.is_empty(),
                egui::Button::new("Open"),
            )
            .clicked()
            || (enter && !text.is_empty() && !view.unlocking);
        if view.unlocking {
            ui.add(crate::spinner::Spinner::new());
        }
        if let Some(err) = &view.password_error {
            ui.label(RichText::new(err).color(tokens.error));
        }
        if submit {
            actions.push(("preview.pdf.password", json!({ "password": text })));
            text.clear();
        }
    });
    ui.allocate_space(ui.available_size());
    // An entered password leaves egui memory as soon as it is sent.
    set_temp(ui, key, text);
}

fn outline(ui: &mut Ui, pane: PaneId, tokens: &Tokens, shown: &Shown, actions: &mut Vec<Action>) {
    let items = &shown.doc.info().outline;
    ui.label(RichText::new("Bookmarks").color(tokens.text_muted).strong());
    let row = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    ScrollArea::vertical()
        .id_salt(id(pane, "outline"))
        .auto_shrink([false, false])
        .show_rows(ui, row, items.len(), |ui, range| {
            for i in range {
                let Some(item) = items.get(i) else { continue };
                ui.horizontal(|ui| {
                    ui.add_space(item.depth.min(12) as f32 * 12.0);
                    let enabled = item.dest.is_some();
                    let title = if item.title.trim().is_empty() {
                        "(untitled)"
                    } else {
                        item.title.as_str()
                    };
                    let resp = ui.add_enabled(
                        enabled,
                        egui::Button::new(RichText::new(title).color(tokens.text)).frame(false),
                    );
                    let resp =
                        resp.on_disabled_hover_text("This bookmark does not point to a page");
                    if resp.clicked() {
                        actions.push(("preview.pdf.outline.go", json!({ "index": i })));
                    }
                });
            }
        });
}

fn signatures(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    shown: &Shown,
    actions: &mut Vec<Action>,
) {
    let source_id = s.pdf_doc(pane).map_or(shown.id, |d| d.id);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Signatures")
                .color(tokens.text_muted)
                .strong(),
        );
        if ui.small_button("×").on_hover_text("Close").clicked() {
            actions.push(("preview.pdf.signatures.toggle", json!({})));
        }
    });
    ScrollArea::vertical()
        .id_salt(id(pane, "sigs"))
        .auto_shrink([false, false])
        .show(ui, |ui| match s.pdf.verify_state(source_id) {
            None if shown.doc.info().forms.signed == 0 => {
                ui.label(
                    RichText::new("The document has empty signature fields and no signatures yet.")
                        .color(tokens.text_muted),
                );
            }
            None | Some(VerifyState::Running) => {
                ui.horizontal(|ui| {
                    ui.add(crate::spinner::Spinner::new());
                    ui.label("Checking signatures…");
                });
            }
            Some(VerifyState::TimedOut) => {
                ui.label(
                    RichText::new("Checking the signatures took too long and was stopped.")
                        .color(tokens.sig_changed),
                );
            }
            Some(VerifyState::Done(reports)) => {
                if reports.is_empty() {
                    ui.label(RichText::new("No signatures.").color(tokens.text_muted));
                }
                for r in reports.iter() {
                    let color = match r.status {
                        SigStatus::Valid => tokens.sig_valid,
                        SigStatus::ChangedAfter(_) => tokens.sig_changed,
                        SigStatus::Broken(_) => tokens.sig_broken,
                    };
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(r.status.label()).color(color).strong());
                        if let SigStatus::ChangedAfter(what) = &r.status {
                            for w in what {
                                ui.label(RichText::new(format!("• {w}")).color(tokens.text_muted));
                            }
                        }
                        if let SigStatus::Broken(why) = &r.status {
                            ui.label(RichText::new(sentence(why)).color(tokens.text));
                        }
                        let row = |ui: &mut Ui, k: &str, v: &Option<String>| {
                            if let Some(v) = v {
                                ui.label(RichText::new(format!("{k}: {v}")).color(tokens.text));
                            }
                        };
                        row(ui, "Signed by", &r.signer.as_ref().map(|c| c.name.clone()));
                        if let Some(c) = &r.signer {
                            if c.self_signed {
                                ui.label(
                                    RichText::new(
                                        "Self-signed certificate: the signer vouches for themselves.",
                                    )
                                    .color(tokens.sig_changed),
                                );
                            }
                            ui.collapsing("Certificate", |ui| {
                                let valid = c.validity_text();
                                for (k, v) in [
                                    ("Subject", &c.subject),
                                    ("Issuer", &c.issuer),
                                    ("Valid", &valid),
                                    ("Serial", &c.serial),
                                    ("SHA-256 fingerprint", &c.fingerprint),
                                ] {
                                    ui.label(RichText::new(format!("{k}: {v}")).color(tokens.text_muted));
                                }
                            });
                        }
                        row(ui, "Time (from the signer's computer)", &r.time);
                        row(ui, "Reason", &r.reason);
                        row(ui, "Location", &r.location);
                        row(ui, "Contact", &r.contact);
                        ui.label(RichText::new(format!("Field: {}", r.field)).color(tokens.text_muted));
                        if !r.weak.is_empty() {
                            ui.label(
                                RichText::new(format!("Weak cryptography: {}.", r.weak.join(", ")))
                                    .color(tokens.sig_changed),
                            );
                        }
                        if let Some(p) = &r.cert_time_problem {
                            ui.label(RichText::new(p).color(tokens.sig_changed));
                        }
                    });
                }
                ui.add_space(6.0);
                crate::dialog_style::hint(
                    ui,
                    tokens,
                    "Intact means the signed bytes are unchanged. Who the signer is was not checked against trusted authorities.",
                );
            }
        });
}

/// `the file was rebuilt…` → `The file was rebuilt….`
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// Fit scale for the pane, before the pane's zoom.
fn fit_scale(fit: PdfFit, avail: Vec2, sizes: &[(f32, f32)]) -> f32 {
    let max_w = sizes.iter().map(|s| s.0).fold(1.0f32, f32::max);
    let max_h = sizes.iter().map(|s| s.1).fold(1.0f32, f32::max);
    let w = (avail.x - 2.0 * GAP).max(40.0);
    let h = (avail.y - 2.0 * GAP).max(40.0);
    match fit {
        PdfFit::Width => w / max_w,
        PdfFit::Page => (w / max_w).min(h / max_h),
        PdfFit::Actual => ACTUAL,
    }
    .clamp(0.05, 16.0)
}

type TexCache = HashMap<usize, (Arc<Pixels>, egui::TextureHandle)>;

/// The texture of `px`, uploaded once while it stays on screen.
fn texture(
    ui: &Ui,
    pane: PaneId,
    used: &mut TexCache,
    old: &mut TexCache,
    px: &Arc<Pixels>,
) -> egui::TextureId {
    let key = Arc::as_ptr(px) as usize;
    if let Some(e) = used.get(&key) {
        return e.1.id();
    }
    let entry = old.remove(&key).unwrap_or_else(|| {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [px.width as usize, px.height as usize],
            &px.rgba,
        );
        let tex = ui.ctx().load_texture(
            format!("pv-pdf-{pane}-{key}"),
            image,
            egui::TextureOptions::LINEAR,
        );
        (px.clone(), tex)
    });
    let tex = entry.1.id();
    used.insert(key, entry);
    tex
}

fn to_screen(page_rect: Rect, scale: f32, r: [f32; 4]) -> Rect {
    Rect::from_min_max(
        page_rect.min + vec2(r[0], r[1]) * scale,
        page_rect.min + vec2(r[2], r[3]) * scale,
    )
}

fn to_page(page_rect: Rect, scale: f32, p: Pos2) -> (f32, f32) {
    let v = (p - page_rect.min) / scale;
    (v.x, v.y)
}

#[allow(clippy::too_many_lines)]
fn pages(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    view: &PdfView,
    shown: &Shown,
    actions: &mut Vec<Action>,
) {
    let info = shown.doc.info();
    let area_rect = ui.max_rect();
    let sizes: Vec<(f32, f32)> = info.pages.iter().map(|p| (p.width, p.height)).collect();
    let n = sizes.len();
    if n == 0 {
        ui.label("This PDF has no pages.");
        return;
    }
    let zoom = s.preview_slot(pane).map_or(1.0, |sl| sl.scale);
    let scale = fit_scale(view.fit, ui.available_size(), &sizes) * zoom;
    let ppp = ui.ctx().pixels_per_point();
    let step = RenderKey::step_for(scale * ppp);
    let colors = view.color_mode();
    let too_complex = s.pdf.too_complex(shown.id);
    // Page tops in content coordinates.
    let mut tops = Vec::with_capacity(n + 1);
    let mut y = GAP;
    let mut content_w: f32 = 0.0;
    for (w, h) in &sizes {
        tops.push(y);
        y += h * scale + GAP;
        content_w = content_w.max(w * scale + 2.0 * GAP);
    }
    tops.push(y);
    let content_h = y;
    // Navigation requests (page field, bookmarks, links, Find) scroll once each.
    let goto_key = id(pane, "goto-seq");
    let mut area = super::paged_scroll(
        ui,
        ScrollArea::both().auto_shrink([false, false]),
        ("pv-pdf", pane),
    );
    // Top of page `p` in content coordinates (`tops` has one more entry than pages).
    let top_of = |p: usize| tops.get(p).copied().unwrap_or(content_h);
    if let Some(goto) = view.goto
        && temp::<u64>(ui, goto_key) != goto.seq
    {
        set_temp(ui, goto_key, goto.seq);
        let page = goto.page.min(n - 1);
        let off = top_of(page) - GAP + goto.top.unwrap_or(0.0).max(0.0) * scale;
        area = area.vertical_scroll_offset(off.max(0.0));
    }
    let find = s.preview_slot(pane).map(|sl| &sl.find);
    let reveal_key = id(pane, "reveal");
    let source_text = s.pdf_doc(pane).map(|d| d.text.clone());
    if let Some(f) = find
        && f.open
        && temp::<u64>(ui, reveal_key) != f.reveal
        && let Some(m) = &f.matches
        && let Some(hit) = m.hits.get(f.current)
    {
        set_temp(ui, reveal_key, f.reveal);
        let page = hit.segment.min(n - 1);
        let top = source_text
            .as_ref()
            .and_then(|t| t.get(page))
            .and_then(|t| {
                t.rects(hit.bytes.start, hit.bytes.end)
                    .first()
                    .map(|r| r[1])
            })
            .unwrap_or(0.0);
        let off = top_of(page) - GAP + (top * scale - ui.available_height() * 0.3).max(0.0);
        area = area.vertical_scroll_offset(off.max(0.0));
    }
    let tex_key = id(pane, "textures");
    let mut old: TexCache = ui.data_mut(|d| d.get_temp(tex_key)).unwrap_or_default();
    let mut used: TexCache = HashMap::new();
    let mut wanted_tiles: Vec<Value> = Vec::new();
    let mut visible: Vec<usize> = Vec::new();
    let mut current = view.page.min(n - 1);
    let fill = view
        .fill
        .as_ref()
        .filter(|f| f.editing && f.state == FillState::Ready);
    let drag_key = id(pane, "drag");
    let mut drag: Drag = temp(ui, drag_key);
    let mut pending = false;
    let output = area.show_viewport(ui, |ui, viewport| {
        ui.set_min_size(vec2(content_w, content_h));
        let origin = ui.min_rect().min;
        let width = ui.min_rect().width().max(viewport.width());
        // Pages that intersect the viewport.
        let first = tops
            .partition_point(|t| *t < viewport.min.y)
            .saturating_sub(1);
        let mut i = first;
        while i < n && top_of(i) < viewport.max.y {
            if top_of(i + 1) > viewport.min.y
                && visible.len() < fm_engine::pdf_view_cmds::MAX_VISIBLE
            {
                visible.push(i);
            }
            i += 1;
        }
        let probe = viewport.min.y + viewport.height() * 0.3;
        current = tops
            .partition_point(|t| *t <= probe)
            .saturating_sub(1)
            .min(n - 1);
        let painter = ui.painter().clone();
        let bg = ui
            .max_rect()
            .union(Rect::from_min_size(origin, vec2(width, content_h)));
        painter.rect_filled(bg, 0.0, tokens.pdf_gap);
        for &p in &visible {
            let Some(&(w, h)) = sizes.get(p) else {
                continue;
            };
            let x = origin.x + ((width - w * scale) / 2.0).max(GAP);
            let page_rect =
                Rect::from_min_size(pos2(x, origin.y + top_of(p)), vec2(w * scale, h * scale));
            let page_bg = match view.colors {
                PdfColors::Original => tokens.checker[0],
                PdfColors::Dark => tokens.panel,
                PdfColors::HighContrast => tokens.pdf_contrast_bg,
            };
            painter.rect_filled(page_rect, 0.0, page_bg);
            let whole = page_pixels(&shown.doc, p, step as f32 / 4.0)
                .is_some_and(|(pw, ph)| u64::from(pw) * u64::from(ph) <= MAX_PIXELS);
            if too_complex {
                painter.text(
                    page_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "This document is too complex to draw.",
                    egui::FontId::proportional(tokens.font_size),
                    tokens.error,
                );
            } else if whole {
                let key = RenderKey {
                    doc: shown.id,
                    page: p,
                    step,
                    colors,
                    tile: None,
                };
                match s.pdf.get(&key) {
                    Some(RenderState::Ready(px)) => {
                        let t = texture(ui, pane, &mut used, &mut old, &px);
                        egui::Image::new((t, page_rect.size())).paint_at(ui, page_rect);
                    }
                    Some(RenderState::Failed(why)) => {
                        painter.text(
                            page_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            why,
                            egui::FontId::proportional(tokens.font_size),
                            tokens.error,
                        );
                    }
                    _ => {
                        pending = true;
                        if let Some(px) = s.pdf.fallback(shown.id, p, colors) {
                            // A sharper version is on its way: show this one stretched.
                            let t = texture(ui, pane, &mut used, &mut old, &px);
                            egui::Image::new((t, page_rect.size())).paint_at(ui, page_rect);
                        } else {
                            painter.text(
                                page_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("Page {}", p + 1),
                                egui::FontId::proportional(tokens.font_size),
                                tokens.text_muted,
                            );
                        }
                    }
                }
            } else if let Some((pw, ph)) = page_pixels(&shown.doc, p, step as f32 / 4.0) {
                // Above the pixel cap: draw the visible part in tiles.
                let px_per_pt = step as f32 / 4.0;
                let vis = page_rect.intersect(viewport.translate(origin.to_vec2()));
                let to_px = |v: f32, base: f32| ((v - base) / scale * px_per_pt).max(0.0);
                let x0 = (to_px(vis.min.x, page_rect.min.x) as u32 / TILE) * TILE;
                let y0 = (to_px(vis.min.y, page_rect.min.y) as u32 / TILE) * TILE;
                let x1 = (to_px(vis.max.x, page_rect.min.x) as u32).min(pw);
                let y1 = (to_px(vis.max.y, page_rect.min.y) as u32).min(ph);
                let mut ty = y0;
                while ty < y1 && wanted_tiles.len() < 64 {
                    let mut tx = x0;
                    while tx < x1 && wanted_tiles.len() < 64 {
                        let tile = Tile {
                            x: tx,
                            y: ty,
                            width: TILE.min(pw - tx),
                            height: TILE.min(ph - ty),
                        };
                        wanted_tiles.push(json!([p, tile.x, tile.y, tile.width, tile.height]));
                        let r = Rect::from_min_size(
                            page_rect.min + vec2(tile.x as f32, tile.y as f32) / px_per_pt * scale,
                            vec2(tile.width as f32, tile.height as f32) / px_per_pt * scale,
                        );
                        let key = RenderKey {
                            doc: shown.id,
                            page: p,
                            step,
                            colors,
                            tile: Some(tile),
                        };
                        match s.pdf.get(&key) {
                            Some(RenderState::Ready(px)) => {
                                let t = texture(ui, pane, &mut used, &mut old, &px);
                                egui::Image::new((t, r.size())).paint_at(ui, r);
                            }
                            Some(RenderState::Failed(_)) => {}
                            _ => pending = true,
                        }
                        tx += TILE;
                    }
                    ty += TILE;
                }
            }
            painter.rect_stroke(
                page_rect,
                0.0,
                Stroke::new(tokens.border_width, tokens.pdf_page_border),
                egui::StrokeKind::Outside,
            );
            // Find highlights (Find searched the loaded document's text).
            if let (Some(f), Some(text)) = (find, source_text.as_ref().and_then(|t| t.get(p)))
                && f.open
                && let Some(m) = &f.matches
            {
                let lo = m.hits.partition_point(|h| h.segment < p);
                for (k, hit) in m
                    .hits
                    .iter()
                    .enumerate()
                    .skip(lo)
                    .take_while(|(_, h)| h.segment == p)
                {
                    let color = if k == f.current {
                        tokens.warning.gamma_multiply(0.55)
                    } else {
                        tokens.warning.gamma_multiply(0.3)
                    };
                    for r in text.rects(hit.bytes.start, hit.bytes.end) {
                        painter.rect_filled(to_screen(page_rect, scale, r), 1.0, color);
                    }
                }
            }
            // Text selection.
            let sel = match (drag.anchor, drag.head) {
                (Some(a), Some(b)) => Some(if a <= b { (a, b) } else { (b, a) }),
                _ => view.selection.map(|s| (s.start, s.end)),
            };
            if let Some((a, b)) = sel
                && (a.0..=b.0).contains(&p)
                && let Some(text) = shown.text.get(p)
            {
                let from = if p == a.0 { a.1 } else { 0 };
                let to = if p == b.0 { b.1 } else { usize::MAX };
                for r in text.rects(from, to.min(text.text.len())) {
                    painter.rect_filled(
                        to_screen(page_rect, scale, r),
                        0.0,
                        tokens.selection_bg.gamma_multiply(0.6),
                    );
                }
            }
            page_interaction(
                ui, s, pane, tokens, view, shown, fill, p, page_rect, scale, &mut drag, actions,
            );
        }
    });
    set_temp(ui, drag_key, drag);
    // Textures not drawn this frame are released.
    ui.data_mut(|d| d.insert_temp(tex_key, used));
    crate::frame_cache::touch::<TexCache>(ui.ctx(), tex_key);
    super::remember_scroll(ui, &output);
    let head = fill_menu(
        ui,
        s,
        pane,
        tokens,
        area_rect,
        view,
        info.forms.fields,
        actions,
    );
    // The signature state gets its own square on the pages, left of a Fill & Sign button.
    // It steps aside while Fill & Sign is open (its tiles use the space) or the
    // Signatures panel already shows the details.
    if view.fill.is_none() && !view.signatures_open {
        let doc = s.pdf_doc(pane).map_or(shown.id, |d| d.id);
        if let Some(notice) = sig_notice(s, doc, info.forms.signed, tokens) {
            signature_alert(ui, pane, tokens, area_rect, head, &notice, actions);
        }
    }
    // Tell the engine what is on screen when it changes.
    let contrast = [rgb(tokens.pdf_contrast_fg), rgb(tokens.pdf_contrast_bg)];
    let report = json!({
        "pages": visible,
        "scale": scale * ppp,
        "current": current,
        "tiles": wanted_tiles,
        "contrast": contrast,
    });
    let last_key = id(pane, "visible");
    let fingerprint = format!("{}:{report}", shown.id);
    if temp::<String>(ui, last_key) != fingerprint {
        set_temp(ui, last_key, fingerprint);
        actions.push(("preview.pdf.visible", report));
    }
    if pending {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

/// Clicks, drags and overlays on one page: links, selection, form fields, signatures.
#[allow(clippy::too_many_arguments)]
fn page_interaction(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    view: &PdfView,
    shown: &Shown,
    fill: Option<&PdfSession>,
    page: usize,
    page_rect: Rect,
    scale: f32,
    drag: &mut Drag,
    actions: &mut Vec<Action>,
) {
    let resp = ui.interact(
        page_rect,
        id(pane, "page").with(page),
        Sense::click_and_drag(),
    );
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let place_key = id(pane, "placing");
    let placing = temp::<bool>(ui, place_key) && fill.is_some_and(|f| f.pending_visual.is_some());
    // Placed signatures: drawn on top, draggable.
    let mut over_overlay = false;
    if let Some(f) = view.fill.as_ref() {
        over_overlay |= placements(
            ui, pane, tokens, f, page, page_rect, scale, pointer, actions,
        );
    }
    // Form fields while filling.
    if let Some(f) = fill
        && !placing
    {
        over_overlay |= fields(ui, pane, tokens, f, page, page_rect, scale, actions);
    }
    if placing {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if resp.clicked()
            && let (Some(p), Some(f)) = (pointer, fill)
            && let Some(v) = &f.pending_visual
        {
            let (x, y) = to_page(page_rect, scale, p);
            let (pw, ph) = (page_rect.width() / scale, page_rect.height() / scale);
            let w = PLACE_W.min(pw - 8.0).max(8.0);
            let h = (w * (v.height / v.width.max(1.0)) as f32).clamp(8.0, ph - 8.0);
            let x0 = (x - w / 2.0).clamp(0.0, (pw - w).max(0.0));
            let y0 = (y - h / 2.0).clamp(0.0, (ph - h).max(0.0));
            actions.push((
                "pdf.sign.place",
                json!({ "page": page, "rect": [x0, y0, x0 + w, y0 + h] }),
            ));
            set_temp(ui, place_key, false);
        }
        return;
    }
    if over_overlay {
        return;
    }
    // Links.
    let mut on_link = false;
    if let (Some(links), Some(p)) = (shown.text.links(page), resp.hover_pos()) {
        for (k, link) in links.iter().enumerate() {
            let r = to_screen(page_rect, scale, link.rect);
            if r.contains(p) {
                on_link = true;
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                let tip = match &link.target {
                    LinkTarget::Dest(d) => format!("Go to page {}", d.page + 1),
                    LinkTarget::Uri(u) => u.chars().take(200).collect(),
                    LinkTarget::Unsupported(w) => format!("Not followed: {w}"),
                };
                egui::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    id(pane, "link-tip"),
                    egui::PopupAnchor::Pointer,
                )
                .show(|ui| ui.label(tip));
                if resp.clicked() {
                    actions.push(("preview.pdf.link", json!({ "page": page, "index": k })));
                }
                break;
            }
        }
    }
    if on_link {
        return;
    }
    // Text selection by dragging.
    let text = shown.text.get(page);
    if text.as_ref().is_some_and(|t| !t.is_empty()) && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if resp.drag_started()
        && let (Some(p), Some(t)) = (pointer, &text)
    {
        let (x, y) = to_page(page_rect, scale, p);
        if let Some(o) = t.offset_at(x, y) {
            drag.anchor = Some((page, o));
            drag.head = Some((page, o));
        }
    }
    // The head follows the pointer onto whichever page it is over.
    if drag.anchor.is_some()
        && let Some(p) = ui.ctx().pointer_latest_pos()
        && page_rect.contains(p)
        && ui.input(|i| i.pointer.primary_down())
        && let Some(t) = &text
    {
        let (x, y) = to_page(page_rect, scale, p);
        if let Some(o) = t.offset_at(x, y) {
            drag.head = Some((page, o));
        }
    }
    if ui.input(|i| i.pointer.primary_released())
        && let (Some(a), Some(b)) = (drag.anchor, drag.head)
        && page_rect.contains(ui.ctx().pointer_latest_pos().unwrap_or(Pos2::ZERO))
    {
        if a == b {
            actions.push(("preview.pdf.select", json!({})));
        } else {
            actions.push((
                "preview.pdf.select",
                json!({ "start": {"page": a.0, "offset": a.1}, "end": {"page": b.0, "offset": b.1} }),
            ));
        }
        *drag = Drag::default();
    }
    if resp.clicked() && view.selection.is_some() {
        actions.push(("preview.pdf.select", json!({})));
    }
    let _ = s;
}

fn current_text(f: &PdfSession, field: &Field) -> String {
    match f.value(&field.name) {
        Some(FieldValue::Text(t)) => t.clone(),
        Some(FieldValue::Selected(v)) => v.join(", "),
        _ => String::new(),
    }
}

/// Form field overlays; returns whether the pointer is over one.
#[allow(clippy::too_many_arguments)]
fn fields(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    f: &PdfSession,
    page: usize,
    page_rect: Rect,
    scale: f32,
    actions: &mut Vec<Action>,
) -> bool {
    let edit_key = id(pane, "field-edit");
    let mut editing: Option<FieldEdit> = ui.data(|d| d.get_temp(edit_key));
    let had_editor = editing.is_some();
    let mut over = false;
    // Tab in a text field commits it and moves to the next text field.
    let mut tab_to: Option<String> = None;
    let blocked = f.blocked.is_some();
    for field in f.fields.iter() {
        for (wi, w) in field.widgets.iter().enumerate() {
            if w.page != page {
                continue;
            }
            let r = to_screen(page_rect, scale, w.rect);
            let fillable = field.fillable() && !blocked;
            let unsigned_sig = matches!(field.kind, FieldKind::Signature { signed: false });
            if !fillable && !unsigned_sig {
                continue;
            }
            let painter = ui.painter();
            painter.rect_filled(r, 1.0, tokens.field_fill);
            if field.required && f.value(&field.name).is_none_or(FieldValue::is_empty) {
                painter.rect_stroke(
                    r,
                    1.0,
                    Stroke::new(1.0, tokens.warning),
                    egui::StrokeKind::Inside,
                );
            }
            let resp = ui.interact(r, id(pane, "field").with((&field.name, wi)), Sense::click());
            let resp = match &field.tooltip {
                Some(t) => resp.on_hover_text(t),
                None => resp.on_hover_text(&field.name),
            };
            if resp.hovered() {
                over = true;
                ui.painter().rect_stroke(
                    r,
                    1.0,
                    Stroke::new(2.0, tokens.field_focus),
                    egui::StrokeKind::Outside,
                );
            }
            if editing.as_ref().is_some_and(|e| e.field == field.name) && wi == 0 {
                over = true;
                if text_editor(ui, pane, tokens, &mut editing, r, actions) {
                    tab_to = Some(field.name.clone());
                }
                continue;
            }
            match &field.kind {
                FieldKind::Text {
                    multiline,
                    password,
                    ..
                } if resp.clicked() => {
                    if *password {
                        actions.push(("preview.pdf.select", json!({})));
                    }
                    editing = Some(FieldEdit {
                        field: field.name.clone(),
                        text: current_text(f, field),
                        multiline: *multiline,
                        focused: false,
                    });
                }
                FieldKind::Checkbox { .. } => {
                    let on = !f.value(&field.name).is_none_or(FieldValue::is_empty);
                    if on {
                        mark(ui, tokens, r, "✔");
                    }
                    if resp.clicked() {
                        actions
                            .push(("pdf.form.set", json!({ "field": field.name, "value": !on })));
                    }
                }
                FieldKind::Radio { .. } => {
                    let state = w.on_state.clone().unwrap_or_default();
                    let on = matches!(f.value(&field.name), Some(FieldValue::State(Some(s))) if *s == state);
                    if on {
                        mark(ui, tokens, r, "●");
                    }
                    if resp.clicked() && !state.is_empty() {
                        actions.push((
                            "pdf.form.set",
                            json!({ "field": field.name, "value": state }),
                        ));
                    }
                }
                FieldKind::Combo { options, editable }
                | FieldKind::List {
                    options,
                    multi: editable,
                } => {
                    let multi = matches!(field.kind, FieldKind::List { multi: true, .. });
                    let combo_editable =
                        matches!(field.kind, FieldKind::Combo { editable: true, .. });
                    let _ = editable;
                    let selected: Vec<String> = match f.value(&field.name) {
                        Some(FieldValue::Selected(v)) => v.clone(),
                        _ => Vec::new(),
                    };
                    egui::Popup::menu(&resp).show(|ui| {
                        ui.set_min_width(r.width().max(140.0));
                        for o in options {
                            let on = selected.contains(&o.export);
                            if ui.selectable_label(on, &o.label).clicked() {
                                let value = if multi {
                                    let mut v = selected.clone();
                                    if on {
                                        v.retain(|x| x != &o.export);
                                    } else {
                                        v.push(o.export.clone());
                                    }
                                    json!(v)
                                } else {
                                    json!(o.export)
                                };
                                actions.push((
                                    "pdf.form.set",
                                    json!({ "field": field.name, "value": value }),
                                ));
                            }
                        }
                        if combo_editable && ui.button("Type a value…").clicked() {
                            ui.data_mut(|d| {
                                d.insert_temp(
                                    edit_key,
                                    FieldEdit {
                                        field: field.name.clone(),
                                        text: selected.first().cloned().unwrap_or_default(),
                                        multiline: false,
                                        focused: false,
                                    },
                                );
                            });
                        }
                    });
                }
                FieldKind::Signature { signed: false } if resp.clicked() => {
                    super::pdf_sign::open(ui.ctx(), pane, Some(field.name.clone()));
                }
                _ => {}
            }
        }
    }
    if let Some(from) = tab_to {
        let texts: Vec<&Field> = f
            .fields
            .iter()
            .filter(|x| {
                x.fillable() && matches!(x.kind, FieldKind::Text { .. }) && !x.widgets.is_empty()
            })
            .collect();
        let next = texts
            .iter()
            .position(|x| x.name == from)
            .and_then(|i| texts.get(i + 1));
        editing = next.map(|x| FieldEdit {
            field: x.name.clone(),
            text: current_text(f, x),
            multiline: matches!(
                x.kind,
                FieldKind::Text {
                    multiline: true,
                    ..
                }
            ),
            focused: false,
        });
        if let Some(w) = next.and_then(|x| x.widgets.first())
            && w.page != page
        {
            actions.push(("preview.pdf.page", json!({ "page": w.page + 1 })));
        }
    }
    // A "Type a value…" menu choice may have stored an editor this frame: keep it.
    match editing {
        Some(e) => set_temp(ui, edit_key, e),
        None if had_editor => ui.data_mut(|d| d.remove::<FieldEdit>(edit_key)),
        None => {}
    }
    over
}

fn mark(ui: &Ui, tokens: &Tokens, r: Rect, glyph: &str) {
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional((r.height() * 0.8).clamp(6.0, 48.0)),
        tokens.field_focus,
    );
}

/// The in-place editor of a text field: Enter (single line) or leaving it commits,
/// Escape cancels. Returns true when Tab committed it (move to the next field).
fn text_editor(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    editing: &mut Option<FieldEdit>,
    r: Rect,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(e) = editing.as_mut() else {
        return false;
    };
    let tab = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
    let rect = if e.multiline {
        r
    } else {
        r.expand2(vec2(0.0, (20.0 - r.height()).max(0.0) / 2.0))
    };
    let edit_id = id(pane, "field-editor");
    let mut commit = false;
    let mut cancel = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        let te = if e.multiline {
            egui::TextEdit::multiline(&mut e.text).lock_focus(false)
        } else {
            egui::TextEdit::singleline(&mut e.text)
        };
        let resp = ui.add_sized(rect.size(), te.id(edit_id).text_color(tokens.text));
        if !e.focused {
            resp.request_focus();
            e.focused = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        } else if resp.lost_focus() || tab {
            commit = true;
        }
    });
    if commit {
        actions.push(("pdf.form.set", json!({ "field": e.field, "value": e.text })));
        *editing = None;
    } else if cancel {
        *editing = None;
    }
    commit && tab
}

/// Placed visual signatures on `page`: outline, drag to move, × to remove.
#[allow(clippy::too_many_arguments)]
fn placements(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    f: &PdfSession,
    page: usize,
    page_rect: Rect,
    scale: f32,
    pointer: Option<Pos2>,
    actions: &mut Vec<Action>,
) -> bool {
    let move_key = id(pane, "moving");
    let mut moving: Option<Moving> = ui.data(|d| d.get_temp(move_key));
    let mut over = false;
    let (pw, ph) = (page_rect.width() / scale, page_rect.height() / scale);
    for p in f.placements.iter().filter(|p| p.page == page) {
        let rect = moving.filter(|m| m.id == p.id).map_or(p.rect, |m| m.rect);
        let r = to_screen(page_rect, scale, rect);
        let resp = ui.interact(r, id(pane, "placement").with(p.id), Sense::click_and_drag());
        over |= resp.hovered() || resp.dragged();
        ui.painter().rect_stroke(
            r,
            2.0,
            Stroke::new(1.5, tokens.field_focus),
            egui::StrokeKind::Outside,
        );
        if p.field.is_none() {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if resp.dragged() {
                let d = resp.drag_delta() / scale;
                let w = rect[2] - rect[0];
                let h = rect[3] - rect[1];
                let x0 = (rect[0] + d.x).clamp(0.0, (pw - w).max(0.0));
                let y0 = (rect[1] + d.y).clamp(0.0, (ph - h).max(0.0));
                moving = Some(Moving {
                    id: p.id,
                    page,
                    rect: [x0, y0, x0 + w, y0 + h],
                });
            }
            if resp.drag_stopped()
                && let Some(m) = moving.take()
            {
                actions.push((
                    "pdf.sign.move",
                    json!({ "id": m.id, "rect": m.rect, "page": m.page }),
                ));
            }
        }
        let close = Rect::from_center_size(r.right_top(), vec2(16.0, 16.0));
        let x = ui.interact(close, id(pane, "placement-x").with(p.id), Sense::click());
        ui.painter()
            .circle_filled(close.center(), 8.0, tokens.panel);
        ui.painter().text(
            close.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(12.0),
            tokens.text,
        );
        let x = x.on_hover_text("Remove this signature");
        over |= x.hovered();
        if x.clicked() {
            actions.push(("pdf.sign.remove", json!({ "id": p.id })));
        }
    }
    let _ = pointer;
    match moving {
        Some(m) => set_temp(ui, move_key, m),
        None => ui.data_mut(|d| d.remove::<Moving>(move_key)),
    }
    over
}

/// Whether the Fill & Sign strip under the toolbar has anything to say: progress, errors,
/// a question before saving, the Save as path or the Replace confirmation, or notes about
/// the form. The actions themselves are the tiles on the pages.
fn needs_strip(ui: &Ui, pane: PaneId, f: &PdfSession) -> bool {
    f.state != FillState::Ready
        || f.blocked.is_some()
        || f.xfa_hybrid
        || f.scripts
        || f.error.is_some()
        || f.prompt.is_some()
        || f.save != SaveState::Idle
        || temp::<Confirm>(ui, id(pane, "confirm")) != Confirm::None
}

/// Fill & Sign messages: progress, errors, questions before saving.
fn fill_strip(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    f: &PdfSession,
    actions: &mut Vec<Action>,
) {
    let confirm_key = id(pane, "confirm");
    let mut confirm: Confirm = temp(ui, confirm_key);
    match &f.state {
        FillState::Opening => {
            ui.horizontal(|ui| {
                ui.add(crate::spinner::Spinner::new());
                ui.label("Preparing the form…");
            });
            return;
        }
        FillState::Failed(why) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(why).color(tokens.error));
                if ui.button("Close").clicked() {
                    actions.push(("pdf.discard", json!({})));
                }
            });
            return;
        }
        FillState::Ready => {}
    }
    match &f.save {
        SaveState::Building | SaveState::Writing(_) => {
            ui.horizontal(|ui| {
                ui.add(crate::spinner::Spinner::new());
                ui.label("Saving…");
            });
        }
        SaveState::Failed(e) => {
            ui.label(RichText::new(format!("Not saved: {e}")).color(tokens.error));
        }
        SaveState::Idle => {}
    }
    if let Some(why) = &f.blocked {
        ui.label(RichText::new(why).color(tokens.warning));
    }
    if f.xfa_hybrid {
        crate::dialog_style::hint(
            ui,
            tokens,
            "This form also has an XFA part; saving removes it so every viewer shows the values you entered.",
        );
    }
    if f.scripts {
        // Scripts are never run (security): one short line pointing to a full PDF app.
        crate::dialog_style::hint(
            ui,
            tokens,
            "Form scripts don't run here, so totals won't update. Use a PDF app for this form.",
        );
    }
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(tokens.error));
    }
    if let Some(p) = &f.prompt {
        ui.horizontal_wrapped(|ui| {
            if !p.empty_required.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Required fields are empty: {}.",
                        p.empty_required.join(", ")
                    ))
                    .color(tokens.warning),
                );
            }
            if p.breaks_signatures {
                ui.label(
                    RichText::new(
                        "Adding a signature on the page marks the existing digital signatures as changed.",
                    )
                    .color(tokens.warning),
                );
            }
            if ui.button("Save anyway").clicked() {
                match &p.target {
                    SaveTarget::Copy => actions.push(("pdf.save", json!({ "confirm": true }))),
                    SaveTarget::As(uri) => actions.push((
                        "pdf.save_as",
                        json!({ "uri": uri.to_string(), "confirm": true }),
                    )),
                    SaveTarget::Overwrite => {
                        actions.push(("pdf.save_overwrite", json!({ "confirm": true })))
                    }
                }
            }
        });
    }
    match &mut confirm {
        Confirm::None => {}
        Confirm::Overwrite => {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(
                        "Replace the original file with the filled version? This cannot be undone.",
                    )
                    .color(tokens.warning),
                );
                if ui.button("Replace").clicked() {
                    actions.push(("pdf.save_overwrite", json!({ "confirm": true })));
                    confirm = Confirm::None;
                } else if ui.button("Cancel").clicked() {
                    confirm = Confirm::None;
                }
            });
        }
        Confirm::SaveAs(path) => {
            let mut done = None;
            ui.horizontal(|ui| {
                ui.label("Save as:");
                ui.add(
                    egui::TextEdit::singleline(path).desired_width(ui.available_width() - 140.0),
                );
                if ui.button("Save").clicked() && !path.trim().is_empty() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
            match done {
                Some(true) => {
                    actions.push(("pdf.save_as", json!({ "uri": path.trim() })));
                    confirm = Confirm::None;
                }
                Some(false) => confirm = Confirm::None,
                None => {}
            }
        }
    }
    set_temp(ui, confirm_key, confirm);
}

/// Size of the Fill & Sign button, before the pane's zoom.
const FILL_BUTTON: f32 = 84.0;
/// Height of an action tile under it.
const TILE_H: f32 = 58.0;
/// Space between tiles.
const TILE_GAP: f32 = 6.0;

/// The painted icon of an action tile.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Pen,
    Target,
    Star,
    Reset,
    Save,
    SaveAs,
    Replace,
    Close,
}

/// One action tile.
struct ActionTile {
    label: &'static str,
    tip: &'static str,
    glyph: Glyph,
    fill: Color32,
    enabled: bool,
    selected: bool,
    action: TileAction,
}

enum TileAction {
    Command(&'static str),
    OpenSignature,
    TogglePlacing,
    Confirm(Confirm),
}

/// Short status for the header tile while Fill & Sign is open.
fn status(f: &PdfSession) -> String {
    if f.is_saving() {
        return "Saving…".into();
    }
    match (f.values.len(), f.placements.len()) {
        (0, 0) => match f.fields.iter().filter(|x| x.fillable()).count() {
            0 => "No fields".into(),
            1 => "1 field".into(),
            n => format!("{n} fields"),
        },
        (v, 0) => format!("{v} changed"),
        (0, p) => format!("{p} signed"),
        (v, p) => format!("{} changes", v + p),
    }
}

fn summary(f: &PdfSession) -> String {
    match (f.values.len(), f.placements.len()) {
        (0, 0) => "Nothing changed yet. Fill in fields or add a signature.".into(),
        (v, p) => {
            let mut parts = Vec::new();
            if v > 0 {
                parts.push(format!(
                    "{v} field{} changed",
                    if v == 1 { "" } else { "s" }
                ));
            }
            if p > 0 {
                parts.push(format!(
                    "{p} signature{} placed",
                    if p == 1 { "" } else { "s" }
                ));
            }
            format!("{}, not saved yet.", parts.join(", "))
        }
    }
}

/// The Fill & Sign button floating at the top right of the pages: large on forms, so
/// filling them is hard to miss; other PDFs have it in the preview bar (`bar_button`). While Fill & Sign is open it heads a column of coloured
/// action tiles (wrapping into more columns when the pane is short).
#[allow(clippy::too_many_arguments)]
fn fill_menu(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    area: Rect,
    view: &PdfView,
    fields: usize,
    actions: &mut Vec<Action>,
) -> Option<Rect> {
    let z = tokens.zoom;
    let side = FILL_BUTTON * z;
    // Clear of the vertical scrollbar and the page edge.
    let margin = 20.0 * z;
    let open = view
        .fill
        .as_ref()
        .filter(|f| f.editing && f.state == FillState::Ready);
    // Most PDFs are not forms: their Fill & Sign button is in the preview bar
    // (`bar_button`), so nothing covers the pages until Fill & Sign is open.
    if open.is_none() && fields == 0 {
        return None;
    }
    if area.width() < side + 2.0 * margin || area.height() < side + 2.0 * margin {
        return None;
    }
    let collapsed_key = id(pane, "menu-collapsed");
    let collapsed: bool = temp(ui, collapsed_key);
    let head = Rect::from_min_size(
        pos2(area.max.x - side - margin, area.min.y + margin),
        vec2(side, side),
    );
    let resp = ui.interact(head, id(pane, "fill-sign-button"), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Fill & Sign"));
    let hovered = resp.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let radius = tokens.corner_radius * 2.0 * z;
    let painter = ui.painter().clone();
    shadow(&painter, tokens, head, radius);
    let fill = if hovered {
        tokens.accent_text
    } else {
        tokens.accent
    };
    painter.rect_filled(head, radius, fill);
    if resp.has_focus() {
        focus_ring(&painter, tokens, head, radius);
    }
    let ink = tokens.action_text;
    sign_icon(
        &painter,
        pos2(head.center().x, head.min.y + side * 0.36),
        side * 0.38,
        ink,
    );
    let sub = match open {
        Some(f) => Some(status(f)),
        None if fields == 1 => Some("1 field".into()),
        None if fields > 1 => Some(format!("{fields} fields")),
        None => None,
    };
    let label_y = head.min.y + side * if sub.is_some() { 0.70 } else { 0.76 };
    painter.text(
        pos2(head.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "Fill & Sign",
        egui::FontId::proportional((13.0 * z).max(9.0)),
        ink,
    );
    if let Some(sub) = &sub {
        painter.text(
            pos2(head.center().x, head.min.y + side * 0.86),
            egui::Align2::CENTER_CENTER,
            sub,
            egui::FontId::proportional((11.0 * z).max(8.0)),
            ink.gamma_multiply(0.85),
        );
    }
    if open.is_some() {
        // A chevron says the tiles fold away.
        let c = pos2(head.max.x - 10.0 * z, head.min.y + 10.0 * z);
        let d = 3.5 * z;
        let pts = if collapsed {
            vec![
                c + vec2(-d, -d * 0.5),
                c + vec2(0.0, d * 0.5),
                c + vec2(d, -d * 0.5),
            ]
        } else {
            vec![
                c + vec2(-d, d * 0.5),
                c + vec2(0.0, -d * 0.5),
                c + vec2(d, d * 0.5),
            ]
        };
        painter.add(egui::Shape::line(pts, Stroke::new(1.5 * z, ink)));
    }
    let tip = match open {
        Some(f) => format!("{} Click to {} the actions.", summary(f), if collapsed { "show" } else { "hide" }),
        None if fields > 0 => {
            "This PDF is a form: fill it in and sign it. The original stays unchanged until you save.".into()
        }
        None => "Add your signature (drawn, typed, an image or a digital certificate). The original stays unchanged until you save.".into(),
    };
    if resp.on_hover_text(tip).clicked() {
        if open.is_some() {
            set_temp(ui, collapsed_key, !collapsed);
        } else {
            set_temp(ui, collapsed_key, false);
            actions.push(("pdf.form.edit", json!({})));
        }
    }
    let Some(f) = open else {
        return Some(head);
    };
    if collapsed {
        return Some(head);
    }
    let busy = f.is_saving();
    let dirty = f.is_dirty();
    let place_key = id(pane, "placing");
    let placing: bool = temp(ui, place_key);
    let mut tiles = vec![ActionTile {
        label: "Signature",
        tip: "Make a signature: draw it, type it, use an image, or sign digitally with a certificate",
        glyph: Glyph::Pen,
        fill: tokens.action_sign,
        enabled: !busy,
        selected: false,
        action: TileAction::OpenSignature,
    }];
    if f.pending_visual.is_some() {
        tiles.push(ActionTile {
            label: "Place",
            tip: "Click on a page to put your signature there; drag it to adjust",
            glyph: Glyph::Target,
            fill: tokens.action_place,
            enabled: !busy,
            selected: placing,
            action: TileAction::TogglePlacing,
        });
        if s.pdf_signatures_dir.is_some() && s.pdf_saved.len() < fm_engine::pdf_sign_cmds::MAX_SAVED
        {
            tiles.push(ActionTile {
                label: "Remember",
                tip: "Keep this signature on this computer for next time",
                glyph: Glyph::Star,
                fill: tokens.action_remember,
                enabled: !busy,
                selected: false,
                action: TileAction::Command("pdf.sign.saved.store"),
            });
        }
    }
    if !f.values.is_empty() {
        tiles.push(ActionTile {
            label: "Reset",
            tip: "Put every field back to its value in the file",
            glyph: Glyph::Reset,
            fill: tokens.action_neutral,
            enabled: !busy,
            selected: false,
            action: TileAction::Command("pdf.form.reset"),
        });
    }
    tiles.extend([
        ActionTile {
            label: "Save copy",
            tip: "Save next to the original as “… (filled).pdf” or “… (signed).pdf”",
            glyph: Glyph::Save,
            fill: tokens.action_save,
            enabled: dirty && !busy,
            selected: false,
            action: TileAction::Command("pdf.save"),
        },
        ActionTile {
            label: "Save as…",
            tip: "Choose where to save the filled copy",
            glyph: Glyph::SaveAs,
            fill: tokens.action_save_as,
            enabled: dirty && !busy,
            selected: false,
            action: TileAction::Confirm(Confirm::SaveAs(fm_engine::item_cmds::display_path(
                &f.uri,
            ))),
        },
        ActionTile {
            label: "Replace",
            tip: "Write the changes into the original file (asks first)",
            glyph: Glyph::Replace,
            fill: tokens.action_replace,
            enabled: dirty && !busy,
            selected: false,
            action: TileAction::Confirm(Confirm::Overwrite),
        },
        ActionTile {
            label: if dirty { "Discard" } else { "Close" },
            tip: if dirty {
                "Throw away the changes and keep the original as it is"
            } else {
                "Close Fill & Sign"
            },
            glyph: Glyph::Close,
            fill: if dirty {
                tokens.action_discard
            } else {
                tokens.action_neutral
            },
            enabled: !busy,
            selected: false,
            action: TileAction::Command("pdf.discard"),
        },
    ]);
    // Lay the tiles out under the header, wrapping into columns to the left.
    let tile = vec2(side, TILE_H * z);
    let gap = TILE_GAP * z;
    let top = head.max.y + gap;
    let per_col = (((area.max.y - margin - top + gap) / (tile.y + gap)).floor() as usize).max(1);
    for (k, t) in tiles.into_iter().enumerate() {
        let col = k / per_col;
        let row = k % per_col;
        let x = head.min.x - col as f32 * (tile.x + gap);
        if x < area.min.x + margin {
            break;
        }
        let r = Rect::from_min_size(pos2(x, top + row as f32 * (tile.y + gap)), tile);
        let resp = ui.interact(r, id(pane, "fill-tile").with(t.label), Sense::click());
        let label = t.label;
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, t.enabled, label));
        shadow(&painter, tokens, r, radius);
        let (bg, fg) = if t.enabled {
            (
                if resp.hovered() {
                    t.fill.gamma_multiply(0.85)
                } else {
                    t.fill
                },
                tokens.action_text,
            )
        } else {
            (tokens.panel, tokens.text_muted)
        };
        painter.rect_filled(r, radius, bg);
        if !t.enabled {
            painter.rect_stroke(
                r,
                radius,
                Stroke::new(tokens.border_width, tokens.border),
                egui::StrokeKind::Inside,
            );
        }
        if t.selected {
            painter.rect_stroke(
                r.shrink(3.0 * z),
                radius,
                Stroke::new(2.0 * z, fg),
                egui::StrokeKind::Inside,
            );
        }
        if resp.has_focus() {
            focus_ring(&painter, tokens, r, radius);
        }
        glyph(
            &painter,
            t.glyph,
            pos2(r.center().x, r.min.y + tile.y * 0.38),
            tile.y * 0.38,
            fg,
        );
        painter.text(
            pos2(r.center().x, r.min.y + tile.y * 0.78),
            egui::Align2::CENTER_CENTER,
            t.label,
            egui::FontId::proportional((12.0 * z).max(8.0)),
            fg,
        );
        if t.enabled {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let resp = resp.on_hover_text(t.tip);
            if resp.clicked() {
                match t.action {
                    TileAction::Command(id) => actions.push((id, json!({}))),
                    TileAction::OpenSignature => super::pdf_sign::open(ui.ctx(), pane, None),
                    TileAction::TogglePlacing => set_temp(ui, place_key, !placing),
                    TileAction::Confirm(c) => set_temp(ui, id(pane, "confirm"), c),
                }
            }
        } else {
            let _ = resp.on_hover_text(t.tip);
        }
    }
    Some(head)
}

/// Whether the preview bar shows the Fill & Sign button for this pane.
pub(super) fn bar_button_shown(s: &Session, pane: PaneId) -> bool {
    let (Some(view), Some(shown)) = (s.pdf_view(pane), s.pdf_shown(pane)) else {
        return false;
    };
    view.fill.is_none() && shown.doc.info().forms.fields == 0
}

/// The Fill & Sign button at the right end of the preview bar, for PDFs without form
/// fields: an accent button with the pen icon that never folds into the view-options
/// menu, so it is easy to find without covering the pages. Forms get
/// the large button on the pages instead, and once Fill & Sign is open its actions are
/// there too, so the bar button is shown only while nothing is being filled.
pub(super) fn bar_button(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<Action>,
) {
    if !bar_button_shown(s, pane) {
        return;
    }
    let ink = tokens.action_text;
    let h = ui.spacing().interact_size.y;
    let icon = h * 0.7;
    // Room on the left of the label for the painted pen, whose barrel reaches about 0.8
    // of its size right of the centre.
    let indent = icon * 1.3 + 6.0;
    let label = RichText::new("Fill & Sign").color(ink).strong();
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let pad = ui.spacing().button_padding.x;
    let size = vec2(pad + indent + galley.size().x + pad, h);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Fill & Sign"));
    let hovered = resp.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    let radius = tokens.corner_radius;
    painter.rect_filled(
        r,
        radius,
        if hovered {
            tokens.accent_text
        } else {
            tokens.accent
        },
    );
    if resp.has_focus() {
        focus_ring(painter, tokens, r, radius);
    }
    sign_icon(
        painter,
        pos2(r.min.x + pad + icon * 0.5, r.center().y - h * 0.04),
        icon,
        ink,
    );
    painter.galley(
        pos2(r.min.x + pad + indent, r.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    let tip = "Add your signature (drawn, typed, an image or a digital certificate). The original stays unchanged until you save.";
    if resp.on_hover_text(tip).clicked() {
        set_temp(ui, id(pane, "menu-collapsed"), false);
        actions.push(("pdf.form.edit", json!({})));
    }
}

/// A square on the pages in the signature-state colour, so a broken, changed or intact
/// signature is seen straight away. Clicking it opens the Signatures panel.
fn signature_alert(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    area: Rect,
    head: Option<Rect>,
    notice: &SigNotice,
    actions: &mut Vec<Action>,
) {
    let z = tokens.zoom;
    let side = FILL_BUTTON * z;
    let margin = 20.0 * z;
    let right = head.map_or(area.max.x - margin, |h| h.min.x - TILE_GAP * z);
    let r = Rect::from_min_size(pos2(right - side, area.min.y + margin), vec2(side, side));
    if r.min.x < area.min.x + margin || area.height() < side + 2.0 * margin {
        return;
    }
    let resp = ui.interact(r, id(pane, "signature-alert"), Sense::click());
    let label = format!("Signature {}", notice.word);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let hovered = resp.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter().clone();
    let radius = tokens.corner_radius * 2.0 * z;
    shadow(&painter, tokens, r, radius);
    painter.rect_filled(
        r,
        radius,
        if hovered {
            notice.fill.gamma_multiply(0.85)
        } else {
            notice.fill
        },
    );
    if resp.has_focus() {
        focus_ring(&painter, tokens, r, radius);
    }
    let ink = tokens.action_text;
    let c = pos2(r.center().x, r.min.y + side * 0.32);
    let t = side * 0.17;
    let line = Stroke::new((2.0 * z).max(1.5), ink);
    if notice.ok {
        // A check mark in a circle.
        painter.circle_stroke(c, t * 1.05, line);
        painter.add(egui::Shape::line(
            vec![
                c + vec2(-t * 0.5, 0.0),
                c + vec2(-t * 0.12, t * 0.4),
                c + vec2(t * 0.55, -t * 0.38),
            ],
            line,
        ));
    } else {
        warning_triangle(&painter, c, t, line, ink);
    }
    let font = egui::FontId::proportional((12.0 * z).max(8.0));
    painter.text(
        pos2(r.center().x, r.min.y + side * 0.66),
        egui::Align2::CENTER_CENTER,
        "Signature",
        font.clone(),
        ink,
    );
    painter.text(
        pos2(r.center().x, r.min.y + side * 0.83),
        egui::Align2::CENTER_CENTER,
        notice.word,
        font,
        ink,
    );
    if resp.on_hover_text(notice.tip).clicked() {
        actions.push(("preview.pdf.signatures.toggle", json!({})));
    }
}

/// Warning triangle with an exclamation mark, centred on `c`.
fn warning_triangle(painter: &egui::Painter, c: Pos2, t: f32, line: Stroke, ink: Color32) {
    painter.add(egui::Shape::closed_line(
        vec![
            c + vec2(0.0, -t),
            c + vec2(t * 1.1, t * 0.85),
            c + vec2(-t * 1.1, t * 0.85),
        ],
        line,
    ));
    painter.line_segment([c + vec2(0.0, -t * 0.35), c + vec2(0.0, t * 0.3)], line);
    painter.circle_filled(c + vec2(0.0, t * 0.58), (line.width * 0.65).max(1.0), ink);
}

fn shadow(painter: &egui::Painter, tokens: &Tokens, r: Rect, radius: f32) {
    painter.rect_filled(
        r.translate(vec2(0.0, 3.0 * tokens.zoom)),
        radius,
        tokens.pdf_gap.gamma_multiply(0.5),
    );
}

fn focus_ring(painter: &egui::Painter, tokens: &Tokens, r: Rect, radius: f32) {
    painter.rect_stroke(
        r.expand(3.0),
        radius + 3.0,
        Stroke::new(2.0, tokens.accent),
        egui::StrokeKind::Outside,
    );
}

/// Tile icons, drawn from shapes so they scale with the pane and need no icon font.
fn glyph(p: &egui::Painter, g: Glyph, c: Pos2, size: f32, color: Color32) {
    let w = (size * 0.1).max(1.5);
    let stroke = Stroke::new(w, color);
    let h = size / 2.0;
    match g {
        Glyph::Pen => sign_icon(p, c, size, color),
        Glyph::Target => {
            p.circle_stroke(c, h * 0.8, stroke);
            p.circle_filled(c, w * 1.2, color);
            for d in [
                vec2(1.0, 0.0),
                vec2(-1.0, 0.0),
                vec2(0.0, 1.0),
                vec2(0.0, -1.0),
            ] {
                p.line_segment([c + d * h * 0.45, c + d * h * 1.05], stroke);
            }
        }
        Glyph::Star => {
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    let r = if i % 2 == 0 { h * 0.95 } else { h * 0.42 };
                    c + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            let mut closed = pts.clone();
            if let Some(first) = pts.first() {
                closed.push(*first);
            }
            p.add(egui::Shape::line(closed, stroke));
        }
        Glyph::Reset => {
            // A counter-clockwise arc with an arrow head.
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = 0.6 + i as f32 / 20.0 * 4.6;
                    c + vec2(a.cos(), -a.sin()) * h * 0.75
                })
                .collect();
            let start = pts.first().copied().unwrap_or(c);
            p.add(egui::Shape::line(pts, stroke));
            p.add(egui::Shape::convex_polygon(
                vec![
                    start + vec2(-w * 1.5, -w * 2.5),
                    start + vec2(w * 2.5, -w * 0.5),
                    start + vec2(-w * 0.5, w * 1.5),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Glyph::Save | Glyph::SaveAs | Glyph::Replace => {
            // A floppy disk: body, shutter and label.
            let body = Rect::from_center_size(c, vec2(size * 0.8, size * 0.8));
            p.rect_stroke(body, 2.0, stroke, egui::StrokeKind::Middle);
            let shutter = Rect::from_min_max(
                body.min + vec2(body.width() * 0.25, 0.0),
                pos2(
                    body.max.x - body.width() * 0.25,
                    body.min.y + body.height() * 0.32,
                ),
            );
            p.rect_filled(shutter, 1.0, color);
            let label = Rect::from_min_max(
                pos2(
                    body.min.x + body.width() * 0.2,
                    body.max.y - body.height() * 0.38,
                ),
                pos2(body.max.x - body.width() * 0.2, body.max.y - w),
            );
            p.rect_stroke(
                label,
                1.0,
                Stroke::new(w * 0.8, color),
                egui::StrokeKind::Middle,
            );
            let badge = body.right_bottom() + vec2(w, w);
            match g {
                Glyph::SaveAs => {
                    // Three dots: choose where.
                    for k in 0..3 {
                        p.circle_filled(badge + vec2(-(k as f32) * w * 2.2, 0.0), w * 0.8, color);
                    }
                }
                Glyph::Replace => {
                    // An exclamation mark: this changes the original.
                    let b = badge + vec2(0.0, -size * 0.35);
                    p.line_segment([b, b + vec2(0.0, size * 0.22)], stroke);
                    p.circle_filled(b + vec2(0.0, size * 0.33), w * 0.7, color);
                }
                _ => {}
            }
        }
        Glyph::Close => {
            let d = h * 0.6;
            p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
            p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
        }
    }
}

/// A pen writing a signature: a slanted pen with its nib on the end of a squiggle.
fn sign_icon(painter: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    let stroke = Stroke::new((size * 0.09).max(1.5), color);
    // Signature squiggle along the bottom.
    let base = c.y + size * 0.36;
    let left = c.x - size * 0.5;
    let pts: Vec<Pos2> = (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            let x = left + t * size * 0.72;
            let y = base - (t * std::f32::consts::TAU * 1.5).sin() * size * 0.12 * (1.0 - t * 0.4);
            pos2(x, y)
        })
        .collect();
    let nib = *pts.last().unwrap_or(&c);
    painter.add(egui::Shape::line(pts, stroke));
    // The pen: a body from the nib up to the top right, at 45 degrees.
    let dir = vec2(0.70, -0.72).normalized();
    let normal = vec2(-dir.y, dir.x);
    let tip_len = size * 0.18;
    let body_len = size * 0.62;
    let half = size * 0.11;
    let tip_end = nib + dir * tip_len;
    let top = tip_end + dir * body_len;
    // Nib (a triangle) and body (a quad), filled; a cap line near the top.
    painter.add(egui::Shape::convex_polygon(
        vec![nib, tip_end + normal * half, tip_end - normal * half],
        color,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip_end + normal * half,
            top + normal * half,
            top - normal * half,
            tip_end - normal * half,
        ],
        color,
        Stroke::NONE,
    ));
    let cap = tip_end + dir * (body_len * 0.72);
    painter.line_segment(
        [cap + normal * (half + 1.5), cap - normal * (half + 1.5)],
        Stroke::new(stroke.width * 0.8, color),
    );
}
