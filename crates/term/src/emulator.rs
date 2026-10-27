use std::num::NonZeroU16;

use vt100::Parser;

use crate::charset::Charsets;

/// Lines of scrollback kept per terminal.
pub const SCROLLBACK: usize = 10_000;
/// Smallest and largest grid accepted; hostile or absurd sizes are clamped.
pub const MIN_SIZE: u16 = 2;
pub const MAX_ROWS: u16 = 300;
pub const MAX_COLS: u16 = 600;

/// A terminal colour as the parser reports it; the UI maps it through the theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for Color {
    fn from(c: vt100::Color) -> Self {
        match c {
            vt100::Color::Default => Self::Default,
            vt100::Color::Idx(i) => Self::Indexed(i),
            vt100::Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

/// One grid cell. A wide character occupies two cells: the first holds the glyph, the
/// second is a `wide_continuation` with no text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
    pub wide_continuation: bool,
}

/// An immutable copy of the visible screen, safe to hand to the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: u16,
    pub cols: u16,
    pub cells: Vec<Cell>,
    /// `(row, col)`; `None` when the app hid the cursor or the view is scrolled back.
    pub cursor: Option<(u16, u16)>,
    pub scrollback_offset: usize,
    pub scrollback_len: usize,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub alternate_screen: bool,
}

impl Snapshot {
    pub fn cell(&self, row: u16, col: u16) -> Option<&Cell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.cells
            .get(usize::from(row) * usize::from(self.cols) + usize::from(col))
    }

    /// Text between two cells (inclusive), rows joined by `\n`, trailing blanks trimmed.
    pub fn text_between(&self, a: (u16, u16), b: (u16, u16)) -> String {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        for row in start.0..=end.0.min(self.rows.saturating_sub(1)) {
            let from = if row == start.0 { start.1 } else { 0 };
            let to = if row == end.0 {
                end.1.min(self.cols.saturating_sub(1))
            } else {
                self.cols.saturating_sub(1)
            };
            let mut line = String::new();
            for col in from..=to {
                match self.cell(row, col) {
                    Some(c) if c.wide_continuation => {}
                    Some(c) if c.text.is_empty() => line.push(' '),
                    Some(c) => line.push_str(&c.text),
                    None => {}
                }
            }
            out.push_str(line.trim_end());
            if row != end.0 {
                out.push('\n');
            }
        }
        out
    }
}

fn dim(v: u16, max: u16) -> NonZeroU16 {
    NonZeroU16::new(v.clamp(MIN_SIZE, max)).unwrap_or(NonZeroU16::MIN)
}

/// The VT parser and screen model. Pure data: feed it bytes, read snapshots.
pub struct Emulator {
    parser: Parser,
    charsets: Charsets,
    scratch: Vec<u8>,
}

