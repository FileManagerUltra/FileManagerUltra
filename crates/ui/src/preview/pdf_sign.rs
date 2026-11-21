//! The signature dialog (design D7–D9): draw, type or pick an image for a visual
//! signature, reuse a remembered one, or sign digitally with a certificate file.
//! It only dispatches `pdf.sign.*` commands; the certificate password is sent once and
//! cleared from the dialog right after.

use egui::{Context, Id, Pos2, RichText, Sense, Stroke, Ui, vec2};
use fm_engine::Session;
use fm_engine::pdf_view::PdfView;
use fm_layout::PaneId;
use fm_pdf::forms::FieldKind;
use fm_pdf::visual::VisualKind;
use serde_json::json;

use super::Action;
use crate::theme::Tokens;

/// Drawing pad size in points (also the pad size sent with the strokes).
const PAD: (f32, f32) = (420.0, 150.0);
const IMAGE_PURPOSE: &str = "pdf.sign.image";
const CERT_PURPOSE: &str = "pdf.sign.cert";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Draw,
    Type,
    Image,
    Saved,
    Digital,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Look {
    #[default]
    NameDate,
    Visual,
    Invisible,
}

#[derive(Clone, Default)]
struct State {
    pane: PaneId,
    /// The empty signature field this is for, if any.
    field: Option<String>,
    tab: Tab,
    strokes: Vec<Vec<[f32; 2]>>,
    name: String,
    image: String,
    remove_white: bool,
    cert: String,
    password: String,
    reason: String,
    location: String,
    look: Look,
}

fn key() -> Id {
    Id::NULL.with("pv-pdf-sign-dialog")
}

fn pending_key(pane: PaneId) -> Id {
    Id::NULL.with(("pv-pdf-sign-place-into", pane))
}

/// Open the dialog for `pane` (optionally for an empty signature field).
pub(super) fn open(ctx: &Context, pane: PaneId, field: Option<String>) {
    ctx.data_mut(|d| {
        let mut st: State = d.get_temp(key()).unwrap_or_default();
        if st.pane != pane {
            st = State::default();
        }
        st.pane = pane;
        st.field = field;
        st.password.clear();
        d.insert_temp(key(), Some(st));
    });
}

fn take_picked(s: &mut Session, purpose: &str) -> Option<String> {
    if s.picked.as_ref().is_some_and(|(p, _)| p == purpose) {
        return s.picked.take().map(|(_, path)| path);
    }
    None
}

/// Draw the dialog if it is open for `pane`, and finish a pending "place into field".
pub(super) fn dialog(
    ctx: &Context,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<Action>,
) {
    let image = take_picked(s, IMAGE_PURPOSE);
    let cert = take_picked(s, CERT_PURPOSE);
    let s: &Session = s;
    let Some(view) = s.pdf_view(pane) else {
        return;
    };
    // A visual signature made for a field goes into it once it is ready.
    if let Some(field) = ctx.data(|d| d.get_temp::<String>(pending_key(pane)))
        && let Some(f) = &view.fill
        && !f.visual_loading
    {
        ctx.data_mut(|d| d.remove_temp::<String>(pending_key(pane)));
        if f.pending_visual.is_some() && f.error.is_none() {
            actions.push(("pdf.sign.place", json!({ "field": field })));
        }
    }
    let Some(mut st) = ctx.data(|d| d.get_temp::<Option<State>>(key())).flatten() else {
        return;
    };
    if st.pane != pane {
        return;
    }
    let Some(fill) = &view.fill else {
        ctx.data_mut(|d| d.insert_temp::<Option<State>>(key(), None));
        return;
    };
    if let Some(path) = image {
        st.image = path;
    }
    if let Some(path) = cert {
        st.cert = path;
    }
    let mut open = true;
    let mut close = false;
    let title = match &st.field {
        Some(f) => format!("Sign “{f}”"),
        None => "Signature".to_string(),
    };
    crate::dialog_style::window(ctx, tokens, &title, &mut open)
        .id(Id::NULL.with(("pv-pdf-sign-window", pane)))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.horizontal(|ui| {
                for (t, label) in [
                    (Tab::Draw, "Draw"),
                    (Tab::Type, "Type"),
                    (Tab::Image, "Image"),
                    (Tab::Saved, "Remembered"),
                    (Tab::Digital, "Digital"),
                ] {
                    if ui
                        .add(egui::Button::selectable(st.tab == t, label))
                        .clicked()
                    {
                        st.tab = t;
                        if t == Tab::Saved && s.pdf_saved.is_empty() && !s.pdf_saved_loading {
                            actions.push(("pdf.sign.saved.load", json!({})));
                        }
                    }
                }
            });
            ui.separator();
            let visual = match st.tab {
                Tab::Draw => draw_tab(ui, tokens, &mut st),
                Tab::Type => type_tab(ui, tokens, &mut st),
                Tab::Image => image_tab(ui, s, tokens, &mut st, actions),
                Tab::Saved => saved_tab(ui, s, tokens, &mut st, actions),
                Tab::Digital => {
                    if digital_tab(ui, s, tokens, view, &mut st, actions) {
                        close = true;
                    }
                    None
                }
            };
            if let Some(v) = visual {
                actions.push(("pdf.sign.visual", v));
                match st.field.clone() {
                    Some(field) => ctx.data_mut(|d| {
                        d.insert_temp(pending_key(pane), field);
                    }),
                    None => ctx.data_mut(|d| {
                        d.insert_temp(Id::NULL.with(("pv-pdf", pane, "placing")), true);
                    }),
                }
                close = true;
            }
            if let Some(e) = &fill.error {
                ui.label(RichText::new(e).color(tokens.error));
            }
        });
    if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.password.clear();
        ctx.data_mut(|d| d.insert_temp::<Option<State>>(key(), None));
    } else {
        ctx.data_mut(|d| d.insert_temp(key(), Some(st)));
    }
}

