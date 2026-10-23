//! Terminal pane body: paints the engine's screen snapshot and turns key, paste, wheel
//! and mouse input into `term.*` commands. No pty access here (capability `terminal-pane`).

use egui::{
    Align2, Color32, Event, EventFilter, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui, vec2,
};
use fm_engine::Session;
use fm_engine::term_cmds::{TermSelection, exit_code};
use fm_layout::PaneId;
use fm_term::Snapshot;

use crate::theme::Tokens;

const PAD: f32 = 4.0;

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
        .layout_no_wrap("M".to_string(), tokens.font(), tokens.text);
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
    s.terminal_resize(pane, rows, cols);

    let Some(snap) = s.terminal_snapshot(pane) else {
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
    }
    handle_pointer(ui, s, pane, &response, rect, &m, &snap);
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
                if modifiers.shift && !modifiers.ctrl && matches!(key, Key::PageUp | Key::PageDown)
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
            // egui-winit turns Ctrl/Cmd+C/X/V into these events and drops the key press, so on
            // platforms where Ctrl is not the clipboard modifier they must reach the shell as
            // control bytes. On macOS Cmd+C/V stay copy/paste (Ctrl+C arrives as a key).
            Event::Copy if cfg!(target_os = "macos") => {
                flush(s, &mut data);
                s.dispatch_or_notice("term.copy", &serde_json::Value::Null);
            }
            Event::Copy => data.push('\u{3}'),
            Event::Cut => data.push('\u{18}'),
            Event::Paste(text) if cfg!(target_os = "macos") || mods.shift => {
                flush(s, &mut data);
                s.dispatch_or_notice(
                    "term.paste",
                    &serde_json::json!({ "pane": pane, "text": text }),
                );
            }
            Event::Paste(_) => data.push('\u{16}'),
            _ => {}
        }
    }
    flush(s, &mut data);
    if scroll != 0 {
        s.terminal_scroll(pane, scroll);
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
    let cell = cell_at(rect, m, snap, pos);
    if response.drag_started() {
        s.terminal_select(Some(TermSelection {
            pane,
            anchor: cell,
            head: cell,
        }));
    } else if response.dragged()
        && let Some(sel) = s.terminal_selection.filter(|x| x.pane == pane)
    {
        s.terminal_select(Some(TermSelection { head: cell, ..sel }));
    } else if response.clicked() {
        s.terminal_select(None);
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
    let font = tokens.font();
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
            if selection.is_some_and(|(a, b)| (row, col) >= a && (row, col) <= b) {
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
            tokens.font(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
