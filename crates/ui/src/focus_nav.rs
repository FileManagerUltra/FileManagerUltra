//! Keyboard focus helpers for dialogs: Tab boundaries, roving focus, visible focus.
//!
//! egui moves focus in the order widgets register: Tab gives it to the next widget drawn,
//! and from the last widget it wraps to the first widget of the whole app. To keep Tab inside
//! a dialog, its first and last stops take Tab over while focused ([`keys`] with
//! `Lock::tab`), and either handle it ([`focus_later`]) or hand it back to egui
//! ([`pass_tab`]). Both must happen *before* the focused widget registers, so widgets that
//! use this have ids known in advance.

use egui::{Context, Id, Key, Response, Ui};

use crate::theme::Tokens;

/// A Tab press, by direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Forward,
    Back,
}

/// Which keys a focused widget handles itself instead of egui's focus movement.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Lock {
    pub tab: bool,
    pub arrows: bool,
}

/// Keys pressed this frame for a focused widget (see [`keys`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Keys {
    pub tab: Option<Tab>,
    /// Up, Down, Left, Right, Home or End (only with `Lock::arrows`).
    pub arrow: Option<Key>,
}

pub(crate) fn has_focus(ctx: &Context, id: Id) -> bool {
    ctx.memory(|m| m.focused()) == Some(id)
}

/// Call before the widget `id` is added. While it has focus, egui stops moving focus for
/// the locked keys (from the next press on) and this reports them instead.
pub(crate) fn keys(ui: &Ui, id: Id, lock: Lock) -> Keys {
    if !has_focus(ui.ctx(), id) {
        return Keys::default();
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                tab: lock.tab,
                horizontal_arrows: lock.arrows,
                vertical_arrows: lock.arrows,
                ..Default::default()
            },
        );
    });
    ui.input(|i| {
        let plain = !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.command;
        let direction = if i.modifiers.shift {
            Tab::Back
        } else {
            Tab::Forward
        };
        let tab = (lock.tab && plain && i.key_pressed(Key::Tab)).then_some(direction);
        let arrow = if lock.arrows && plain && !i.modifiers.shift {
            [
                Key::ArrowUp,
                Key::ArrowDown,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::Home,
                Key::End,
            ]
            .into_iter()
            .find(|k| i.key_pressed(*k))
        } else {
            None
        };
        Keys { tab, arrow }
    })
}

/// Let egui continue its own Tab order from the focused widget. Call before that widget is
/// added, in the frame [`keys`] reported the press.
pub(crate) fn pass_tab(ui: &Ui, tab: Tab) {
    ui.memory_mut(|m| {
        m.move_focus(match tab {
            Tab::Forward => egui::FocusDirection::Next,
            Tab::Back => egui::FocusDirection::Previous,
        });
    });
}

fn later_id() -> Id {
    Id::NULL.with("focus-nav-later")
}

/// Give `id` the focus from the next frame on. A same-frame request would let a widget drawn
/// later in this frame handle the very key press that moved the focus.
pub(crate) fn focus_later(ctx: &Context, id: Id) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(later_id(), (id, pass)));
}

/// Whether a [`focus_later`] request for `id` is due.
pub(crate) fn wants_focus(ctx: &Context, id: Id) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(Id, u64)>(later_id()))
        .is_some_and(|(target, made)| target == id && made < pass)
}

/// Take a due [`focus_later`] request for this widget: focus it and scroll it into view.
pub(crate) fn claim(ui: &Ui, response: &Response) {
    if wants_focus(ui.ctx(), response.id) {
        response.request_focus();
        response.scroll_to_me(None);
        ui.data_mut(|d| d.remove::<(Id, u64)>(later_id()));
    }
}

fn mode_id() -> Id {
    Id::NULL.with("focus-nav-keyboard-mode")
}

fn last_focus_id() -> Id {
    Id::NULL.with("focus-nav-last-focus")
}

/// Call once per frame (before the dialog's widgets). Navigation keys turn keyboard mode on,
/// a pointer press turns it off: like the web's `:focus-visible`, the ring shows only for
/// keyboard users. Returns the mode.
pub(crate) fn track(ctx: &Context) -> bool {
    let (keyboard, pointer) = ctx.input(|i| {
        let keyboard = i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: Key::Tab
                        | Key::ArrowUp
                        | Key::ArrowDown
                        | Key::ArrowLeft
                        | Key::ArrowRight
                        | Key::Home
                        | Key::End
                        | Key::PageUp
                        | Key::PageDown,
                    pressed: true,
                    ..
                }
            )
        });
        let pointer = i
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. }));
        (keyboard, pointer)
    });
    let mut on = keyboard_mode(ctx);
    if pointer {
        on = false;
    }
    if keyboard {
        on = true;
    }
    ctx.data_mut(|d| d.insert_temp(mode_id(), on));
    on
}

