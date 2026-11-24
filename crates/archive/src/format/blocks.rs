//! Cuts a tool's listing, arriving in chunks, into the blocks (one per entry) that its
//! parser reads, so entries can be used while the tool is still printing. Knows nothing
//! about what a block means.

/// Where one block ends and the next starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A blank line ends a block (7-Zip `-slt`).
    BlankLine,
    /// A line starting with this text begins a new block (`unrar lt`: `Name:`). Whatever
    /// came before it, such as a banner, is its own block.
    LineStart(&'static str),
}

#[derive(Debug)]
pub struct Blocks {
    rule: Rule,
    line: Vec<u8>,
    block: String,
}

impl Blocks {
    pub fn new(rule: Rule) -> Self {
        Self {
            rule,
            line: Vec::new(),
            block: String::new(),
        }
    }

    /// Take the next bytes; `emit` gets every block that is now complete.
    pub fn feed(&mut self, chunk: &[u8], emit: &mut dyn FnMut(&str)) {
        for part in chunk.split_inclusive(|&b| b == b'\n') {
            self.line.extend_from_slice(part);
            if part.last() == Some(&b'\n') {
                self.end_line(emit);
            }
        }
    }

    /// The tool is done: the last block and an unfinished last line.
    pub fn finish(&mut self, emit: &mut dyn FnMut(&str)) {
        if !self.line.is_empty() {
            self.end_line(emit);
        }
        self.flush(emit);
    }

    fn end_line(&mut self, emit: &mut dyn FnMut(&str)) {
        let bytes = std::mem::take(&mut self.line);
        let text = String::from_utf8_lossy(&bytes);
        let line = text.trim_end_matches(['\n', '\r']);
        match self.rule {
            Rule::BlankLine if line.is_empty() => self.flush(emit),
            Rule::LineStart(start) if line.trim_start().starts_with(start) => {
                self.flush(emit);
                self.push(line);
            }
            _ => self.push(line),
        }
    }

    fn push(&mut self, line: &str) {
        self.block.push_str(line);
        self.block.push('\n');
    }

    fn flush(&mut self, emit: &mut dyn FnMut(&str)) {
        if !self.block.is_empty() {
            emit(&self.block);
            self.block.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rule: Rule, chunks: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut b = Blocks::new(rule);
        for c in chunks {
            b.feed(c.as_bytes(), &mut |s| out.push(s.to_string()));
        }
        b.finish(&mut |s| out.push(s.to_string()));
        out
    }

    #[test]
    fn blank_lines_end_blocks_wherever_the_chunks_fall() {
        let whole = run(Rule::BlankLine, &["a = 1\nb = 2\n\nc = 3\r\n\r\nd = 4"]);
        let cut = run(
            Rule::BlankLine,
            &["a = 1\nb", " = 2\n", "\nc = 3\r", "\n\r\nd =", " 4"],
        );
        assert_eq!(whole, ["a = 1\nb = 2\n", "c = 3\n", "d = 4\n"]);
        assert_eq!(cut, whole);
    }

    #[test]
    fn a_start_line_begins_a_block_and_the_banner_stands_alone() {
        let blocks = run(
            Rule::LineStart("Name:"),
            &["UNRAR 7\n\n", "Name: a\n Type: File\n", "Name: b\n Size: 3"],
        );
        assert_eq!(blocks.len(), 3);
        assert!(blocks.first().is_some_and(|b| b.starts_with("UNRAR")));
        assert_eq!(
            blocks.get(2).map(String::as_str),
            Some("Name: b\n Size: 3\n")
        );
    }
}
