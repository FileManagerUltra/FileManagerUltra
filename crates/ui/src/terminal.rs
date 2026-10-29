//! Terminal pane body: paints the engine's screen snapshot and turns key, paste, wheel
//! and mouse input into `term.*` commands. No pty access here (capability `terminal-pane`).

use egui::{
    Align2, Color32, Event, EventFilter, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui, vec2,
};
use fm_engine::Session;
use fm_engine::term_cmds::{TermSelection, exit_code};
use fm_layout::PaneId;
use fm_term::{MouseAction, MouseButton, MouseEvent, Snapshot};

use crate::theme::Tokens;

const PAD: f32 = 4.0;
pub const MENU_PASTE_REQUESTED: &str = "fm-terminal-menu-paste-requested";

pub fn request_paste(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.insert_persisted(Id::NULL.with(MENU_PASTE_REQUESTED), true);
    });
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn mod_param(m: Modifiers) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl)
}

fn csi_letter(letter: char, m: Modifiers, app_cursor: bool) -> Vec<u8> {
    if mod_param(m) > 1 {
        format!("\x1b[1;{}{letter}", mod_param(m)).into_bytes()
    } else if app_cursor {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn csi_tilde(code: u8, m: Modifiers) -> Vec<u8> {
    if mod_param(m) > 1 {
        format!("\x1b[{code};{}~", mod_param(m)).into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

/// The bytes a key press sends to the shell, or `None` when the key is not for the
/// terminal: plain printable keys (they arrive as text events) and `Ctrl+Shift+…` chords
/// (app shortcuts such as copy/paste and pane swap).
pub fn key_bytes(key: Key, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    if m.ctrl && m.shift {
        return None;
    }
    let plain = Modifiers { shift: false, ..m };
    let bytes = match key {
        Key::Enter => b"\r".to_vec(),
        Key::Backspace => vec![if m.ctrl { 0x08 } else { 0x7f }],
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => vec![0x1b],
        Key::ArrowUp => csi_letter('A', m, app_cursor),
        Key::ArrowDown => csi_letter('B', m, app_cursor),
        Key::ArrowRight => csi_letter('C', m, app_cursor),
        Key::ArrowLeft => csi_letter('D', m, app_cursor),
        Key::Home => csi_letter('H', m, app_cursor),
        Key::End => csi_letter('F', m, app_cursor),
        Key::Insert => csi_tilde(2, m),
        Key::Delete => csi_tilde(3, m),
        Key::PageUp => csi_tilde(5, m),
        Key::PageDown => csi_tilde(6, m),
        Key::F1 | Key::F2 | Key::F3 | Key::F4 => {
            let letter = match key {
                Key::F1 => 'P',
                Key::F2 => 'Q',
                Key::F3 => 'R',
                _ => 'S',
            };
            if mod_param(m) > 1 {
                format!("\x1b[1;{}{letter}", mod_param(m)).into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        Key::F5 => csi_tilde(15, m),
        Key::F6 => csi_tilde(17, m),
        Key::F7 => csi_tilde(18, m),
        Key::F8 => csi_tilde(19, m),
        Key::F9 => csi_tilde(20, m),
        Key::F10 => csi_tilde(21, m),
        Key::F11 => csi_tilde(23, m),
        Key::F12 => csi_tilde(24, m),
        Key::Space if m.ctrl => vec![0],
        _ if m.ctrl && !m.alt => vec![ctrl_byte(key)?],
        _ if m.alt && !m.ctrl => {
            let c = key_char(key)?;
            let c = if m.shift { c.to_ascii_uppercase() } else { c };
            vec![0x1b, c as u8]
        }
        _ => return None,
    };
    let _ = plain;
    Some(bytes)
}

fn key_char(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_ascii_alphanumeric()).then(|| c.to_ascii_lowercase())
}

fn ctrl_byte(key: Key) -> Option<u8> {
    match key {
        Key::OpenBracket => Some(0x1b),
        Key::Backslash => Some(0x1c),
        Key::CloseBracket => Some(0x1d),
        _ => {
            let c = key_char(key)?;
            c.is_ascii_lowercase().then(|| c as u8 - b'a' + 1)
        }
    }
}

struct Metrics {
    cw: f32,
    ch: f32,
}

fn metrics(ui: &Ui, tokens: &Tokens) -> Metrics {
    let galley = ui
        .painter()
        .layout_no_wrap("M".to_string(), tokens.term_font(), tokens.text);
    Metrics {
        cw: galley.size().x.max(1.0),
        ch: galley.size().y.max(1.0),
    }
}

fn cell_at(rect: Rect, m: &Metrics, snap: &Snapshot, pos: Pos2) -> (u16, u16) {
    let col = ((pos.x - rect.left() - PAD) / m.cw).floor().max(0.0) as u16;
    let row = ((pos.y - rect.top() - PAD) / m.ch).floor().max(0.0) as u16;
    (
        row.min(snap.rows.saturating_sub(1)),
        col.min(snap.cols.saturating_sub(1)),
    )
}

/// Render the pane's terminal and forward input. Call only for `PaneKind::Terminal`.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens, focused: bool) {
    let size = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let id = Id::NULL.with(("fm-terminal", pane));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let m = metrics(ui, tokens);

    let cols = ((rect.width() - 2.0 * PAD) / m.cw).floor().max(2.0) as u16;
    let rows = ((rect.height() - 2.0 * PAD) / m.ch).floor().max(2.0) as u16;
    // Restored workspaces carry terminal panes without a shell; start it on first render.
    if s.terminal(pane).is_none()
        && let Err(err) = s.ensure_terminal(pane)
    {
        s.notice = Some(err.to_string());
    }
    s.terminal_resize(pane, rows, cols);

    let Some(mut snap) = s.terminal_snapshot(pane) else {
        ui.painter().rect_filled(rect, 0.0, tokens.term_bg);
        return;
    };

    let exited = exit_code(s, pane);
    if focused && exited.is_none() && s.pending_kind.is_none() {
        if response.clicked() || !ui.ctx().text_edit_focused() {
            response.request_focus();
        }
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        forward_input(ui, s, pane, &snap);
        if let Some(updated) = s.terminal_snapshot(pane) {
            snap = updated;
        }
    }
    if exited.is_none() && s.pending_kind.is_none() {
        handle_pointer(ui, s, pane, &response, rect, &m, &snap);
        if let Some(updated) = s.terminal_snapshot(pane) {
            snap = updated;
        }
    }
    paint(
        ui,
        s,
        pane,
        tokens,
        rect,
        &m,
        &snap,
        focused && exited.is_none(),
    );

    if let Some(code) = exited {
        banner(ui, s, pane, tokens, rect, code);
    }
}

fn forward_input(ui: &Ui, s: &mut Session, pane: PaneId, snap: &Snapshot) {
    let (events, mods) = ui.input(|i| (i.events.clone(), i.modifiers));
    let mut data = String::new();
    let mut scroll = 0i32;
    let flush = |s: &mut Session, data: &mut String| {
        if !data.is_empty() {
            s.dispatch_or_notice(
                "term.input",
                &serde_json::json!({ "pane": pane, "data": std::mem::take(data) }),
            );
        }
    };
    for ev in events {
        match ev {
            Event::Text(t) if !mods.alt && !mods.ctrl => data.push_str(&t),
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if modifiers.shift
                    && (modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
                    && !modifiers.alt
                    && key == Key::C
                {
                    flush(s, &mut data);
                    s.dispatch_or_notice("term.copy", &serde_json::Value::Null);
                } else if modifiers.shift
                    && (modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
                    && !modifiers.alt
                    && key == Key::V
                {
                    flush(s, &mut data);
                    request_paste(ui.ctx());
                } else if modifiers.shift
                    && !modifiers.ctrl
                    && matches!(key, Key::PageUp | Key::PageDown)
                {
                    scroll += if key == Key::PageUp {
                        i32::from(snap.rows) - 1
                    } else {
                        1 - i32::from(snap.rows)
                    };
                } else if let Some(bytes) = key_bytes(key, modifiers, snap.application_cursor) {
                    data.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            // egui-winit turns Ctrl/Cmd+C/X/V into these events and drops the key press.
            Event::Copy if cfg!(target_os = "macos") => {
                flush(s, &mut data);
                s.dispatch_or_notice("term.copy", &serde_json::Value::Null);
            }
            Event::Copy => data.push('\u{3}'),
            Event::Cut => data.push('\u{18}'),
            Event::Paste(text) => {
                let menu_requested = ui.ctx().data_mut(|data| {
                    let id = Id::NULL.with(MENU_PASTE_REQUESTED);
                    let requested = data.get_persisted::<bool>(id).unwrap_or(false);
                    if requested {
                        data.insert_persisted(id, false);
                    }
                    requested
                });
                if cfg!(target_os = "macos") || menu_requested {
                    flush(s, &mut data);
                    s.dispatch_or_notice(
                        "term.paste",
                        &serde_json::json!({ "pane": pane, "text": text }),
                    );
                } else {
                    data.push('\u{16}');
                }
            }
            _ => {}
        }
    }
    flush(s, &mut data);
    if scroll != 0 {
        s.terminal_scroll(pane, scroll);
    }
}

#[derive(Clone, Default)]
struct MouseState {
    pressed: [bool; 3],
    last_cell: Option<(u16, u16)>,
    wheel: f32,
}

fn mouse_button(button: egui::PointerButton) -> Option<(usize, MouseButton)> {
    match button {
        egui::PointerButton::Primary => Some((0, MouseButton::Left)),
        egui::PointerButton::Middle => Some((1, MouseButton::Middle)),
        egui::PointerButton::Secondary => Some((2, MouseButton::Right)),
        _ => None,
    }
}

impl MouseState {
    fn held(&self) -> Option<MouseButton> {
        [MouseButton::Left, MouseButton::Middle, MouseButton::Right]
            .into_iter()
            .zip(self.pressed)
            .find_map(|(button, held)| held.then_some(button))
    }

    fn event(
        &mut self,
        event: &Event,
        snap: &Snapshot,
        cell: Option<(u16, u16)>,
        inside: bool,
        modifiers: Modifiers,
        cell_height: f32,
    ) -> Vec<MouseEvent> {
        if !snap.mouse.enabled() {
            *self = Self::default();
            return Vec::new();
        }
        let Some((row, col)) = cell.or(self.last_cell) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        let mut mods = modifiers;
        match event {
            Event::PointerButton {
                button,
                pressed,
                modifiers,
                ..
            } => {
                mods = *modifiers;
                let Some((index, button)) = mouse_button(*button) else {
                    return Vec::new();
                };
                let Some(held) = self.pressed.get_mut(index) else {
                    return Vec::new();
                };
                if *pressed && inside && !mods.shift {
                    *held = true;
                    actions.push(MouseAction::Press(button));
                    self.last_cell = Some((row, col));
                } else if !pressed && *held {
                    // Finish an application drag even outside the pane or after Shift.
                    *held = false;
                    actions.push(MouseAction::Release(button));
                    self.last_cell = Some((row, col));
                }
            }
            Event::PointerMoved(_) if !mods.shift && (inside || self.held().is_some()) => {
                if self.last_cell != Some((row, col)) {
                    self.last_cell = Some((row, col));
                    actions.push(MouseAction::Move(self.held()));
                }
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } if inside && !modifiers.shift => {
                mods = *modifiers;
                let lines = match unit {
                    egui::MouseWheelUnit::Point => delta.y / cell_height,
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * f32::from(snap.rows),
                };
                self.wheel = (self.wheel + lines).clamp(-32.0, 32.0);
                let steps = self.wheel.trunc() as i32;
                self.wheel -= steps as f32;
                let action = if steps > 0 {
                    MouseAction::ScrollUp
                } else {
                    MouseAction::ScrollDown
                };
                actions.extend(std::iter::repeat_n(action, steps.unsigned_abs() as usize));
            }
            Event::PointerGone => {
                for (held, button) in self.pressed.iter_mut().zip([
                    MouseButton::Left,
                    MouseButton::Middle,
                    MouseButton::Right,
                ]) {
                    if *held {
                        actions.push(MouseAction::Release(button));
                        *held = false;
                    }
                }
                self.last_cell = None;
            }
            _ => {}
        }
        actions
            .into_iter()
            .map(|action| MouseEvent {
                row,
                col,
                action,
                shift: mods.shift,
                alt: mods.alt,
                ctrl: mods.ctrl,
            })
            .collect()
    }
}

fn handle_pointer(
    ui: &Ui,
    s: &mut Session,
    pane: PaneId,
    response: &egui::Response,
    rect: Rect,
    m: &Metrics,
    snap: &Snapshot,
) {
    let state_id = response.id.with("mouse");
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<MouseState>(state_id).unwrap_or_default());
    let (events, mods, pointer) =
        ui.input(|i| (i.events.clone(), i.modifiers, i.pointer.hover_pos()));
    for event in events {
        let pos = match &event {
            Event::PointerButton { pos, .. } | Event::PointerMoved(pos) => Some(*pos),
            _ => pointer,
        };
        let inside = response.hovered() && pos.is_some_and(|p| rect.contains(p));
        let cell = pos.map(|p| cell_at(rect, m, snap, p));
        for event in state.event(&event, snap, cell, inside, mods, m.ch) {
            if matches!(
                event.action,
                MouseAction::Press(_) | MouseAction::ScrollUp | MouseAction::ScrollDown
            ) {
                s.terminal_select(None);
            }
            let (action, button) = match event.action {
                MouseAction::Press(b) => ("press", Some(b)),
                MouseAction::Release(b) => ("release", Some(b)),
                MouseAction::Move(b) => ("move", b),
                MouseAction::ScrollUp => ("up", None),
                MouseAction::ScrollDown => ("down", None),
            };
            let button = button.map(|b| match b {
                MouseButton::Left => "left",
                MouseButton::Middle => "middle",
                MouseButton::Right => "right",
            });
            let mut params = serde_json::json!({
                "pane": pane, "row": event.row, "col": event.col, "action": action,
                "shift": event.shift, "alt": event.alt, "ctrl": event.ctrl,
            });
            if let Some(button) = button {
                params["button"] = button.into();
            }
            s.dispatch_or_notice("term.mouse", &params);
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(state_id, state));
    if snap.mouse.enabled() && !mods.shift {
        return;
    }
    if response.hovered() {
        let wheel = ui.input(|i| i.smooth_scroll_delta.y);
        let lines = (wheel / m.ch).round() as i32;
        if lines != 0 {
            s.terminal_scroll(pane, lines);
        }
    }
    let Some(pos) = response.interact_pointer_pos() else {
        if response.clicked() {
            s.terminal_select(None);
        }
        return;
    };
    let mut snap = s.terminal_snapshot(pane).unwrap_or_else(|| snap.clone());
    if response.dragged() && (pos.y < rect.top() + m.ch || pos.y > rect.bottom() - m.ch) {
        let id = response.id.with("selection-scroll");
        let now = ui.input(|i| i.time);
        let last = ui
            .ctx()
            .data(|d| d.get_temp::<f64>(id))
            .unwrap_or(now - 1.0);
        if now - last >= 0.05 {
            s.terminal_scroll(pane, if pos.y < rect.center().y { 1 } else { -1 });
            ui.ctx().data_mut(|d| d.insert_temp(id, now));
            if let Some(updated) = s.terminal_snapshot(pane) {
                snap = updated;
            }
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    let cell = snap.buffer_cell(cell_at(rect, m, &snap, pos));
    if response.drag_started() {
        let anchor = ui
            .input(|i| i.pointer.press_origin())
            .map_or(cell, |pos| snap.buffer_cell(cell_at(rect, m, &snap, pos)));
        s.terminal_select(Some(TermSelection {
            pane,
            anchor,
            head: cell,
        }));
    } else if response.dragged()
        && let Some(sel) = s.terminal_selection.filter(|x| x.pane == pane)
    {
        s.terminal_select(Some(TermSelection { head: cell, ..sel }));
    } else if response.clicked() {
        if mods.shift
            && let Some(sel) = s.terminal_selection.filter(|sel| sel.pane == pane)
        {
            s.terminal_select(Some(TermSelection { head: cell, ..sel }));
        } else {
            s.terminal_select(None);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    rect: Rect,
    m: &Metrics,
    snap: &Snapshot,
    focused: bool,
) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, tokens.term_bg);
    let origin = rect.min + vec2(PAD, PAD);
    let font = tokens.term_font();
    let selection = s.terminal_selection.filter(|x| x.pane == pane).map(|x| {
        if x.anchor <= x.head {
            (x.anchor, x.head)
        } else {
            (x.head, x.anchor)
        }
    });

    for row in 0..snap.rows {
        let y = origin.y + f32::from(row) * m.ch;
        let mut run = String::new();
        let mut run_start = 0u16;
        let mut run_style: Option<(Color32, bool, bool)> = None;
        let flush = |text: &mut String, start: u16, style: Option<(Color32, bool, bool)>| {
            if let Some((fg, bold, underline)) = style
                && !text.trim().is_empty()
            {
                let pos = Pos2::new(origin.x + f32::from(start) * m.cw, y);
                painter.text(pos, Align2::LEFT_TOP, text.as_str(), font.clone(), fg);
                if bold {
                    painter.text(
                        pos + vec2(0.6, 0.0),
                        Align2::LEFT_TOP,
                        text.as_str(),
                        font.clone(),
                        fg,
                    );
                }
                if underline {
                    let w = text.chars().count() as f32 * m.cw;
                    let uy = y + m.ch - 1.5;
                    painter.line_segment(
                        [Pos2::new(pos.x, uy), Pos2::new(pos.x + w, uy)],
                        Stroke::new(1.0, fg),
                    );
                }
            }
            text.clear();
        };
        for col in 0..snap.cols {
            let Some(cell) = snap.cell(row, col) else {
                continue;
            };
            let (mut fg, mut bg) = (
                tokens.term_color(cell.fg, true),
                tokens.term_color(cell.bg, false),
            );
            if cell.inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.dim {
                fg = fg.gamma_multiply(0.6);
            }
            let x = origin.x + f32::from(col) * m.cw;
            let cell_rect = Rect::from_min_size(Pos2::new(x, y), vec2(m.cw, m.ch));
            if bg != tokens.term_bg {
                painter.rect_filled(cell_rect, 0.0, bg);
            }
            let buffer_cell = snap.buffer_cell((row, col));
            if selection.is_some_and(|(a, b)| buffer_cell >= a && buffer_cell <= b) {
                painter.rect_filled(cell_rect, 0.0, tokens.term_selection);
            }
            if snap.cursor == Some((row, col)) {
                if focused {
                    painter.rect_filled(cell_rect, 0.0, tokens.term_cursor.gamma_multiply(0.85));
                } else {
                    painter.rect_stroke(
                        cell_rect,
                        0.0,
                        Stroke::new(1.0, tokens.term_cursor),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            if cell.wide_continuation {
                continue;
            }
            let style = (fg, cell.bold, cell.underline);
            if run_style != Some(style) {
                flush(&mut run, run_start, run_style);
                run_style = Some(style);
                run_start = col;
            }
            if cell.text.is_empty() {
                run.push(' ');
            } else if let Some(arms) = box_arms(&cell.text) {
                paint_box(&painter, cell_rect, arms, fg);
                run.push(' ');
            } else {
                run.push_str(&cell.text);
            }
        }
        flush(&mut run, run_start, run_style);
    }

    if snap.scrollback_offset > 0 {
        let label = format!("scrollback -{} (type to return)", snap.scrollback_offset);
        painter.text(
            rect.right_top() + vec2(-PAD - 2.0, PAD),
            Align2::RIGHT_TOP,
            label,
            tokens.term_font(),
            tokens.warning,
        );
    }
}

fn banner(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens, rect: Rect, code: i32) {
    let h = tokens.row_height + 2.0 * tokens.padding;
    let strip = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - h), rect.max);
    ui.painter().rect_filled(strip, 0.0, tokens.panel);
    let mut restart = false;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(strip.shrink(tokens.padding)),
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(tokens.muted(format!("process exited (code {code})")));
                if ui.button("Restart").clicked() {
                    restart = true;
                }
            });
        },
    );
    if restart {
        s.dispatch_or_notice("term.restart", &serde_json::json!({ "pane": pane }));
    }
}

/// Which arms (up, down, left, right) a light box-drawing character has.
fn box_arms(text: &str) -> Option<[bool; 4]> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(match c {
        '─' => [false, false, true, true],
        '│' => [true, true, false, false],
        '┌' => [false, true, false, true],
        '┐' => [false, true, true, false],
        '└' => [true, false, false, true],
        '┘' => [true, false, true, false],
        '├' => [true, true, false, true],
        '┤' => [true, true, true, false],
        '┬' => [false, true, true, true],
        '┴' => [true, false, true, true],
        '┼' => [true, true, true, true],
        _ => return None,
    })
}

/// Paint a box-drawing cell edge to edge so borders join without font gaps.
fn paint_box(
    painter: &egui::Painter,
    rect: Rect,
    [up, down, left, right]: [bool; 4],
    color: Color32,
) {
    let c = rect.center().round();
    let stroke = Stroke::new(1.0, color);
    let half = 0.5;
    if up {
        painter.line_segment(
            [
                Pos2::new(c.x + half, rect.top()),
                Pos2::new(c.x + half, c.y + half),
            ],
            stroke,
        );
    }
    if down {
        painter.line_segment(
            [
                Pos2::new(c.x + half, c.y + half),
                Pos2::new(c.x + half, rect.bottom()),
            ],
            stroke,
        );
    }
    if left {
        painter.line_segment(
            [
                Pos2::new(rect.left(), c.y + half),
                Pos2::new(c.x + half, c.y + half),
            ],
            stroke,
        );
    }
    if right {
        painter.line_segment(
            [
                Pos2::new(c.x + half, c.y + half),
                Pos2::new(rect.right(), c.y + half),
            ],
            stroke,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse_snapshot() -> Snapshot {
        let mut emulator = fm_term::Emulator::new(24, 80);
        emulator.feed(b"\x1b[?1002h\x1b[?1006h");
        emulator.snapshot()
    }

    fn button(pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton {
            pos: egui::pos2(20.0, 20.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn wheel(lines: f32, modifiers: Modifiers) -> Event {
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers,
        }
    }

    #[test]
    fn application_click_drag_release_and_shift_override() {
        let snap = mouse_snapshot();
        let mut state = MouseState::default();
        let cell = Some((2, 4));
        let press = state.event(
            &button(true, Modifiers::NONE),
            &snap,
            cell,
            true,
            Modifiers::NONE,
            16.0,
        );
        assert_eq!(press.len(), 1);
        assert_eq!(press[0].action, MouseAction::Press(MouseButton::Left));
        let moved = Event::PointerMoved(egui::pos2(30.0, 20.0));
        assert!(
            state
                .event(&moved, &snap, cell, true, Modifiers::NONE, 16.0)
                .is_empty()
        );
        let drag = state.event(&moved, &snap, Some((2, 5)), true, Modifiers::NONE, 16.0);
        assert_eq!(drag[0].action, MouseAction::Move(Some(MouseButton::Left)));
        let release = state.event(
            &button(false, Modifiers::SHIFT),
            &snap,
            Some((2, 79)),
            false,
            Modifiers::SHIFT,
            16.0,
        );
        assert_eq!(release[0].action, MouseAction::Release(MouseButton::Left));
        assert!(state.held().is_none());
        assert!(
            state
                .event(
                    &button(true, Modifiers::SHIFT),
                    &snap,
                    cell,
                    true,
                    Modifiers::SHIFT,
                    16.0
                )
                .is_empty()
        );
        assert!(
            state.held().is_none(),
            "Shift selection is not captured by the app"
        );
        assert!(
            state
                .event(
                    &button(false, Modifiers::NONE),
                    &snap,
                    cell,
                    true,
                    Modifiers::NONE,
                    16.0
                )
                .is_empty()
        );
        assert!(
            state
                .event(
                    &button(true, Modifiers::NONE),
                    &snap,
                    cell,
                    false,
                    Modifiers::NONE,
                    16.0
                )
                .is_empty()
        );
    }

    #[test]
    fn application_wheel_is_fractional_bounded_and_shift_bypasses_it() {
        let snap = mouse_snapshot();
        let mut state = MouseState::default();
        let cell = Some((0, 0));
        for _ in 0..3 {
            assert!(
                state
                    .event(
                        &wheel(0.25, Modifiers::NONE),
                        &snap,
                        cell,
                        true,
                        Modifiers::NONE,
                        16.0
                    )
                    .is_empty()
            );
        }
        let events = state.event(
            &wheel(0.25, Modifiers::NONE),
            &snap,
            cell,
            true,
            Modifiers::NONE,
            16.0,
        );
        assert_eq!(events[0].action, MouseAction::ScrollUp);
        assert!(
            state
                .event(
                    &wheel(1.0, Modifiers::SHIFT),
                    &snap,
                    cell,
                    true,
                    Modifiers::SHIFT,
                    16.0
                )
                .is_empty()
        );
        assert!(
            state
                .event(
                    &wheel(1.0, Modifiers::NONE),
                    &snap,
                    cell,
                    false,
                    Modifiers::NONE,
                    16.0
                )
                .is_empty()
        );
        let flood = state.event(
            &wheel(-1e9, Modifiers::NONE),
            &snap,
            cell,
            true,
            Modifiers::NONE,
            16.0,
        );
        assert_eq!(flood.len(), 32);
        assert!(flood.iter().all(|e| e.action == MouseAction::ScrollDown));
        let disabled = Snapshot::default();
        assert!(
            state
                .event(
                    &button(true, Modifiers::NONE),
                    &disabled,
                    cell,
                    true,
                    Modifiers::NONE,
                    16.0
                )
                .is_empty()
        );
    }

    #[test]
    fn leaving_the_window_releases_captured_buttons() {
        let snap = mouse_snapshot();
        let mut state = MouseState::default();
        state.event(
            &button(true, Modifiers::NONE),
            &snap,
            Some((1, 2)),
            true,
            Modifiers::NONE,
            16.0,
        );
        let release = state.event(
            &Event::PointerGone,
            &snap,
            None,
            false,
            Modifiers::NONE,
            16.0,
        );
        assert_eq!(release[0].action, MouseAction::Release(MouseButton::Left));
        assert!(state.held().is_none());
    }

    #[test]
    fn pointer_reports_dispatch_through_the_command_registry() {
        use fm_engine::{CommandSpec, Params, Registry};
        let mut s = Session::default();
        s.registry = Registry::from_specs(vec![CommandSpec {
            id: "term.mouse",
            label: "Mouse",
            menu: &[],
            shortcut: None,
            params: "mouse",
            enabled: |_| true,
            run: |s, params| {
                s.clipboard_outbox
                    .get_or_insert_default()
                    .push_str(&format!("{params}\n"));
                Ok(())
            },
        }])
        .expect("registry");
        let snap = mouse_snapshot();
        let ctx = egui::Context::default();
        let render = |s: &mut Session, mut events: Vec<Event>, shift| {
            events.insert(
                0,
                Event::ModifiersChanged(if shift {
                    Modifiers::SHIFT
                } else {
                    Modifiers::NONE
                }),
            );
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 350.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(400.0, 300.0), Sense::click_and_drag());
                    handle_pointer(
                        ui,
                        s,
                        1,
                        &response,
                        rect,
                        &Metrics { cw: 8.0, ch: 16.0 },
                        &snap,
                    );
                });
            });
            output.textures_delta.clear();
        };
        render(&mut s, vec![], false);
        render(
            &mut s,
            vec![
                Event::PointerMoved(egui::pos2(20.0, 20.0)),
                button(true, Modifiers::NONE),
            ],
            false,
        );
        render(
            &mut s,
            vec![button(false, Modifiers::NONE), wheel(-1.0, Modifiers::NONE)],
            false,
        );
        let reports: Vec<Params> = s
            .clipboard_outbox
            .take()
            .expect("reports")
            .lines()
            .map(|line| serde_json::from_str(line).expect("json"))
            .collect();
        assert!(
            reports
                .iter()
                .any(|e| e["action"] == "press" && e["row"] == 0 && e["col"] == 1)
        );
        assert!(reports.iter().any(|e| e["action"] == "release"));
        assert!(reports.iter().any(|e| e["action"] == "down"));
        render(
            &mut s,
            vec![button(true, Modifiers::SHIFT), wheel(1.0, Modifiers::SHIFT)],
            true,
        );
        for x in [80.0, 120.0] {
            render(&mut s, vec![Event::PointerMoved(egui::pos2(x, 20.0))], true);
        }
        render(&mut s, vec![button(false, Modifiers::SHIFT)], true);
        assert!(
            s.clipboard_outbox.is_none(),
            "Shift must keep local selection/scrollback"
        );
        let selection = s.terminal_selection.expect("Shift drag selects text");
        assert_ne!(selection.anchor, selection.head);
        render(&mut s, vec![], false);
        assert_eq!(
            s.terminal_selection,
            Some(selection),
            "selection survives releasing Shift"
        );
    }

    fn bytes(key: Key, m: Modifiers) -> Option<String> {
        key_bytes(key, m, false).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    fn frame(s: &mut Session, events: Vec<Event>) {
        let tokens = Tokens::dark();
        let ctx = egui::Context::default();
        crate::theme::apply_to(&ctx, &tokens);
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| show(ui, s, 1, &tokens, true));
        });
        out.textures_delta.clear();
    }

    fn record_terminal_command(
        s: &mut Session,
        id: &str,
        params: &fm_engine::Params,
        key: Option<&str>,
    ) -> Result<(), fm_vfs::FmError> {
        let value = key
            .and_then(|key| params.get(key))
            .and_then(fm_engine::Params::as_str)
            .unwrap_or_default();
        s.clipboard_outbox = Some(format!("{id}:{value}"));
        Ok(())
    }

    fn record_input(s: &mut Session, params: &fm_engine::Params) -> Result<(), fm_vfs::FmError> {
        record_terminal_command(s, "term.input", params, Some("data"))
    }

    fn record_copy(s: &mut Session, params: &fm_engine::Params) -> Result<(), fm_vfs::FmError> {
        record_terminal_command(s, "term.copy", params, None)
    }

    fn record_paste(s: &mut Session, params: &fm_engine::Params) -> Result<(), fm_vfs::FmError> {
        record_terminal_command(s, "term.paste", params, Some("text"))
    }

    fn forwarded_input(events: Vec<Event>, modifiers: Modifiers) -> Option<String> {
        forwarded_input_with_menu_paste(events, modifiers, false)
    }

    fn forwarded_input_with_menu_paste(
        events: Vec<Event>,
        modifiers: Modifiers,
        menu_paste: bool,
    ) -> Option<String> {
        use fm_engine::{CommandSpec, Registry};

        let mut s = Session::default();
        s.registry = Registry::from_specs(vec![
            CommandSpec {
                id: "term.input",
                label: "Terminal input test",
                menu: &[],
                shortcut: None,
                params: "test",
                enabled: |_| true,
                run: record_input,
            },
            CommandSpec {
                id: "term.copy",
                label: "Terminal copy test",
                menu: &[],
                shortcut: None,
                params: "test",
                enabled: |_| true,
                run: record_copy,
            },
            CommandSpec {
                id: "term.paste",
                label: "Terminal paste test",
                menu: &[],
                shortcut: None,
                params: "test",
                enabled: |_| true,
                run: record_paste,
            },
        ])
        .expect("registry");

        let ctx = egui::Context::default();
        if menu_paste {
            request_paste(&ctx);
        }
        let events = std::iter::once(Event::ModifiersChanged(modifiers))
            .chain(events)
            .collect();
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| forward_input(ui, &mut s, 1, &Snapshot::default()),
        );
        out.textures_delta.clear();
        s.clipboard_outbox
    }

    #[test]
    fn clipboard_events_distinguish_terminal_shortcuts_from_control_input() {
        let ctrl = Modifiers::CTRL;
        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;

        assert_eq!(
            forwarded_input(vec![key(Key::C, ctrl_shift)], ctrl_shift).as_deref(),
            Some("term.copy:")
        );
        assert_eq!(
            forwarded_input(
                vec![key(Key::V, ctrl_shift), Event::Paste("pasted".into()),],
                ctrl_shift
            )
            .as_deref(),
            Some("term.paste:pasted")
        );
        assert_eq!(
            forwarded_input_with_menu_paste(vec![Event::Paste("menu pasted".into())], ctrl, true)
                .as_deref(),
            Some("term.paste:menu pasted")
        );
        if cfg!(target_os = "macos") {
            assert_eq!(
                forwarded_input(vec![Event::Copy], Modifiers::MAC_CMD).as_deref(),
                Some("term.copy:")
            );
            assert_eq!(
                forwarded_input(vec![Event::Paste("pasted".into())], Modifiers::MAC_CMD).as_deref(),
                Some("term.paste:pasted")
            );
        } else {
            assert_eq!(
                forwarded_input(vec![Event::Copy], ctrl).as_deref(),
                Some("term.input:\u{3}")
            );
            assert_eq!(
                forwarded_input(vec![Event::Paste("pasted".into())], ctrl).as_deref(),
                Some("term.input:\u{16}")
            );
        }
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn wait(s: &mut Session, mut done: impl FnMut(&Session) -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            s.poll();
            if done(s) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[allow(clippy::expect_used)]
    #[test]
    fn backspace_and_ctrl_c_reach_the_shell() {
        let mut s = Session::new().expect("session");
        if s.dispatch("pane.kind.set", &serde_json::json!({"kind": "terminal"}))
            .is_err()
        {
            return;
        }
        // Resize happens on the first frame; typed text, Backspace and Enter follow.
        frame(&mut s, vec![]);
        frame(
            &mut s,
            vec![
                Event::Text("echoo".into()),
                key(Key::Backspace, Modifiers::NONE),
                Event::Text(" fm-$((6*7))".into()),
                key(Key::Enter, Modifiers::NONE),
            ],
        );
        assert!(wait(&mut s, |s| {
            s.terminal_snapshot(1).is_some_and(|n| {
                n.text_between((0, 0), (n.rows - 1, n.cols - 1))
                    .contains("fm-42")
            })
        }));

        // On non-mac platforms egui-winit delivers Ctrl+C as `Event::Copy`, never as a key.
        if cfg!(target_os = "macos") {
            return;
        }
        frame(
            &mut s,
            vec![
                Event::Text("sleep 30".into()),
                key(Key::Enter, Modifiers::NONE),
            ],
        );
        assert!(wait(&mut s, |s| s.terminal_busy(1)));
        frame(&mut s, vec![Event::Copy]);
        assert!(
            wait(&mut s, |s| !s.terminal_busy(1)),
            "Ctrl+C should interrupt sleep"
        );
    }

    #[test]
    fn control_letters_become_control_bytes() {
        assert_eq!(key_bytes(Key::C, Modifiers::CTRL, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::D, Modifiers::CTRL, false), Some(vec![4]));
        assert_eq!(key_bytes(Key::Z, Modifiers::CTRL, false), Some(vec![26]));
        assert_eq!(
            key_bytes(Key::OpenBracket, Modifiers::CTRL, false),
            Some(vec![0x1b])
        );
        assert_eq!(key_bytes(Key::Space, Modifiers::CTRL, false), Some(vec![0]));
    }

    #[test]
    fn plain_printable_keys_come_as_text_not_key_bytes() {
        assert_eq!(key_bytes(Key::A, Modifiers::NONE, false), None);
        assert_eq!(key_bytes(Key::Num5, Modifiers::SHIFT, false), None);
        assert_eq!(key_bytes(Key::Space, Modifiers::NONE, false), None);
    }

    #[test]
    fn ctrl_shift_chords_stay_with_the_app() {
        let m = Modifiers::CTRL | Modifiers::SHIFT;
        for key in [Key::C, Key::V, Key::Tab, Key::ArrowLeft] {
            assert_eq!(key_bytes(key, m, false), None);
        }
    }

    #[test]
    fn editing_keys_use_xterm_sequences() {
        assert_eq!(bytes(Key::Enter, Modifiers::NONE).as_deref(), Some("\r"));
        assert_eq!(
            key_bytes(Key::Backspace, Modifiers::NONE, false),
            Some(vec![0x7f])
        );
        assert_eq!(bytes(Key::Tab, Modifiers::SHIFT).as_deref(), Some("\x1b[Z"));
        assert_eq!(
            bytes(Key::Delete, Modifiers::NONE).as_deref(),
            Some("\x1b[3~")
        );
        assert_eq!(
            bytes(Key::PageUp, Modifiers::NONE).as_deref(),
            Some("\x1b[5~")
        );
        assert_eq!(bytes(Key::F1, Modifiers::NONE).as_deref(), Some("\x1bOP"));
        assert_eq!(bytes(Key::F5, Modifiers::NONE).as_deref(), Some("\x1b[15~"));
        assert_eq!(
            bytes(Key::F12, Modifiers::NONE).as_deref(),
            Some("\x1b[24~")
        );
    }

    #[test]
    fn arrows_follow_application_cursor_mode_and_modifiers() {
        assert_eq!(
            bytes(Key::ArrowUp, Modifiers::NONE).as_deref(),
            Some("\x1b[A")
        );
        assert_eq!(
            key_bytes(Key::ArrowUp, Modifiers::NONE, true)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .as_deref(),
            Some("\x1bOA")
        );
        assert_eq!(
            bytes(Key::ArrowRight, Modifiers::CTRL).as_deref(),
            Some("\x1b[1;5C")
        );
        assert_eq!(
            bytes(Key::ArrowLeft, Modifiers::ALT).as_deref(),
            Some("\x1b[1;3D")
        );
        assert_eq!(
            bytes(Key::Delete, Modifiers::CTRL).as_deref(),
            Some("\x1b[3;5~")
        );
    }

    #[test]
    fn alt_letters_send_escape_prefix() {
        assert_eq!(bytes(Key::B, Modifiers::ALT).as_deref(), Some("\x1bb"));
        assert_eq!(bytes(Key::F, Modifiers::ALT).as_deref(), Some("\x1bf"));
    }

    #[test]
    fn box_characters_map_to_arms_and_others_do_not() {
        assert_eq!(super::box_arms("┌"), Some([false, true, false, true]));
        assert_eq!(super::box_arms("┼"), Some([true; 4]));
        assert_eq!(super::box_arms("x"), None);
        assert_eq!(super::box_arms("──"), None);
        assert_eq!(super::box_arms(""), None);
    }
}
