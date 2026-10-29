//! Document-only reading faces; the application's fonts are left unchanged.

use egui::{Context, FontData, FontFamily, FontId};

const REGULAR: &str = "markdown-regular";
const BOLD: &str = "markdown-bold";

pub fn ready(ctx: &Context) -> bool {
    ctx.fonts(|f| f.definitions().font_data.contains_key(BOLD))
}

pub fn install(ctx: &Context) {
    let id = egui::Id::NULL.with("markdown-fonts-installed");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let mut fonts = egui::FontDefinitions::default();
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (name, bytes) in [
        (
            REGULAR,
            include_bytes!("../assets/fonts/AtkinsonHyperlegible-Regular.ttf").as_slice(),
        ),
        (
            BOLD,
            include_bytes!("../assets/fonts/AtkinsonHyperlegible-Bold.ttf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), FontData::from_static(bytes).into());
        let mut family = vec![name.into()];
        family.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }
    ctx.set_fonts(fonts);
    ctx.data_mut(|d| d.insert_temp(id, true));
    ctx.request_repaint();
}

pub fn font(ctx: &Context, size: f32, bold: bool) -> FontId {
    if ready(ctx) {
        FontId::new(
            size,
            FontFamily::Name(if bold { BOLD } else { REGULAR }.into()),
        )
    } else {
        // Fonts installed after another widget laid out text activate next pass.
        FontId::proportional(size)
    }
}
