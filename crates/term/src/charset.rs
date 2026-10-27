//! DEC Special Graphics ("line drawing") translation.
//!
//! `vt100` ignores character-set designation, so curses/dialog programs that draw boxes
//! with `ESC ( 0` + `lqqk` would show letters. This filter sits in front of the parser:
//! it tracks G0/G1 designation and SO/SI shifts, swallows those controls, and rewrites
//! printable bytes in graphics mode to the matching Unicode characters. Escape sequences
//! pass through untouched, and state survives across `feed` chunks.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// After ESC.
    Esc,
    /// After `ESC (` (G0) or `ESC )` (G1): the next byte names the set.
    Designate(usize),
    /// Inside CSI: until a final byte.
    Csi,
    /// Inside OSC/DCS/APC/PM/SOS: until BEL or ESC.
    Str,
}

#[derive(Clone, Debug, Default)]
pub struct Charsets {
    state: State,
    /// Whether G0/G1 is the line-drawing set.
    graphics: [bool; 2],
    /// Active set: 0 after SI (default), 1 after SO.
    active: usize,
}

const ESC: u8 = 0x1b;
const SO: u8 = 0x0e;
const SI: u8 = 0x0f;

impl Charsets {
    /// Rewrite `input` into `out` (cleared first).
    pub fn translate(&mut self, input: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.reserve(input.len());
        for &b in input {
            match self.state {
                State::Ground => match b {
                    ESC => self.state = State::Esc,
                    SO => self.active = 1,
                    SI => self.active = 0,
                    _ => match self.map(b) {
                        Some(c) => {
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        None => out.push(b),
                    },
                },
                State::Esc => match b {
                    b'(' => self.state = State::Designate(0),
                    b')' => self.state = State::Designate(1),
                    _ => {
                        out.extend_from_slice(&[ESC, b]);
                        self.state = match b {
                            b'[' => State::Csi,
                            b']' | b'P' | b'X' | b'^' | b'_' => State::Str,
                            ESC => State::Esc,
                            b'c' => {
                                self.graphics = [false; 2];
                                self.active = 0;
                                State::Ground
                            }
                            _ => State::Ground,
                        };
                        if b == ESC {
                            // The first ESC was a lone one; keep only the new one pending.
                            out.pop();
                        }
                    }
                },
                State::Designate(g) => {
                    if let Some(slot) = self.graphics.get_mut(g) {
                        *slot = b == b'0';
                    }
                    self.state = State::Ground;
                }
                State::Csi => {
                    if b == ESC {
                        self.state = State::Esc;
                    } else {
                        out.push(b);
                        if (0x40..=0x7e).contains(&b) || b == 0x18 || b == 0x1a {
                            self.state = State::Ground;
                        }
                    }
                }
                State::Str => {
                    if b == ESC {
                        self.state = State::Esc;
                    } else {
                        out.push(b);
                        if b == 0x07 || b == 0x18 || b == 0x1a {
                            self.state = State::Ground;
                        }
                    }
                }
            }
        }
    }

    fn map(&self, b: u8) -> Option<char> {
        if !self.graphics.get(self.active).copied().unwrap_or(false) {
            return None;
        }
        Some(match b {
            b'`' => '◆',
            b'a' => '▒',
            b'f' => '°',
            b'g' => '±',
            b'j' => '┘',
            b'k' => '┐',
            b'l' => '┌',
            b'm' => '└',
            b'n' => '┼',
            b'o' => '⎺',
            b'p' => '⎻',
            b'q' => '─',
            b'r' => '⎼',
            b's' => '⎽',
            b't' => '├',
            b'u' => '┤',
            b'v' => '┴',
            b'w' => '┬',
            b'x' => '│',
            b'y' => '≤',
            b'z' => '≥',
            b'{' => 'π',
            b'|' => '≠',
            b'}' => '£',
            b'~' => '·',
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut Charsets, input: &[u8]) -> String {
        let mut out = Vec::new();
        c.translate(input, &mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn g0_line_drawing_becomes_box_characters() {
        let mut c = Charsets::default();
        assert_eq!(run(&mut c, b"\x1b(0lqqk\x1b(B lqk"), "┌──┐ lqk");
    }

    #[test]
    fn shift_out_uses_g1_and_shift_in_returns() {
        let mut c = Charsets::default();
        assert_eq!(run(&mut c, b"\x1b)0a\x0ex\x0fx"), "a│x");
    }

    #[test]
    fn escape_sequences_pass_through_untranslated() {
        let mut c = Charsets::default();
        let s = run(&mut c, b"\x1b(0\x1b[1;34mq\x1b]0;title\x07q\x1b[?25l");
        assert_eq!(s, "\x1b[1;34m─\x1b]0;title\x07─\x1b[?25l");
    }

    #[test]
    fn state_survives_chunk_boundaries() {
        let mut c = Charsets::default();
        let mut s = run(&mut c, b"\x1b");
        s += &run(&mut c, b"(");
        s += &run(&mut c, b"0x\x1b[3");
        s += &run(&mut c, b"1mx");
        assert_eq!(s, "│\x1b[31m│");
    }

    #[test]
    fn reset_restores_ascii() {
        let mut c = Charsets::default();
        assert_eq!(run(&mut c, b"\x1b(0q\x1bcq"), "─\x1bcq");
    }
}
