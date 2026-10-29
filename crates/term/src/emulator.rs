use std::num::NonZeroU16;

use vt100::Parser;

use crate::charset::Charsets;
use crate::mouse::{MouseEvent, MouseProtocol};
use crate::queries::{Queries, Query};

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
    pub mouse: MouseProtocol,
}

impl Snapshot {
    pub fn buffer_cell(&self, cell: (u16, u16)) -> (usize, u16) {
        (
            self.scrollback_len.saturating_sub(self.scrollback_offset) + usize::from(cell.0),
            cell.1,
        )
    }

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
    scrollback_capacity: usize,
    parser: Parser,
    charsets: Charsets,
    scratch: Vec<u8>,
    queries: Queries,
    /// Answers to terminal queries, for the session to send back to the program.
    replies: Vec<u8>,
}

/// Cap on unsent replies, so a program flooding queries cannot grow memory.
const MAX_REPLIES: usize = 4096;

impl Emulator {
    pub fn scrollback_capacity(&self) -> usize {
        self.scrollback_capacity
    }

    pub fn new(rows: u16, cols: u16) -> Self {
        Self::with_scrollback(rows, cols, SCROLLBACK)
    }

    pub fn with_scrollback(rows: u16, cols: u16, capacity: usize) -> Self {
        let capacity = capacity.clamp(5_000, 1_000_000);
        Self {
            scrollback_capacity: capacity,
            parser: Parser::new(dim(rows, MAX_ROWS), dim(cols, MAX_COLS), capacity),
            charsets: Charsets::default(),
            queries: Queries::default(),
            replies: Vec::new(),
            scratch: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.charsets.translate(bytes, &mut self.scratch);
        // Parse up to each query before answering it, so a cursor report is accurate.
        let mut start = 0;
        for i in 0..self.scratch.len() {
            let Some(query) = self.scratch.get(i).and_then(|b| self.queries.step(*b)) else {
                continue;
            };
            if let Some(part) = self.scratch.get(start..=i) {
                self.parser.process(part);
            }
            start = i + 1;
            self.answer(query);
        }
        if let Some(rest) = self.scratch.get(start..) {
            self.parser.process(rest);
        }
    }

    fn answer(&mut self, query: Query) {
        let reply = match query {
            Query::CursorPosition => {
                let (row, col) = self.parser.screen().cursor_position();
                format!("\x1b[{};{}R", u32::from(row) + 1, u32::from(col) + 1)
            }
            Query::Status => "\x1b[0n".to_string(),
            Query::PrimaryAttributes => "\x1b[?62;22c".to_string(),
            Query::SecondaryAttributes => "\x1b[>0;10;1c".to_string(),
        };
        if self.replies.len() + reply.len() <= MAX_REPLIES {
            self.replies.extend_from_slice(reply.as_bytes());
        }
    }

    /// Replies owed to the program since the last call.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser
            .set_size(dim(rows, MAX_ROWS), dim(cols, MAX_COLS));
    }

    pub fn size(&self) -> (u16, u16) {
        let (r, c) = self.parser.screen().size();
        (r.get(), c.get())
    }

    pub fn mouse_protocol(&self) -> MouseProtocol {
        let screen = self.parser.screen();
        MouseProtocol {
            mode: screen.mouse_protocol_mode(),
            encoding: screen.mouse_protocol_encoding(),
        }
    }

    pub fn mouse_report(&self, event: MouseEvent) -> Result<Vec<u8>, fm_vfs::FmError> {
        let (rows, cols) = self.size();
        self.mouse_protocol().report(event, rows, cols)
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
            .set_scrollback(next.min(self.scrollback_capacity));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }

    pub fn clear_scrollback(&mut self) {
        self.parser.screen_mut().clear_scrollback();
    }