fn use_button(ui: &mut Ui, enabled: bool, field: bool) -> bool {
    let label = if field {
        "Use in field"
    } else {
        "Use and place"
    };
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}

fn draw_tab(ui: &mut Ui, tokens: &Tokens, st: &mut State) -> Option<serde_json::Value> {
    ui.label(
        RichText::new("Draw your signature with the mouse, pen or finger.")
            .color(tokens.text_muted),
    );
    let (rect, resp) = ui.allocate_exact_size(vec2(PAD.0, PAD.1), Sense::drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tokens.checker[0]);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(tokens.border_width, tokens.border),
        egui::StrokeKind::Inside,
    );
    painter.hline(
        rect.x_range().shrink(16.0),
        rect.max.y - 30.0,
        Stroke::new(1.0, tokens.pdf_page_border),
    );
    let local = |p: Pos2| {
        [
            (p.x - rect.min.x).clamp(0.0, PAD.0),
            (p.y - rect.min.y).clamp(0.0, PAD.1),
        ]
    };
    if resp.drag_started()
        && let Some(p) = resp.interact_pointer_pos()
        && st.strokes.len() < fm_pdf::visual::MAX_STROKES
    {
        st.strokes.push(vec![local(p)]);
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some(last) = st.strokes.last_mut()
        && last.len() < fm_pdf::visual::MAX_POINTS
    {
        let pt = local(p);
        if last
            .last()
            .is_none_or(|q| (q[0] - pt[0]).abs() + (q[1] - pt[1]).abs() > 1.0)
        {
            last.push(pt);
        }
    }
    // Ink stays dark on the light pad in both themes, like paper.
    let ink = Stroke::new(2.0, tokens.sig_ink);
    for stroke in &st.strokes {
        let pts: Vec<Pos2> = stroke.iter().map(|p| rect.min + vec2(p[0], p[1])).collect();
        if let [dot] = pts.as_slice() {
            painter.circle_filled(*dot, 1.0, ink.color);
        } else {
            painter.add(egui::Shape::line(pts, ink));
        }
    }
    let mut out = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!st.strokes.is_empty(), egui::Button::new("Undo stroke"))
            .clicked()
        {
            st.strokes.pop();
        }
        if ui
            .add_enabled(!st.strokes.is_empty(), egui::Button::new("Clear"))
            .clicked()
        {
            st.strokes.clear();
        }
        if use_button(ui, !st.strokes.is_empty(), st.field.is_some()) {
            out = Some(json!({
                "kind": "drawn",
                "strokes": st.strokes,
                "width": PAD.0,
                "height": PAD.1,
            }));
        }
    });
    out
}

