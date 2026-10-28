use fm_vfs::FmError;

pub use vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press(MouseButton),
    Release(MouseButton),
    Move(Option<MouseButton>),
    ScrollUp,
    ScrollDown,
}

/// Coordinates are zero-based screen cells, not pixels or scrollback positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub row: u16,
    pub col: u16,
    pub action: MouseAction,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseProtocol {
    pub mode: MouseProtocolMode,
    pub encoding: MouseProtocolEncoding,
}

impl MouseProtocol {
    pub fn enabled(self) -> bool {
        self.mode != MouseProtocolMode::None
    }

    /// Empty when this mode does not request the event, or a legacy coordinate cannot
    /// represent it. Never clamp a legacy report onto a different, clickable cell.
    pub fn report(self, event: MouseEvent, rows: u16, cols: u16) -> Result<Vec<u8>, FmError> {
        if event.row >= rows || event.col >= cols {
            return Err(FmError::InvalidParams(
                "mouse cell is outside the terminal".into(),
            ));
        }
        use MouseAction as A;
        use MouseProtocolMode as M;
        let wanted = match event.action {
            A::Press(_) => self.enabled(),
            A::Release(_) | A::ScrollUp | A::ScrollDown => {
                matches!(self.mode, M::PressRelease | M::ButtonMotion | M::AnyMotion)
            }
            A::Move(button) => {
                self.mode == M::AnyMotion || (self.mode == M::ButtonMotion && button.is_some())
            }
        };
        if !wanted {
            return Ok(Vec::new());
        }
        let release = matches!(event.action, A::Release(_));
        let code = match event.action {
            A::Press(b) | A::Release(b) => b.code(),
            A::Move(b) => 32 + b.map_or(3, MouseButton::code),
            A::ScrollUp => 64,
            A::ScrollDown => 65,
        } + u8::from(event.shift) * 4
            + u8::from(event.alt) * 8
            + u8::from(event.ctrl) * 16;
        let x = u32::from(event.col) + 1;
        let y = u32::from(event.row) + 1;
        if self.encoding == MouseProtocolEncoding::Sgr {
            let end = if release { 'm' } else { 'M' };
            return Ok(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
        }
        let code = if release { (code & 28) | 3 } else { code };
        let mut bytes = b"\x1b[M".to_vec();
        for value in [u32::from(code) + 32, x + 32, y + 32] {
            match self.encoding {
                MouseProtocolEncoding::Default => {
                    if value > 255 {
                        return Ok(Vec::new());
                    }
                    bytes.push(value as u8);
                }
                MouseProtocolEncoding::Utf8 => {
                    let c = char::from_u32(value).ok_or_else(|| {
                        FmError::InvalidParams("mouse coordinate cannot be encoded".into())
                    })?;
                    let mut buf = [0; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                MouseProtocolEncoding::Sgr => {}
            }
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(action: MouseAction) -> MouseEvent {
        MouseEvent {
            row: 3,
            col: 7,
            action,
            shift: false,
            alt: false,
            ctrl: false,
        }
    }

    #[test]
    fn sgr_click_release_drag_wheel_and_modifiers() {
        use MouseAction as A;
        let p = MouseProtocol {
            mode: MouseProtocolMode::AnyMotion,
            encoding: MouseProtocolEncoding::Sgr,
        };
        for (action, expected) in [
            (A::Press(MouseButton::Left), "\x1b[<0;8;4M"),
            (A::Release(MouseButton::Right), "\x1b[<2;8;4m"),
            (A::Move(Some(MouseButton::Middle)), "\x1b[<33;8;4M"),
            (A::Move(None), "\x1b[<35;8;4M"),
            (A::ScrollUp, "\x1b[<64;8;4M"),
            (A::ScrollDown, "\x1b[<65;8;4M"),
        ] {
            assert_eq!(
                p.report(event(action), 24, 80).expect("report"),
                expected.as_bytes()
            );
        }
        let e = MouseEvent {
            shift: true,
            alt: true,
            ctrl: true,
            ..event(A::Press(MouseButton::Left))
        };
        assert_eq!(p.report(e, 24, 80).expect("report"), b"\x1b[<28;8;4M");
    }

    #[test]
    fn reporting_respects_requested_mode() {
        use MouseAction as A;
        use MouseProtocolMode as M;
        for (mode, wanted) in [
            (M::None, [false; 5]),
            (M::Press, [true, false, false, false, false]),
            (M::PressRelease, [true, true, false, false, true]),
            (M::ButtonMotion, [true, true, true, false, true]),
            (M::AnyMotion, [true; 5]),
        ] {
            let p = MouseProtocol {
                mode,
                encoding: MouseProtocolEncoding::Sgr,
            };
            for (action, wanted) in [
                A::Press(MouseButton::Left),
                A::Release(MouseButton::Left),
                A::Move(Some(MouseButton::Left)),
                A::Move(None),
                A::ScrollUp,
            ]
            .into_iter()
            .zip(wanted)
            {
                assert_eq!(
                    !p.report(event(action), 24, 80).expect("report").is_empty(),
                    wanted,
                    "{mode:?} {action:?}"
                );
            }
        }
    }

    #[test]
    fn legacy_bytes_utf8_and_coordinate_limits() {
        let mut p = MouseProtocol {
            mode: MouseProtocolMode::PressRelease,
            encoding: MouseProtocolEncoding::Default,
        };
        assert_eq!(
            p.report(event(MouseAction::Press(MouseButton::Left)), 24, 80)
                .expect("report"),
            b"\x1b[M ($"
        );
        assert_eq!(
            p.report(event(MouseAction::Release(MouseButton::Right)), 24, 80)
                .expect("report"),
            b"\x1b[M#($"
        );
        // A byte above ASCII must stay a byte, not be UTF-8 encoded by term.input.
        let e = MouseEvent {
            col: 100,
            row: 222,
            ..event(MouseAction::Press(MouseButton::Left))
        };
        assert_eq!(
            p.report(e, 300, 600).expect("report"),
            [27, 91, 77, 32, 133, 255]
        );
        let e = MouseEvent { col: 223, ..e };
        assert!(p.report(e, 300, 600).expect("unrepresentable").is_empty());
        p.encoding = MouseProtocolEncoding::Utf8;
        assert_eq!(
            p.report(e, 300, 600).expect("report"),
            "\x1b[M \u{100}\u{ff}".as_bytes()
        );
        assert!(p.report(e, 24, 80).is_err());
    }
}
