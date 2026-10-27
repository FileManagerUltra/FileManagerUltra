//! Case-insensitive file-name patterns for Select by Pattern, the quick filter and Find Files.
//!
//! A pattern list is split on `;`, `,` or `|` (`*.jpg; *.png`). `*` matches any run of
//! characters, `?` exactly one, `[abc]` / `[a-z]` / `[!abc]` a class. Matching is iterative
//! (no recursion, no regex), so hostile patterns cannot blow the stack or run away.

use fm_vfs::FmError;

/// Longest pattern text accepted.
pub const MAX_PATTERN_LEN: usize = 1024;
/// Most alternatives in one pattern list.
pub const MAX_PATTERNS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Char(char),
    Any,
    Star,
    Class {
        negated: bool,
        items: Vec<(char, char)>,
    },
}

/// One or more alternatives; a name matches when any alternative does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    alternatives: Vec<Vec<Token>>,
}

/// Whether `text` uses any wildcard (otherwise a filter is a plain substring search).
pub fn has_wildcards(text: &str) -> bool {
    text.contains(['*', '?', '['])
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn parse_one(pattern: &str) -> Vec<Token> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '*' => {
                if tokens.last() != Some(&Token::Star) {
                    tokens.push(Token::Star);
                }
            }
            '?' => tokens.push(Token::Any),
            '[' => {
                // A class runs to the next `]` (a leading `]` is literal); without one, `[`
                // is an ordinary character.
                let mut j = i;
                let negated = matches!(chars.get(j), Some('!' | '^'));
                if negated {
                    j += 1;
                }
                let start = j;
                if chars.get(j) == Some(&']') {
                    j += 1;
                }
                while chars.get(j).is_some_and(|c| *c != ']') {
                    j += 1;
                }
                if chars.get(j) != Some(&']') {
                    tokens.push(Token::Char('['));
                    continue;
                }
                let body = chars.get(start..j).unwrap_or(&[]);
                let mut items = Vec::new();
                let mut k = 0;
                while let Some(&lo) = body.get(k) {
                    if body.get(k + 1) == Some(&'-')
                        && let Some(&hi) = body.get(k + 2)
                    {
                        items.push((fold(lo).min(fold(hi)), fold(lo).max(fold(hi))));
                        k += 3;
                    } else {
                        items.push((fold(lo), fold(lo)));
                        k += 1;
                    }
                }
                tokens.push(Token::Class { negated, items });
                i = j + 1;
            }
            other => tokens.push(Token::Char(fold(other))),
        }
    }
    tokens
}

fn single(token: &Token, c: char) -> bool {
    match token {
        Token::Char(t) => *t == c,
        Token::Any => true,
        Token::Star => false,
        Token::Class { negated, items } => {
            items.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c)) != *negated
        }
    }
}

/// Classic linear-time wildcard match with one backtrack point.
fn matches_tokens(tokens: &[Token], name: &[char]) -> bool {
    let (mut t, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match tokens.get(t) {
            Some(Token::Star) => {
                star = Some((t, n));
                t += 1;
            }
            Some(token) if name.get(n).is_some_and(|c| single(token, *c)) => {
                t += 1;
                n += 1;
            }
            _ => match star {
                Some((st, sn)) => {
                    t = st + 1;
                    n = sn + 1;
                    star = Some((st, sn + 1));
                }
                None => return false,
            },
        }
    }
    tokens
        .get(t..)
        .unwrap_or(&[])
        .iter()
        .all(|t| *t == Token::Star)
}

impl Glob {
    /// Parse a pattern list. Empty alternatives are ignored; an all-empty list is an error.
    pub fn parse(text: &str) -> Result<Glob, FmError> {
        if text.len() > MAX_PATTERN_LEN {
            return Err(FmError::InvalidParams(format!(
                "pattern too long (limit {MAX_PATTERN_LEN} bytes)"
            )));
        }
        let alternatives: Vec<Vec<Token>> = text
            .split([';', ',', '|'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(parse_one)
            .collect();
        if alternatives.is_empty() {
            return Err(FmError::InvalidParams("pattern is empty".to_string()));
        }
        if alternatives.len() > MAX_PATTERNS {
            return Err(FmError::InvalidParams(format!(
                "too many patterns (limit {MAX_PATTERNS})"
            )));
        }
        Ok(Glob { alternatives })
    }

    pub fn matches(&self, name: &str) -> bool {
        let folded: Vec<char> = name.chars().map(fold).collect();
        self.alternatives
            .iter()
            .any(|tokens| matches_tokens(tokens, &folded))
    }
}

/// A name test for the quick filter and Find Files: wildcards make it a [`Glob`]; plain text
/// is a case-insensitive substring.
#[derive(Debug, Clone)]
pub enum NameTest {
    Glob(Glob),
    Contains(String),
}

impl NameTest {
    pub fn parse(text: &str) -> Result<NameTest, FmError> {
        let text = text.trim();
        if has_wildcards(text) || text.contains([';', '|']) {
            Glob::parse(text).map(NameTest::Glob)
        } else if text.len() > MAX_PATTERN_LEN {
            Err(FmError::InvalidParams("filter too long".to_string()))
        } else {
            Ok(NameTest::Contains(text.to_lowercase()))
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        match self {
            NameTest::Glob(g) => g.matches(name),
            NameTest::Contains(needle) => name.to_lowercase().contains(needle.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, n: &str) -> bool {
        Glob::parse(p).map(|g| g.matches(n)).unwrap_or(false)
    }

    #[test]
    fn wildcards_classes_and_alternatives() {
        assert!(m("*.jpg", "Photo.JPG"));
        assert!(!m("*.jpg", "photo.jpeg"));
        assert!(m("*.jpg; *.png", "a.png"));
        assert!(m("a?c", "abc") && !m("a?c", "ac"));
        assert!(m("[a-c]*", "Beta") && !m("[!a-c]*", "beta"));
        assert!(m("*", "") && m("**", "x"));
        assert!(m("[", "[") && m("x[", "x["));
        assert!(m("[]]", "]"));
        assert!(m("2026-*-27*", "2026-10-27_14-08.jpg"));
        assert!(!m("*a*b*c*d*e*f*g*h*i*j*k", &"a".repeat(5000)));
    }

    #[test]
    fn hostile_patterns_are_rejected_without_panics() {
        assert!(Glob::parse("").is_err());
        assert!(Glob::parse(" ; , ").is_err());
        assert!(Glob::parse(&"*".repeat(MAX_PATTERN_LEN + 1)).is_err());
        assert!(Glob::parse(&"a;".repeat(MAX_PATTERNS + 1)).is_err());
        for p in ["[", "[!", "[]", "[a-", "\\", "[z-a]"] {
            let _ = Glob::parse(p).map(|g| g.matches("anything"));
        }
    }

    #[test]
    fn name_tests_fall_back_to_substring() {
        let t = NameTest::parse("SHOT").expect("t");
        assert!(t.matches("Screenshot from 2022.png"));
        let t = NameTest::parse("*.png").expect("t");
        assert!(!t.matches("x.png.txt"));
        assert!(NameTest::parse("").expect("empty").matches("anything"));
    }
}