fn type_tab(ui: &mut Ui, tokens: &Tokens, st: &mut State) -> Option<serde_json::Value> {
    ui.label(
        RichText::new("Type your name; it is set in a handwriting font.").color(tokens.text_muted),
    );
    ui.add(
        egui::TextEdit::singleline(&mut st.name)
            .hint_text("Your name")
            .char_limit(fm_pdf::visual::MAX_TYPED)
            .desired_width(PAD.0),
    );
    let supported = fm_pdf::visual::typed_supported(&st.name);
    if let Err(c) = supported {
        ui.label(
            RichText::new(format!(
                "The handwriting font has no “{c}”. Draw your signature or use an image."
            ))
            .color(tokens.warning),
        );
    }
    let ok = !st.name.trim().is_empty() && supported.is_ok();
    let mut out = None;
    if use_button(ui, ok, st.field.is_some()) {
        out = Some(json!({ "kind": "typed", "name": st.name.trim() }));
    }
    out
}

fn image_tab(
    ui: &mut Ui,
    s: &Session,
    tokens: &Tokens,
    st: &mut State,
    actions: &mut Vec<Action>,
) -> Option<serde_json::Value> {
    ui.label(
        RichText::new("A PNG or JPEG of your signature, up to 8 MiB.").color(tokens.text_muted),
    );
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.image).hint_text("Image file").desired_width(PAD.0 - 90.0));
        if ui.button("Browse…").clicked() {
            actions.push((
                "picker.open",
                json!({ "purpose": IMAGE_PURPOSE, "kind": "file", "title": "Signature image", "start": st.image }),
            ));
        }
    });
    ui.checkbox(&mut st.remove_white, "Make a white background transparent");
    let _ = s;
    let mut out = None;
    if use_button(ui, !st.image.trim().is_empty(), st.field.is_some()) {
        out = Some(
            json!({ "kind": "image", "uri": st.image.trim(), "remove_white": st.remove_white }),
        );
    }
    out
}

fn saved_tab(
    ui: &mut Ui,
    s: &Session,
    tokens: &Tokens,
    st: &mut State,
    actions: &mut Vec<Action>,
) -> Option<serde_json::Value> {
    if s.pdf_signatures_dir.is_none() {
        ui.label(
            RichText::new("There is no folder to remember signatures in.").color(tokens.text_muted),
        );
        return None;
    }
    if s.pdf_saved_loading {
        ui.add(crate::spinner::Spinner::new());
        return None;
    }
    if s.pdf_saved.is_empty() {
        ui.label(
            RichText::new("No remembered signatures. After making one, choose Remember in the Fill & Sign bar.")
                .color(tokens.text_muted),
        );
        return None;
    }
    let mut out = None;
    for saved in &s.pdf_saved {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(160.0, 48.0), Sense::hover());
            ui.painter().rect_filled(rect, 3.0, tokens.checker[0]);
            match &saved.visual.kind {
                VisualKind::Drawn {
                    strokes,
                    width,
                    height,
                } => {
                    let k = (rect.width() / width.max(1.0)).min(rect.height() / height.max(1.0));
                    let ink = Stroke::new(1.2, tokens.sig_ink);
                    for stroke in strokes {
                        let pts: Vec<Pos2> = stroke
                            .iter()
                            .map(|p| rect.min + vec2(p[0], p[1]) * k)
                            .collect();
                        ui.painter().add(egui::Shape::line(pts, ink));
                    }
                }
                VisualKind::Typed { name } => {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        name,
                        egui::FontId::proportional(16.0),
                        tokens.sig_ink,
                    );
                }
                VisualKind::Image { .. } => {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Image",
                        egui::FontId::proportional(14.0),
                        tokens.sig_ink,
                    );
                }
            }
            if use_button(ui, true, st.field.is_some()) {
                out = Some(json!({ "kind": "saved", "id": saved.id }));
            }
            if ui.button("Forget").clicked() {
                actions.push(("pdf.sign.saved.delete", json!({ "id": saved.id })));
            }
        });
    }
    out
}

