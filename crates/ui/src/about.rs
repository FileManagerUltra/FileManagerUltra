//! The third-party licence section of the About dialog (`fm_preview::notices`).

use egui::{RichText, ScrollArea, Ui};

use crate::theme::Tokens;

/// One collapsible entry per bundled third-party part: what it is for, its licence, where
/// its source is, and the full licence text.
pub fn third_party(ui: &mut Ui, tokens: &Tokens) {
    ui.label(RichText::new("Third-party licences").strong());
    for (n, notice) in fm_preview::notices::NOTICES.iter().enumerate() {
        egui::CollapsingHeader::new(format!("{} ({})", notice.name, notice.licence))
            .id_salt(("about-notice", n))
            .show(ui, |ui| {
                ui.label(notice.purpose);
                ui.horizontal_wrapped(|ui| {
                    ui.label(tokens.muted("Source:"));
                    ui.add(egui::Label::new(notice.source).selectable(true));
                });
                ScrollArea::vertical()
                    .id_salt(("about-notice-text", n))
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(notice.text).monospace().small())
                                .selectable(true)
                                .wrap(),
                        );
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn about_lists_every_notice_and_opens_its_licence_text() {
        let tokens = Tokens::dark();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(520.0, 520.0))
            .build_ui(move |ui| {
                crate::theme::apply_to(ui.ctx(), &tokens);
                third_party(ui, &tokens);
            });
        harness.run();
        for notice in fm_preview::notices::NOTICES {
            harness.get_by_label_contains(notice.name);
        }
        harness.get_by_label_contains("Eclipse ELK").click();
        harness.run();
        harness.get_by_label_contains("Eclipse Public License - v 2.0");
        harness.get_by_label_contains("crates.io/crates/merman-elk-layered");
    }
}
