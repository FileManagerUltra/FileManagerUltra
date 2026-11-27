//! The bar above a Preview pane's content, the same for every kind of preview and for Edit
//! mode: what is shown on the left, then on the right the tools of this view and, at fixed
//! spots from the right edge, the common three: Wrap, Find and the Edit/Preview switch.
//! The common three are always drawn (greyed out where they do not apply) so they never
//! move; the view's own tools fold into a "View options" menu when the pane is narrow.

use egui::{Align, Layout, Ui, vec2};

use crate::theme::Tokens;
use crate::tool_icons::{self as icons, Icon};

/// State of the common tools.
pub(crate) struct Common<'a> {
    /// Edit mode: the switch shows Preview (eye); otherwise Edit (pencil).
    pub editing: bool,
    pub mode_enabled: bool,
    pub mode_tip: &'a str,
    pub find_enabled: bool,
    pub find_active: bool,
    pub find_tip: &'a str,
    pub wrap_enabled: bool,
    pub wrap_active: bool,
    pub wrap_tip: &'a str,
}

/// Which common tool was clicked this frame.
#[derive(Default)]
pub(crate) struct Clicks {
    pub mode: bool,
    pub find: bool,
    pub wrap: bool,
}

/// A view tool: an icon on the bar, or a labelled toggle inside the "View options" menu.
/// `name` is the accessible name (and the menu label), `tip` the tooltip.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tool(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    name: &str,
    tip: &str,
    enabled: bool,
    active: bool,
    menu: bool,
) -> egui::Response {
    if menu {
        let r = ui.add_enabled(enabled, egui::Button::selectable(active, name));
        if enabled {
            r.on_hover_text(tip)
        } else {
            r.on_disabled_hover_text(tip)
        }
    } else {
        icons::tool_button(ui, tokens, icon, name, tip, enabled, active)
    }
}

/// Width `draw` takes, laid out left to right without being shown.
fn measure(ui: &mut Ui, draw: &mut dyn FnMut(&mut Ui)) -> f32 {
    let rect = egui::Rect::from_min_size(ui.cursor().min, vec2(10_000.0, ui.available_height()));
    let mut sizing = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center))
            .sizing_pass()
            .invisible(),
    );
    draw(&mut sizing);
    sizing.min_rect().width()
}

/// Draws the bar. `context` fills the left side; `pinned` (never folded) and `tools`
/// (folded into the menu when narrow; the flag says it is drawing inside the menu) sit
/// left of the common three.
pub(crate) fn show(
    ui: &mut Ui,
    tokens: &Tokens,
    context: &mut dyn FnMut(&mut Ui),
    common: &Common<'_>,
    pinned: &mut dyn FnMut(&mut Ui),
    tools: &mut dyn FnMut(&mut Ui, bool),
) -> Clicks {
    let mut clicks = Clicks::default();
    // One row as tall as the icon buttons, so pickers, labels and icons line up.
    let row = ui.spacing().interact_size.y;
    ui.spacing_mut().interact_size.y = row.max(icons::BUTTON.y * tokens.zoom);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let context_w = measure(ui, &mut |ui| context(ui));
        let tools_w = measure(ui, &mut |ui| tools(ui, false));
        let pinned_w = measure(ui, &mut |ui| pinned(ui));
        let button = icons::BUTTON.x * tokens.zoom + ui.spacing().item_spacing.x;
        let common_w = 3.0 * button + 16.0;
        let fits = context_w + tools_w + pinned_w + common_w + 8.0 <= ui.available_width();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (mode_icon, mode_name) = if common.editing {
                (Icon::Preview, "Preview")
            } else {
                (Icon::Edit, "Edit")
            };
            clicks.mode = icons::tool_button(
                ui,
                tokens,
                mode_icon,
                mode_name,
                common.mode_tip,
                common.mode_enabled,
                false,
            )
            .clicked();
            clicks.find = icons::tool_button(
                ui,
                tokens,
                Icon::Find,
                "Find",
                common.find_tip,
                common.find_enabled,
                common.find_active,
            )
            .clicked();
            clicks.wrap = icons::tool_button(
                ui,
                tokens,
                Icon::Wrap,
                "Wrap",
                common.wrap_tip,
                common.wrap_enabled,
                common.wrap_active,
            )
            .clicked();
            if tools_w > 0.0 || pinned_w > 0.0 {
                icons::divider(ui, tokens);
            }
            let block = |ui: &mut Ui, width: f32, draw: &mut dyn FnMut(&mut Ui)| {
                ui.allocate_ui_with_layout(
                    vec2(width, ui.available_height()),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        draw(ui);
                    },
                );
            };
            if pinned_w > 0.0 {
                block(ui, pinned_w, &mut |ui| pinned(ui));
            }
            if tools_w > 0.0 {
                if fits {
                    block(ui, tools_w, &mut |ui| tools(ui, false));
                } else {
                    let menu = icons::button(ui, tokens, Icon::Menu, "View options", true, false);
                    egui::Popup::menu(&menu).show(|ui| tools(ui, true));
                }
            }
            // The left side gets what is left, clipped so it never runs under the tools.
            let rest = ui.available_rect_before_wrap();
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(rest)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let mut clip = ui.clip_rect();
                    clip.max.x = clip.max.x.min(rest.max.x - 4.0);
                    ui.set_clip_rect(clip);
                    context(ui);
                },
            );
        });
    });
    ui.spacing_mut().interact_size.y = row;
    clicks
}