/// Returns true when signing was started (the dialog closes).
fn digital_tab(
    ui: &mut Ui,
    s: &Session,
    tokens: &Tokens,
    view: &PdfView,
    st: &mut State,
    actions: &mut Vec<Action>,
) -> bool {
    let Some(fill) = &view.fill else { return false };
    ui.label(
        RichText::new("Sign with a certificate file (.p12 or .pfx). The signed copy is saved next to the original.")
            .color(tokens.text_muted),
    );
    egui::Grid::new("pv-pdf-sign-digital").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
        ui.label("Certificate");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.cert).desired_width(240.0));
            if ui.button("Browse…").clicked() {
                actions.push((
                    "picker.open",
                    json!({ "purpose": CERT_PURPOSE, "kind": "file", "title": "Certificate file", "start": st.cert }),
                ));
            }
        });
        ui.end_row();
        ui.label("Password");
        ui.add(egui::TextEdit::singleline(&mut st.password).password(true).desired_width(240.0));
        ui.end_row();
        ui.label("Reason");
        ui.add(egui::TextEdit::singleline(&mut st.reason).char_limit(256).hint_text("Optional").desired_width(240.0));
        ui.end_row();
        ui.label("Location");
        ui.add(egui::TextEdit::singleline(&mut st.location).char_limit(256).hint_text("Optional").desired_width(240.0));
        ui.end_row();
        ui.label("Shows");
        ui.vertical(|ui| {
            ui.radio_value(&mut st.look, Look::NameDate, "My name and the date");
            ui.add_enabled_ui(fill.pending_visual.is_some(), |ui| {
                ui.radio_value(&mut st.look, Look::Visual, "My visual signature");
            });
            if st.field.is_none() {
                ui.radio_value(&mut st.look, Look::Invisible, "Nothing (invisible signature)");
            }
        });
        ui.end_row();
    });
    let empty_fields: Vec<&str> = fill
        .fields
        .iter()
        .filter(|f| matches!(f.kind, FieldKind::Signature { signed: false }))
        .map(|f| f.name.as_str())
        .collect();
    if st.field.is_none() && !empty_fields.is_empty() {
        crate::dialog_style::hint(
            ui,
            tokens,
            "Tip: click an empty signature field on the page to sign in that field.",
        );
    }
    if fill.signed_before {
        crate::dialog_style::hint(
            ui,
            tokens,
            "Existing signatures stay valid: the new signature is added as an update.",
        );
    }
    let ready = !st.cert.trim().is_empty() && !st.password.is_empty() && !fill.is_saving();
    let mut started = false;
    if ui
        .add_enabled(ready, egui::Button::new("Sign and save copy"))
        .clicked()
    {
        let appearance = match st.look {
            Look::NameDate => "name_date",
            Look::Visual => "visual",
            Look::Invisible => "none",
        };
        let mut params = json!({
            "cert": st.cert.trim(),
            "password": st.password,
            "appearance": appearance,
        });
        let mut extra = serde_json::Map::new();
        if !st.reason.trim().is_empty() {
            extra.insert("reason".into(), json!(st.reason.trim()));
        }
        if !st.location.trim().is_empty() {
            extra.insert("location".into(), json!(st.location.trim()));
        }
        match &st.field {
            Some(f) => {
                extra.insert("field".into(), json!(f));
            }
            None if st.look != Look::Invisible => {
                // A visible signature without a field: bottom right of the current page.
                if let Some(shown) = s.pdf_shown(st.pane)
                    && let Some(p) = shown.doc.info().pages.get(view.page)
                {
                    let (w, h) = (180.0f32.min(p.width - 16.0), 54.0f32.min(p.height - 16.0));
                    let x1 = p.width - 36.0f32.min(p.width / 10.0);
                    let y1 = p.height - 36.0f32.min(p.height / 10.0);
                    extra.insert("page".into(), json!(view.page));
                    extra.insert("rect".into(), json!([x1 - w, y1 - h, x1, y1]));
                }
            }
            None => {}
        }
        if let Some(obj) = params.as_object_mut() {
            obj.extend(extra);
        }
        actions.push(("pdf.sign.digital", params));
        st.password.clear();
        started = true;
    }
    started
}
