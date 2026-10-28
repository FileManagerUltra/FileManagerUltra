//! Shared dialog chrome and typography. See `docs/style-guide.md`.

use egui::{Context, FontFamily, FontId, RichText, Ui, Window};

use crate::theme::Tokens;

/// Center initially, but retain the user's dragged position afterwards.
pub fn window<'a>(ctx: &Context, tokens: &Tokens, title: &str, open: &'a mut bool) -> Window<'a> {
    Window::new(RichText::new(title).font(FontId::proportional(tokens.font_size + 2.0)))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .movable(true)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .frame(crate::dialogs::frame(tokens))
}

/// Apply only to the dialog's UI, never the application's global style.
pub fn apply(ui: &mut Ui, tokens: &Tokens) {
    let style = ui.style_mut();
    for font in style.text_styles.values_mut() {
        font.family = FontFamily::Proportional;
        font.size = tokens.font_size;
    }
    style.spacing.interact_size.y = 28.0;
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
}

/// Bound long forms while leaving room for the title and footer actions.
pub fn body<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) {
    let available = ui.ctx().content_rect().size();
    egui::ScrollArea::vertical()
        .max_height((available.y - 160.0).max(80.0))
        .show(ui, contents);
}

pub fn hint(ui: &mut Ui, tokens: &Tokens, text: impl Into<String>) {
    ui.label(
        RichText::new(text)
            .font(FontId::proportional(tokens.font_size - 1.0))
            .color(tokens.text_muted),
    );
}

pub fn section_heading(ui: &mut Ui, tokens: &Tokens, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(tokens.text));
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().line_segment(
            [rect.left_center(), rect.right_center()],
            egui::Stroke::new(tokens.border_width, tokens.border),
        );
    });
    ui.add_space(2.0);
}
