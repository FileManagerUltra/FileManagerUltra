//! Mermaid diagrams in the Markdown preview (change `markdown-mermaid`): drawing a diagram,
//! its placeholder or its reason. Colours come from `Tokens::mermaid_palette`. The engine draws the
//! bitmaps (`fm_engine::md_diagrams`); nothing here touches the renderer.

use std::sync::Arc;

use egui::{Id, Ui, vec2};
use fm_engine::md_diagrams::MdDiagramState;
use fm_preview::mermaid::{self, Diagram};

use crate::theme::Tokens;

/// Diagram states for one document, by code-block index. `None` in the map means "not
/// requested yet"; a block missing from the map is past the drawable limit.
pub(super) type DiagramStates = std::collections::HashMap<usize, Option<MdDiagramState>>;

/// Drawn size remembered per diagram, so its placeholder reserves the same space next time.
fn size_id(source: &str) -> Id {
    Id::NULL.with(("pv-mermaid-size", mermaid::source_key(source)))
}

/// A drawn diagram in a panel like a code block: a header row (returned, for the caller's
/// label and controls), then the picture at its natural size × preview zoom, never wider
/// than the column.
pub(super) fn diagram(
    ui: &mut Ui,
    tokens: &Tokens,
    source: &str,
    drawn: &Arc<Diagram>,
    texture: &egui::TextureHandle,
) -> egui::Rect {
    let zoom = super::content_scale(ui);
    let natural = vec2(drawn.natural.0, drawn.natural.1) * zoom;
    let frame = egui::Frame::NONE
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(tokens.border_width, tokens.border))
        .corner_radius(tokens.corner_radius)
        .inner_margin(12.0);
    let inner = frame.show(ui, |ui| {
        ui.set_width((ui.available_width()).max(1.0));
        let edge = tokens.font_size * 1.5;
        let (header, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), edge), egui::Sense::hover());
        ui.add_space(4.0);
        let room = ui.available_width().max(1.0);
        let size = natural * (room / natural.x.max(1.0)).min(1.0);
        let image = ui
            .vertical_centered(|ui| {
                ui.add(egui::Image::new((texture.id(), size)).sense(egui::Sense::hover()))
            })
            .inner;
        (header, image)
    });
    ui.ctx()
        .data_mut(|d| d.insert_temp(size_id(source), inner.response.rect.height()));
    let (header, response) = inner.inner;
    let kind = mermaid::diagram_kind(source).unwrap_or("diagram");
    let label = format!("Mermaid diagram, {kind}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    let description: String = source.chars().take(1000).collect();
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_description(description);
    });
    header
}

/// Space kept while a diagram is drawn: its last drawn height, else 160 points.
pub(super) fn placeholder(ui: &mut Ui, tokens: &Tokens, source: &str) {
    let height = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(size_id(source)))
        .unwrap_or(160.0 * super::content_scale(ui));
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width().max(1.0), height),
        egui::Sense::hover(),
    );
    ui.painter().rect(
        rect,
        tokens.corner_radius,
        tokens.panel,
        egui::Stroke::new(tokens.border_width, tokens.border),
        egui::StrokeKind::Inside,
    );
    let spinner = egui::Rect::from_center_size(rect.center(), vec2(24.0, 24.0));
    // Painted in place: adding a widget at a rect would move the layout cursor back.
    crate::spinner::Spinner::new().paint_at(ui, spinner);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Image, true, "Mermaid diagram, drawing")
    });
}

/// The line above a block that is shown as code instead of a diagram.
pub(super) fn reason(ui: &mut Ui, tokens: &Tokens, why: &str) {
    ui.label(
        egui::RichText::new(format!("Diagram not drawn: {why}"))
            .color(tokens.text_muted)
            .font(crate::markdown_fonts::font(
                ui.ctx(),
                tokens.font_size * 0.9,
                false,
            )),
    );
}
