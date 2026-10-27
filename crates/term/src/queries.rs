//! Terminal queries that need an answer.
//!
//! `vt100` parses output but never replies. Some hosts wait for replies: Windows ConPTY
//! (created with "inherit cursor") sends a cursor-position request and shows nothing until
//! it is answered, and shells/readline probe the cursor or device attributes too. This
//! scanner spots the requests in the output stream (state survives across chunks) so the
//! emulator can answer them in order.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Esc,
    Csi,
    /// A CSI too long to be a query: skip to its final byte.
    CsiSkip,
}

/// A request found in the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    /// `CSI 6 n`: report the cursor position.
    CursorPosition,
    /// `CSI 5 n`: report device status ("OK").
    Status,
    /// `CSI c` / `CSI 0 c`: primary device attributes.
    PrimaryAttributes,
    /// `CSI > c` / `CSI > 0 c`: secondary device attributes.
    SecondaryAttributes,
}

const ESC: u8 = 0x1b;
const MAX_PARAMS: usize = 8;

#[derive(Clone, Debug, Default)]
pub struct Queries {
    state: State,
    params: Vec<u8>,
}

impl Queries {
    /// Advance over one byte; returns the query that this byte completes, if any.
    pub fn step(&mut self, b: u8) -> Option<Query> {
        match self.state {
            State::Ground => {
                if b == ESC {
                    self.state = State::Esc;
                }
                None
            }
            State::Esc => {
                self.state = match b {
                    b'[' => {
                        self.params.clear();
                        State::Csi
                    }
                    ESC => State::Esc,
                    _ => State::Ground,
                };
                None
            }
            State::Csi | State::CsiSkip => {
                if b == ESC {
                    self.state = State::Esc;
                    return None;
                }
                if !(0x40..=0x7e).contains(&b) {
                    if self.state == State::Csi {
                        if self.params.len() < MAX_PARAMS {
                            self.params.push(b);
                        } else {
                            self.state = State::CsiSkip;
                        }
                    }
                    return None;
                }
                let skip = self.state == State::CsiSkip;
                self.state = State::Ground;
                if skip {
                    return None;
                }
                match (self.params.as_slice(), b) {
                    (b"6", b'n') => Some(Query::CursorPosition),
                    (b"5", b'n') => Some(Query::Status),
                    (b"" | b"0", b'c') => Some(Query::PrimaryAttributes),
                    (b">" | b">0", b'c') => Some(Query::SecondaryAttributes),
                    _ => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(q: &mut Queries, bytes: &[u8]) -> Vec<(usize, Query)> {
        bytes
            .iter()
            .enumerate()
            .filter_map(|(i, b)| q.step(*b).map(|x| (i, x)))
            .collect()
    }

    #[test]
    fn finds_queries_at_their_final_byte() {
        let mut q = Queries::default();
        let found = scan(&mut q, b"hi\x1b[6nx\x1b[5n\x1b[c\x1b[>c\x1b[0c");
        assert_eq!(
            found,
            vec![
                (5, Query::CursorPosition),
                (10, Query::Status),
                (13, Query::PrimaryAttributes),
                (17, Query::SecondaryAttributes),
                (21, Query::PrimaryAttributes),
            ]
        );
    }

    #[test]
    fn ignores_other_sequences_and_survives_chunks() {
        let mut q = Queries::default();
        assert!(scan(&mut q, b"\x1b[2J\x1b[1;1H\x1b[16n\x1b[?6n[6n").is_empty());
        assert!(scan(&mut q, b"\x1b[").is_empty());
        assert_eq!(scan(&mut q, b"6n"), vec![(1, Query::CursorPosition)]);
        // Overlong parameters never match and never grow without bound.
        let mut long = b"\x1b[".to_vec();
        long.extend(std::iter::repeat_n(b'6', 10_000));
        long.push(b'n');
        assert!(scan(&mut q, &long).is_empty());
        assert!(q.params.len() <= MAX_PARAMS);
        assert_eq!(scan(&mut q, b"\x1b[6n"), vec![(3, Query::CursorPosition)]);
    }
}