pub(crate) fn keyboard_mode(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(mode_id())).unwrap_or(false)
}

/// The focused widget, if it got the focus this frame (by any means).
fn newly_focused(ctx: &Context) -> Option<Id> {
    let now = ctx.memory(|m| m.focused());
    let before = ctx
        .data(|d| d.get_temp::<Option<Id>>(last_focus_id()))
        .flatten();
    now.filter(|id| Some(*id) != before)
}

/// Call at the end of the dialog's frame (after [`scroll_to_focus`] calls).
pub(crate) fn end_frame(ctx: &Context) {
    let now = ctx.memory(|m| m.focused());
    ctx.data_mut(|d| d.insert_temp(last_focus_id(), now));
}

/// Inside a scroll area, after its contents: in keyboard mode, scroll the widget that just
/// got focus into view if it is part of these contents.
pub(crate) fn scroll_to_focus(ui: &Ui) {
    if !keyboard_mode(ui.ctx()) {
        return;
    }
    let Some(id) = newly_focused(ui.ctx()) else {
        return;
    };
    let Some(response) = ui.ctx().read_response(id) else {
        return;
    };
    if response.layer_id == ui.layer_id() && ui.min_rect().contains(response.rect.center()) {
        ui.scroll_to_rect(response.rect.expand(6.0), None);
    }
}

/// In keyboard mode, a ring around the focused widget of this layer. Text fields keep their
/// own focus style.
pub(crate) fn paint_focus_ring(ui: &Ui, tokens: &Tokens) {
    let ctx = ui.ctx();
    if !keyboard_mode(ctx) || ctx.text_edit_focused() {
        return;
    }
    let Some(response) = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
    else {
        return;
    };
    if response.layer_id != ui.layer_id() {
        return;
    }
    ui.painter().rect_stroke(
        response.rect.expand(2.0),
        tokens.corner_radius,
        egui::Stroke::new(2.0, tokens.accent),
        egui::StrokeKind::Outside,
    );
}

/// What a choice option shows.
pub(crate) enum Face<'a> {
    Text(&'a str),
    /// A colour swatch; the name is its accessible label and tooltip.
    Swatch(egui::Color32, &'a str),
}

/// Mutually exclusive options that form one Tab stop, like a radio group: only the selected
/// option takes focus, the arrow keys move the choice (and apply it, as a click does), and
/// Home/End jump to the ends. `id` must be unique and stable. Returns the newly chosen index.
pub(crate) fn choice_row(
    ui: &mut Ui,
    tokens: &Tokens,
    id: Id,
    label: Option<&str>,
    options: &[Face<'_>],
    selected: Option<usize>,
) -> Option<usize> {
    let mut picked = None;
    let count = options.len();
    if count == 0 {
        return None;
    }
    // With nothing selected, the first option is the Tab stop.
    let stop = selected.filter(|i| *i < count).unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        if let Some(text) = label {
            ui.label(text);
        }
        for (index, face) in options.iter().enumerate() {
            let option_id = id.with(index);
            if index == stop {
                let keys = keys(
                    ui,
                    option_id,
                    Lock {
                        arrows: true,
                        tab: false,
                    },
                );
                let next = match keys.arrow {
                    Some(Key::ArrowLeft | Key::ArrowUp) => Some((stop + count - 1) % count),
                    Some(Key::ArrowRight | Key::ArrowDown) => Some((stop + 1) % count),
                    Some(Key::Home) => Some(0),
                    Some(Key::End) => Some(count - 1),
                    _ => None,
                };
                if let Some(next) = next.filter(|n| *n != stop) {
                    picked = Some(next);
                    focus_later(ui.ctx(), id.with(next));
                }
            }
            let response = option(
                ui,
                tokens,
                option_id,
                face,
                selected == Some(index),
                index == stop,
            );
            claim(ui, &response);
            if response.clicked() && selected != Some(index) {
                picked = Some(index);
            }
        }
    });
    picked
}

