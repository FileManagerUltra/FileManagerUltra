//! Help ▸ About: who the app is (logo, name, version, build, licence), with a button to
//! Help ▸ Third-party Licences: the parts built into the app (`fm_preview::notices`), with
//! their licence texts one at a time.

use egui::{Align, Context, Id, Layout, RichText, ScrollArea, Stroke, Ui, vec2};

use crate::theme::Tokens;

/// Secondary text in the dialog's own (proportional) font; `Tokens::muted` would switch to
/// the UI font.
fn quiet(tokens: &Tokens, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(tokens.text_muted)
}

/// Width of the dialog's content, so opening a licence never makes it jump.
const WIDTH: f32 = 520.0;

fn logo(ctx: &Context) -> Option<egui::TextureHandle> {
    let id = Id::NULL.with("fm-about-logo");
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let (w, h, rgba) = fm_preview::image_prev::decode_original(fm_brand::ICON_PNG, 1 << 20).ok()?;
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    let texture = ctx.load_texture("fm-about-logo", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

/// The About window while `open` is true; the close button clears it. Its licences
/// button sets `licenses`.
pub fn dialog(ctx: &Context, tokens: &Tokens, open: &mut bool, licenses: &mut bool) {
    let version = format!(
        "Version {} · build {}",
        env!("CARGO_PKG_VERSION"),
        fm_engine::diag::build_commit()
    );
    dialog_with_version(ctx, tokens, open, licenses, &version);
}

/// [`dialog`] with the version line given, so snapshots do not change with every release.
pub fn dialog_with_version(
    ctx: &Context,
    tokens: &Tokens,
    open: &mut bool,
    licenses: &mut bool,
    version: &str,
) {
    if !*open {
        return;
    }
    crate::dialog_style::window(ctx, tokens, "About", open).show(ctx, |ui| {
        crate::dialog_style::apply(ui, tokens);
        ui.set_width(WIDTH);
        identity(ui, tokens, version);
        ui.add_space(tokens.padding * 2.0);
        ui.horizontal(|ui| {
            if ui
                .button("Third-party licences…")
                .on_hover_text("Fonts and other parts built into the app, with their licences")
                .clicked()
            {
                *licenses = true;
            }
        });
    });
}

/// The Third-party Licences window while `open` is true; the close button clears it.
pub fn licenses_dialog(ctx: &Context, tokens: &Tokens, open: &mut bool) {
    if !*open {
        return;
    }
    crate::dialog_style::window(ctx, tokens, "Third-party Licences", open)
        .id(Id::NULL.with("fm-licenses-window"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.set_width(WIDTH);
            third_party(ui, tokens);
        });
}

fn identity(ui: &mut Ui, tokens: &Tokens, version: &str) {
    ui.horizontal(|ui| {
        if let Some(texture) = logo(ui.ctx()) {
            ui.add(egui::Image::new(&texture).fit_to_exact_size(vec2(72.0, 72.0)));
        }
        ui.add_space(tokens.padding);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(
                RichText::new(fm_brand::APP_NAME)
                    .size(tokens.font_size * 1.6)
                    .strong(),
            );
            ui.label(quiet(tokens, version));
            if fm_brand::ORG_NAME != fm_brand::APP_NAME {
                ui.label(quiet(tokens, fm_brand::ORG_NAME));
            }
            ui.label("Licensed under the Apache License 2.0");
        });
    });
}

fn open_id() -> Id {
    Id::NULL.with("fm-about-open-notice")
}