impl Emulator {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: Parser::new(dim(rows, MAX_ROWS), dim(cols, MAX_COLS), SCROLLBACK),
            charsets: Charsets::default(),
            scratch: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.charsets.translate(bytes, &mut self.scratch);
        self.parser.process(&self.scratch);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser
            .set_size(dim(rows, MAX_ROWS), dim(cols, MAX_COLS));
    }

    pub fn size(&self) -> (u16, u16) {
        let (r, c) = self.parser.screen().size();
        (r.get(), c.get())
    }

    /// Scroll the view into history by `lines` (negative = towards the live screen).
    pub fn scroll(&mut self, lines: i32) {
        let now = self.parser.screen().scrollback();
        let next = if lines >= 0 {
            now.saturating_add(lines as usize)
        } else {
            now.saturating_sub(lines.unsigned_abs() as usize)
        };
        self.parser
            .screen_mut()
            .set_scrollback(next.min(SCROLLBACK));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }

    pub fn clear_scrollback(&mut self) {
        self.parser.screen_mut().clear_scrollback();
    }

    pub fn snapshot(&self) -> Snapshot {
        let screen = self.parser.screen();
        let (rows, cols) = self.size();
        let mut cells = Vec::with_capacity(usize::from(rows) * usize::from(cols));
        for r in 0..rows {
            for c in 0..cols {
                cells.push(match screen.cell(r, c) {
                    Some(cell) => Cell {
                        text: cell.contents().to_string(),
                        fg: cell.fgcolor().into(),
                        bg: cell.bgcolor().into(),
                        bold: cell.bold(),
                        dim: cell.dim(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        inverse: cell.inverse(),
                        wide: cell.is_wide(),
                        wide_continuation: cell.is_wide_continuation(),
                    },
                    None => Cell::default(),
                });
            }
        }
        let offset = screen.scrollback();
        Snapshot {
            rows,
            cols,
            cells,
            cursor: (!screen.hide_cursor() && offset == 0).then(|| screen.cursor_position()),
            scrollback_offset: offset,
            scrollback_len: screen.scrollback_len(),
            application_cursor: screen.application_cursor(),
            bracketed_paste: screen.bracketed_paste(),
            alternate_screen: screen.alternate_screen(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn text(e: &Emulator) -> String {
        let s = e.snapshot();
        s.text_between((0, 0), (s.rows - 1, s.cols - 1))
            .trim_end()
            .to_string()
    }

    #[test]
    fn plain_text_and_colours_land_in_the_grid() {
        let mut e = Emulator::new(5, 20);
        e.feed(b"hi \x1b[31mred\x1b[0m\r\nnext");
        let s = e.snapshot();
        assert_eq!(s.cell(0, 0).map(|c| c.text.as_str()), Some("h"));
        assert_eq!(s.cell(0, 3).map(|c| c.fg), Some(Color::Indexed(1)));
        assert_eq!(text(&e), "hi red\nnext");
        assert_eq!(s.cursor, Some((1, 4)));
    }

    #[test]
    fn dec_line_drawing_renders_as_box_characters() {
        let mut e = Emulator::new(4, 10);
        e.feed(b"\x1b(0lqk\r\nx x\r\nmqj\x1b(B ok");
        assert_eq!(text(&e), "┌─┐\n│ │\n└─┘ ok");
    }

    #[test]
    fn sizes_are_clamped() {
        let mut e = Emulator::new(0, 0);
        assert_eq!(e.size(), (MIN_SIZE, MIN_SIZE));
        e.resize(u16::MAX, u16::MAX);
        assert_eq!(e.size(), (MAX_ROWS, MAX_COLS));
        e.resize(10, 40);
        assert_eq!(e.snapshot().cells.len(), 400);
    }

    #[test]
    fn scrollback_is_bounded_and_scrolls() {
        let mut e = Emulator::new(3, 10);
        for i in 0..50 {
            e.feed(format!("line{i}\r\n").as_bytes());
        }
        e.scroll(5);
        let s = e.snapshot();
        assert_eq!(s.scrollback_offset, 5);
        assert!(s.cursor.is_none(), "cursor hidden while scrolled back");
        e.scroll(-100);
        assert_eq!(e.snapshot().scrollback_offset, 0);
        e.scroll(i32::MAX);
        assert!(e.snapshot().scrollback_offset <= SCROLLBACK);
        e.clear_scrollback();
        e.scroll_to_bottom();
        assert_eq!(e.snapshot().scrollback_len, 0);
    }

    #[test]
    fn hostile_output_never_panics_and_stays_bounded() {
        let mut e = Emulator::new(24, 80);
        let mut oversized = b"\x1b]0;".to_vec();
        oversized.extend(std::iter::repeat_n(b'A', 2_000_000));
        oversized.push(0x07);
        e.feed(&oversized);
        e.feed(&b"\x1b[999999999;999999999H\x1b[99999999999C\x1b[?1049h\x1b[2J\x1bc"[..]);
        let mut x: u32 = 0x1234_5678;
        let noise: Vec<u8> = (0..200_000)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        e.feed(&noise);
        e.resize(3, 3);
        e.feed(&noise);
        assert_eq!(e.snapshot().cells.len(), 9);
    }

    #[test]
    fn selection_text_skips_wide_continuations_and_trims() {
        let mut e = Emulator::new(2, 10);
        e.feed("a漢b  ".as_bytes());
        let s = e.snapshot();
        assert_eq!(s.text_between((0, 0), (0, 9)), "a漢b");
        assert_eq!(
            s.text_between((0, 9), (0, 0)),
            "a漢b",
            "order does not matter"
        );
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)) {
            let mut e = Emulator::new(10, 30);
            e.feed(&bytes);
            e.resize(4, 7);
            let _ = e.snapshot();
        }
    }
}