fn option(
    ui: &mut Ui,
    tokens: &Tokens,
    id: Id,
    face: &Face<'_>,
    selected: bool,
    focusable: bool,
) -> Response {
    let sense = if focusable {
        egui::Sense::click()
    } else {
        egui::Sense::CLICK
    };
    match face {
        Face::Text(text) => {
            let spacing = ui.spacing();
            let (dot, gap) = (spacing.icon_width, spacing.icon_spacing);
            let galley = ui.painter().layout_no_wrap(
                (*text).to_string(),
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().text_color(),
            );
            let size = egui::vec2(
                dot + gap + galley.size().x,
                galley.size().y.max(dot).max(spacing.interact_size.y),
            );
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let response = ui.interact(rect, id, sense);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, *text)
            });
            if ui.is_rect_visible(rect) {
                let visuals = ui.style().interact(&response);
                let center = egui::pos2(rect.left() + dot / 2.0, rect.center().y);
                let radius = dot / 2.0;
                ui.painter().circle(
                    center,
                    radius + visuals.expansion,
                    visuals.bg_fill,
                    visuals.bg_stroke,
                );
                if selected {
                    ui.painter()
                        .circle_filled(center, radius / 2.2, visuals.fg_stroke.color);
                }
                ui.painter().galley(
                    egui::pos2(
                        rect.left() + dot + gap,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    visuals.text_color(),
                );
            }
            response
        }
        Face::Swatch(color, name) => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
            let response = ui.interact(rect, id, sense);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, *name)
            });
            if ui.is_rect_visible(rect) {
                ui.painter().rect_filled(rect.shrink(2.0), 4.0, *color);
                // White and black swatches must stay visible on either theme.
                ui.painter().rect_stroke(
                    rect.shrink(2.0),
                    4.0,
                    egui::Stroke::new(1.0, tokens.text_muted),
                    egui::StrokeKind::Inside,
                );
                if selected || response.hovered() {
                    ui.painter().rect_stroke(
                        rect,
                        5.0,
                        egui::Stroke::new(2.0, tokens.text),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            response.on_hover_text(*name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Role, Toggled};
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    const OPTIONS: [&str; 3] = ["System", "Dark", "Light"];

    fn harness() -> Harness<'static, usize> {
        Harness::builder()
            .with_size(egui::vec2(400.0, 200.0))
            .build_ui_state(
                |ui, chosen: &mut usize| {
                    let tokens = Tokens::dark();
                    track(ui.ctx());
                    let _ = ui.button("Before");
                    let faces = OPTIONS.map(Face::Text);
                    if let Some(next) = choice_row(
                        ui,
                        &tokens,
                        Id::NULL.with("theme"),
                        Some("Theme"),
                        &faces,
                        Some(*chosen),
                    ) {
                        *chosen = next;
                    }
                    let _ = ui.button("After");
                    paint_focus_ring(ui, &tokens);
                    end_frame(ui.ctx());
                },
                0,
            )
    }

    fn focused(h: &Harness<'_, usize>) -> String {
        h.get_all_by_role(Role::RadioButton)
            .chain(h.get_all_by_role(Role::Button))
            .find(|n| n.accesskit_node().is_focused())
            .and_then(|n| n.accesskit_node().label())
            .unwrap_or_default()
    }

    #[test]
    fn a_choice_row_is_one_tab_stop_with_arrow_keys() {
        let mut h = harness();
        h.get_by_label("Before").focus();
        h.run();
        h.key_press(Key::Tab);
        h.run();
        assert_eq!(focused(&h), "System", "Tab enters at the selected option");
        h.key_press(Key::ArrowRight);
        h.run();
        assert_eq!(*h.state(), 1);
        assert_eq!(focused(&h), "Dark", "focus follows the choice");
        assert_eq!(
            h.get_by_label("Dark").accesskit_node().toggled(),
            Some(Toggled::True)
        );
        h.key_press(Key::End);
        h.run();
        assert_eq!(*h.state(), 2);
        h.key_press(Key::ArrowRight);
        h.run();
        assert_eq!(*h.state(), 0, "arrows wrap around");
        h.key_press(Key::Tab);
        h.run();
        assert_eq!(focused(&h), "After", "one Tab leaves the row");
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
        h.run();
        assert_eq!(focused(&h), "System");
        h.get_by_label("Light").click();
        h.run();
        assert_eq!(*h.state(), 2, "clicking still chooses");
    }
}