    /// Copy retained buffer cells without changing the displayed scroll position.
    pub fn text_between(&mut self, a: (usize, u16), b: (usize, u16)) -> String {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let history = self.parser.screen().scrollback_len();
        let offset = self.parser.screen().scrollback();
        let (rows, cols) = self.size();
        let last = history + usize::from(rows) - 1;
        let mut out = String::new();
        for row in start.0..=end.0.min(last) {
            let screen_row = if row < history {
                self.parser.screen_mut().set_scrollback(history - row);
                0
            } else {
                self.parser.screen_mut().set_scrollback(0);
                (row - history) as u16
            };
            let from = if row == start.0 { start.1 } else { 0 };
            let to = if row == end.0 {
                end.1.min(cols - 1)
            } else {
                cols - 1
            };
            let mut line = String::new();
            for col in from..=to {
                if let Some(cell) = self.parser.screen().cell(screen_row, col)
                    && !cell.is_wide_continuation()
                {
                    if cell.contents().is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(cell.contents());
                    }
                }
            }
            if row > start.0 {
                out.push('\n');
            }
            out.push_str(line.trim_end());
        }
        self.parser.screen_mut().set_scrollback(offset);
        out
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
            mouse: self.mouse_protocol(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn application_controls_mouse_reporting_and_reset() {
        use crate::{MouseProtocolEncoding as E, MouseProtocolMode as M};
        let mut e = Emulator::new(24, 80);
        assert!(!e.snapshot().mouse.enabled());
        for (sequence, mode) in [
            (b"\x1b[?9h".as_slice(), M::Press),
            (b"\x1b[?1000h", M::PressRelease),
            (b"\x1b[?1002h", M::ButtonMotion),
            (b"\x1b[?1003h", M::AnyMotion),
        ] {
            for byte in sequence {
                e.feed(&[*byte]);
            }
            assert_eq!(e.snapshot().mouse.mode, mode);
        }
        e.feed(b"\x1b[?1005h");
        assert_eq!(e.snapshot().mouse.encoding, E::Utf8);
        e.feed(b"\x1b[?1006h");
        assert_eq!(e.snapshot().mouse.encoding, E::Sgr);
        e.feed(b"\x1b[?1003l\x1b[?1006l");
        assert_eq!(e.snapshot().mouse, MouseProtocol::default());
        e.feed(b"\x1b[?1000h\x1b[?1006h\x1bc");
        assert_eq!(e.snapshot().mouse, MouseProtocol::default());
    }

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
    fn queries_are_answered_with_the_cursor_at_that_point() {
        let mut e = Emulator::new(5, 20);
        // ConPTY's startup handshake: nothing is drawn until this is answered.
        e.feed(b"\x1b[6n");
        assert_eq!(e.take_replies(), b"\x1b[1;1R");
        e.feed(b"ab\r\ncd\x1b[6nefgh\x1b[5n\x1b[");
        e.feed(b"c");
        assert_eq!(e.take_replies(), b"\x1b[2;3R\x1b[0n\x1b[?62;22c");
        assert!(e.take_replies().is_empty());
        assert_eq!(text(&e), "ab\ncdefgh");
        for _ in 0..10_000 {
            e.feed(b"\x1b[6n");
        }
        assert!(e.take_replies().len() <= MAX_REPLIES);
    }

    #[test]
    fn dec_line_drawing_renders_as_box_characters() {
        let mut e = Emulator::new(4, 10);
        e.feed(b"\x1b(0lqk\r\nx x\r\nmqj\x1b(B ok");
        assert_eq!(text(&e), "┌─┐\n│ │\n└─┘ ok");
    }

    #[test]
    fn resizing_reflows_soft_wraps_but_preserves_hard_newlines() {
        let mut e = Emulator::new(8, 12);
        e.feed(b"abcdefghijklmnopqrst\r\nhard break\r\nnext");
        e.resize(8, 6);
        assert_eq!(text(&e), "abcdef\nghijkl\nmnopqr\nst\nhard b\nreak\nnext");
        e.resize(8, 20);
        assert_eq!(text(&e), "abcdefghijklmnopqrst\nhard break\nnext");
        e.feed(b"!");
        assert!(text(&e).ends_with("next!"), "cursor follows reflowed text");
        for width in [7, 30, 2, 12, 20] {
            e.resize(12, width);
        }
        assert_eq!(text(&e), "abcdefghijklmnopqrst\nhard break\nnext!");
    }

    #[test]
    fn reflow_preserves_attributes_wide_characters_and_combining_marks() {
        let mut e = Emulator::new(12, 8);
        e.feed("\x1b[31;1mabc漢e\u{301}語xyz\x1b[0m\r\nend".as_bytes());
        e.resize(12, 4);
        e.resize(12, 30);
        let snap = e.snapshot();
        assert_eq!(text(&e), "abc漢e\u{301}語xyz\nend");
        let wide = snap.cell(0, 3).expect("wide");
        assert_eq!(wide.text, "漢");
        assert!(wide.bold);
        assert_eq!(wide.fg, Color::Indexed(1));
        assert!(snap.cell(0, 4).expect("continuation").wide_continuation);
        assert_eq!(snap.cell(0, 5).expect("combining").text, "e\u{301}");
        assert_eq!(snap.cell(1, 0).expect("reset").fg, Color::Default);
    }

    #[test]
    fn alternate_screen_is_not_reflowed_and_normal_buffer_survives() {
        let mut e = Emulator::new(8, 12);
        e.feed(b"abcdefghijklmnop\x1b[?1049h\x1b[Habcdefghijkl");
        e.resize(8, 6);
        assert!(e.snapshot().alternate_screen);
        assert_eq!(text(&e), "abcdef");
        e.feed(b"\x1b[?1049l");
        e.resize(8, 20);
        assert_eq!(text(&e), "abcdefghijklmnop");
        e.feed(b"!");
        assert_eq!(text(&e), "abcdefghijklmnop!");
    }

    #[test]
    fn reflow_keeps_scrollback_text_and_view_anchor() {
        let mut e = Emulator::new(4, 12);
        for n in 0..30 {
            e.feed(format!("row-{n:02}-abcdefgh\r\n").as_bytes());
        }
        e.scroll(10);
        let original = e.snapshot();
        let top_line = (original.scrollback_len - original.scrollback_offset) / 2;
        e.resize(4, 6);
        assert!(e.snapshot().scrollback_offset > 0);
        e.resize(4, 20);
        let wide = e.snapshot();
        assert!(wide.scrollback_offset > 0);
        assert_eq!(
            wide.text_between((0, 0), (0, 19)),
            format!("row-{top_line:02}-abcdefgh"),
        );
        let all = e.text_between((0, 0), (usize::MAX, u16::MAX));
        for n in 0..30 {
            assert!(all.contains(&format!("row-{n:02}-abcdefgh")), "{n}");
        }
    }

    #[test]
    fn reflow_preserves_pending_wrap_saved_cursor_and_explicit_blank_lines() {
        let mut e = Emulator::new(12, 12);
        e.feed(b"abcdefghijkl\x1b7\r\n\r\nhard\r\n");
        e.resize(12, 4);
        e.resize(12, 20);
        assert_eq!(text(&e), "abcdefghijkl\n\nhard");
        e.feed(b"\x1b8!");
        assert!(text(&e).starts_with("abcdefghijkl!"));

        let mut exact = Emulator::new(4, 12);
        exact.feed(b"abcdefghijkl");
        exact.resize(4, 4);
        assert_eq!(exact.snapshot().cursor, Some((2, 4)));
        exact.resize(5, 4);
        exact.feed(b"!");
        exact.resize(5, 20);
        assert_eq!(text(&exact), "abcdefghijkl!");
    }

    #[test]
    fn reflow_keeps_cursor_on_the_same_character_when_not_at_the_end() {
        let mut e = Emulator::new(10, 12);
        e.feed(b"abcdefghijklmnop\r\nsecond\x1b[1;6H");
        e.resize(10, 4);
        e.feed(b"X");
        e.resize(10, 20);
        assert_eq!(text(&e), "abcdeXghijklmnop\nsecond");
    }

    #[test]
    fn reflow_capacity_stays_bounded_when_history_expands() {
        let mut e = Emulator::new(3, 60);
        for n in 0..1200 {
            e.feed(format!("entry-{n:04}-{}\r\n", "a".repeat(40)).as_bytes());
        }
        e.resize(3, 2);
        let small = e.snapshot();
        assert_eq!(small.scrollback_len, SCROLLBACK);
        e.resize(3, 60);
        assert!(e.snapshot().scrollback_len <= SCROLLBACK);
        let retained = e.text_between((0, 0), (usize::MAX, u16::MAX));
        assert!(retained.contains("entry-1199-"));
        assert!(
            !retained.contains("entry-0000-"),
            "oldest rows were evicted"
        );
        e.feed(b"after resize");
        assert!(text(&e).contains("after resize"));
    }

    #[test]
    fn reflow_does_not_evict_the_cursor_when_discarding_excess_bottom_rows() {
        let mut e = Emulator::new(60, 600);
        for _ in 0..40 {
            e.feed(format!("{}\r\n", "a".repeat(580)).as_bytes());
        }
        e.feed(b"\x1b[H");
        e.resize(60, 2);
        let snap = e.snapshot();
        assert_eq!(snap.cursor, Some((0, 0)));
        assert_eq!(snap.scrollback_len, 0);
        assert_eq!(snap.cell(0, 0).expect("top").text, "a");
        assert_eq!(snap.cell(59, 1).expect("bottom").text, "a");
        e.resize(60, 600);
        assert_eq!(text(&e), "a".repeat(120));
    }

    #[test]
    fn height_growth_preserves_soft_wraps_on_trimmed_wide_history_rows() {
        let mut e = Emulator::new(12, 40);
        let line = "漢".repeat(10);
        e.feed(line.as_bytes());
        e.resize(2, 5);
        assert!(e.snapshot().scrollback_len > 0);
        e.resize(12, 5);
        e.resize(12, 40);
        assert_eq!(text(&e), line);
    }

    #[test]
    fn configurable_scrollback_retains_and_copies_beyond_the_old_limit() {
        for capacity in [5_000, 15_000] {
            let mut e = Emulator::with_scrollback(2, 20, capacity);
            for n in 0..16_000 {
                e.feed(format!("row-{n:05}\r\n").as_bytes());
            }
            assert_eq!(e.snapshot().scrollback_len, capacity);
            e.scroll(i32::MAX);
            assert_eq!(e.snapshot().scrollback_offset, capacity);
            let first = 16_000 - capacity - 1;
            assert_eq!(e.text_between((0, 0), (0, 19)), format!("row-{first:05}"));
            e.resize(2, 10);
            assert!(e.snapshot().scrollback_len <= capacity);
        }
        let e = Emulator::with_scrollback(2, 2, 1_000_000);
        assert_eq!(e.scrollback_capacity(), 1_000_000);
        assert_eq!(e.snapshot().scrollback_len, 0);
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
    fn selection_spans_history_and_live_screen_without_moving_the_view() {
        let mut e = Emulator::new(4, 20);
        for n in 0..20 {
            e.feed(format!("line{n:02}\r\n").as_bytes());
        }
        e.scroll(8);
        let before = e.snapshot();
        let anchor = before.buffer_cell((0, 0));
        e.scroll(-5);
        let after = e.snapshot();
        let head = after.buffer_cell((3, 19));
        let expected = (anchor.0..=head.0)
            .map(|n| format!("line{n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(e.text_between(anchor, head), expected);
        assert_eq!(e.text_between(head, anchor), expected);
        assert_eq!(e.snapshot(), after, "copy must not move the viewport");
        assert!(head.0 - anchor.0 > usize::from(after.rows));
        assert!(e.text_between((usize::MAX, 0), (usize::MAX, 10)).is_empty());
    }

    #[test]
    fn selection_text_skips_wide_continuations_and_trims() {
        let mut e = Emulator::new(2, 10);
        e.feed("a漢b  ".as_bytes());
        let s = e.snapshot();
        assert_eq!(s.text_between((0, 0), (0, 9)), "a漢b");
        assert_eq!(e.text_between((0, 0), (0, 9)), "a漢b");
        assert_eq!(
            s.text_between((0, 9), (0, 0)),
            "a漢b",
            "order does not matter"
        );
    }

    proptest::proptest! {
        #[test]
        fn width_roundtrip_preserves_plaintext(
            lines in proptest::collection::vec("[a-zA-Z0-9 ]{0,55}", 1..8),
            narrow in 2u16..32,
        ) {
            let mut e = Emulator::new(40, 60);
            e.feed(lines.join("\r\n").as_bytes());
            let before = text(&e);
            let cursor = e.snapshot().cursor;
            e.resize(40, narrow);
            e.resize(40, 60);
            proptest::prop_assert_eq!(text(&e), before);
            proptest::prop_assert_eq!(e.snapshot().cursor, cursor);
        }

        #[test]
        fn width_roundtrip_preserves_unicode(
            lines in proptest::collection::vec("[ab 漢語é😀]{0,40}", 1..8),
            narrow in 2u16..20,
            middle in 2u16..32,
        ) {
            let mut e = Emulator::new(40, 100);
            e.feed(lines.join("\r\n").as_bytes());
            let before = text(&e);
            let cursor = e.snapshot().cursor;
            e.resize(15, narrow);
            e.resize(27, middle);
            e.resize(40, 100);
            proptest::prop_assert_eq!(text(&e), before);
            proptest::prop_assert_eq!(e.snapshot().cursor, cursor);
        }

        #[test]
        fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)) {
            let mut e = Emulator::new(10, 30);
            for (index, chunk) in bytes.chunks(37).enumerate() {
                e.feed(chunk);
                e.resize((index % 17 + 2) as u16, (index % 43 + 2) as u16);
            }
            e.resize(4, 7);
            e.resize(10, 30);
            e.resize(2, 2);
            e.resize(12, 40);
            let _ = e.snapshot();
        }
    }
}