/// The third-party table: one row per part (name and purpose, licence, a button that shows
/// its licence text below the table).
pub fn third_party(ui: &mut Ui, tokens: &Tokens) {
    let notices = fm_preview::notices::NOTICES;
    let mut shown: Option<usize> = ui.data(|d| d.get_temp(open_id())).flatten();
    ui.label(quiet(
        tokens,
        "Parts built into the app that come with their own licence.",
    ));
    ui.add_space(4.0);
    // A fixed height, so opening a licence never moves the row that was clicked.
    let list_height = 340.0;
    ScrollArea::vertical()
        .id_salt("fm-licenses-list")
        .max_height(list_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Frame::NONE
                .stroke(Stroke::new(tokens.border_width, tokens.border))
                .corner_radius(tokens.corner_radius)
                .show(ui, |ui| {
                    for (n, notice) in notices.iter().enumerate() {
                        if n > 0 {
                            let y = ui.cursor().top();
                            let x = ui.max_rect().x_range();
                            ui.painter().hline(
                                x,
                                y,
                                Stroke::new(tokens.border_width, tokens.border),
                            );
                        }
                        egui::Frame::NONE.inner_margin(10.0).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.set_width((ui.available_width() - 240.0).max(120.0));
                                    ui.add(
                                        egui::Label::new(RichText::new(notice.name).strong())
                                            .wrap(),
                                    );
                                    ui.add(egui::Label::new(quiet(tokens, notice.purpose)).wrap());
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let open = shown == Some(n);
                                    let label = if open { "Hide licence" } else { "View licence" };
                                    if ui
                                        .add(egui::Button::new(label).selected(open))
                                        .on_hover_text(format!("{} licence text", notice.licence))
                                        .clicked()
                                    {
                                        shown = if open { None } else { Some(n) };
                                    }
                                    ui.label(RichText::new(notice.licence).monospace());
                                });
                            });
                        });
                    }
                });
        });
    if let Some(notice) = shown.and_then(|n| notices.get(n)) {
        ui.add_space(tokens.padding);
        licence_text(ui, tokens, notice);
    }
    ui.data_mut(|d| d.insert_temp(open_id(), shown));
}

fn licence_text(ui: &mut Ui, tokens: &Tokens, notice: &fm_preview::notices::Notice) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add(
            egui::Label::new(
                RichText::new(format!("{}: {}", notice.licence, notice.name)).strong(),
            )
            .wrap(),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(quiet(tokens, "Source code:"));
            ui.add(egui::Label::new(notice.source).selectable(true));
        });
    });
    egui::Frame::NONE
        .fill(tokens.panel)
        .stroke(Stroke::new(tokens.border_width, tokens.border))
        .corner_radius(tokens.corner_radius)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("fm-about-licence-text")
                .max_height(260.0)
                .min_scrolled_height(260.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(notice.text)
                                .monospace()
                                .size(tokens.font_size * 0.85),
                        )
                        .selectable(true)
                        .wrap(),
                    );
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn about_lists_every_notice_and_shows_one_licence_at_a_time() {
        let tokens = Tokens::dark();
        let mut harness = Harness::builder()
            .with_size(vec2(640.0, 760.0))
            .build_ui(move |ui| {
                crate::theme::apply_to(ui.ctx(), &tokens);
                third_party(ui, &tokens);
            });
        harness.run();
        for notice in fm_preview::notices::NOTICES {
            harness.get_by_label_contains(notice.name);
        }
        assert!(
            harness
                .query_by_label_contains("Eclipse Public License - v 2.0")
                .is_none()
        );
        let buttons = harness.get_all_by_label("View licence").count();
        assert_eq!(buttons, fm_preview::notices::NOTICES.len());
        let elk = fm_preview::notices::NOTICES
            .iter()
            .position(|n| n.licence == "EPL-2.0")
            .expect("ELK notice");
        harness
            .get_all_by_label("View licence")
            .nth(elk)
            .expect("ELK row")
            .scroll_to_me();
        harness.run();
        harness
            .get_all_by_label("View licence")
            .nth(elk)
            .expect("ELK row")
            .click();
        harness.run();
        harness.get_by_label_contains("Eclipse Public License - v 2.0");
        harness.get_by_label_contains("crates.io/crates/merman-elk-layered");
        harness.get_by_label("Hide licence").click();
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Eclipse Public License - v 2.0")
                .is_none()
        );
    }
}
