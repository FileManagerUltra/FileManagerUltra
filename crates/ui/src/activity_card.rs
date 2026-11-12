//! One card style for everything in the Activity panel (tips, messages, update notices,
//! minimised searches, file jobs), so each kind looks related and the important ones stand
//! out: a tinted card with a coloured edge, a short heading, the text, then its buttons.
//! Colours come from theme tokens only.

use egui::{Color32, FontId, Response, RichText, Ui};

use crate::theme::{Tokens, blend};

/// What a card is about; picks its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Tips, updates, searches and running jobs.
    Info,
    /// Failures and warnings.
    Error,
}

/// A card: `heading` in the tone colour, optional `meta` (e.g. "12 s ago") on the right,
/// then `body`.
pub fn card<R>(
    ui: &mut Ui,
    tokens: &Tokens,
    tone: Tone,
    heading: &str,
    meta: Option<&str>,
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let color = match tone {
        Tone::Info => tokens.accent,
        Tone::Error => tokens.error,
    };
    let output = egui::Frame::new()
        .fill(blend(tokens.panel, color, 0.10))
        .stroke(egui::Stroke::new(
            tokens.border_width.max(1.0),
            blend(tokens.border, color, 0.45),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin {
            left: 14,
            right: 10,
            top: 8,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(heading)
                        .font(FontId::proportional(tokens.font_size - 1.0))
                        .strong()
                        .color(if tone == Tone::Info {
                            tokens.accent_text
                        } else {
                            color
                        }),
                );
                if let Some(meta) = meta {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(meta)
                                .font(FontId::proportional(tokens.font_size - 2.0))
                                .color(tokens.text_muted),
                        );
                    });
                }
            });
            body(ui)
        });
    // The coloured edge, inside the rounded corners.
    let rect = output.response.rect;
    let edge = egui::Rect::from_min_max(
        rect.left_top() + egui::vec2(1.0, 6.0),
        egui::pos2(rect.left() + 4.0, rect.bottom() - 6.0),
    );
    ui.painter().rect_filled(edge, 2.0, color);
    output.inner
}

/// The main text of a card.
pub fn text(ui: &mut Ui, tokens: &Tokens, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(tokens.text)).wrap());
}

/// Secondary lines (details, progress text, per-item errors).
pub fn detail(ui: &mut Ui, tokens: &Tokens, text: &str, color: Option<Color32>) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .font(FontId::proportional(tokens.font_size - 1.0))
                .color(color.unwrap_or(tokens.text_muted)),
        )
        .wrap(),
    );
}

/// The card's main action: filled with the accent colour.
pub fn primary(ui: &mut Ui, tokens: &Tokens, label: &str, enabled: bool) -> Response {
    // The page background on the accent fill: dark on light blue (dark theme), white on
    // deep blue (light theme). Set on the text itself, as strong text ignores overrides.
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).strong().color(tokens.bg)).fill(tokens.accent),
    )
}

/// Other actions: a plain button.
pub fn secondary(ui: &mut Ui, label: &str) -> Response {
    ui.button(label)
}

/// Dismissing the card: quiet text at the right end of the button row.
pub fn quiet(ui: &mut Ui, tokens: &Tokens, label: &str) -> Response {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .font(FontId::proportional(tokens.font_size - 1.0))
                .color(tokens.text_muted),
        )
        .frame(false),
    )
}

/// A row of buttons. Use [`end`] inside it for the dismiss button at the right end.
pub fn buttons(ui: &mut Ui, row: impl FnOnce(&mut Ui)) {
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        row(ui);
    });
}

/// Inside [`buttons`]: lay out `f` from the right end of the row.
pub fn end(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), f);
}
