//! Tiny glob matcher for sync filters. `*` and `?` stay inside one path segment, `**`
//! crosses segments. A pattern without `/` matches the name at any depth; `dir/**` also
//! matches `dir` itself. Matching is case-sensitive on `/`-normalized relative paths and
//! runs in time proportional to pattern × path (no backtracking blow-up).

use fm_vfs::FmError;

/// Longest accepted pattern, in bytes.
pub const MAX_GLOB_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Lit(char),
    One,
    Star,
    Deep,
}

#[derive(Clone, Debug)]
pub struct Glob {
    toks: Vec<Tok>,
    /// No `/` in the pattern: match the final name only.
    name_only: bool,
    /// Pattern ends in `/**`: the directory itself matches too.
    self_too: Option<Vec<Tok>>,
}

impl Glob {
    pub fn compile(pattern: &str) -> Result<Glob, FmError> {
        if pattern.is_empty() || pattern.len() > MAX_GLOB_BYTES || pattern.contains('\0') {
            return Err(FmError::InvalidParams(format!(
                "invalid filter pattern (1 to {MAX_GLOB_BYTES} bytes): {pattern:?}"
            )));
        }
        let trimmed = pattern.trim_start_matches('/');
        // "dir/" is shorthand for "dir/**".
        let expanded;
        let src = if let Some(dir) = trimmed.strip_suffix('/') {
            expanded = format!("{dir}/**");
            expanded.as_str()
        } else {
            trimmed
        };
        let toks = tokens(src);
        let self_too = src.strip_suffix("/**").map(tokens);
        Ok(Glob {
            name_only: !src.contains('/'),
            toks,
            self_too,
        })
    }

    pub fn is_match(&self, rel: &str) -> bool {
        let subject = if self.name_only {
            rel.rsplit('/').next().unwrap_or(rel)
        } else {
            rel
        };
        if run(&self.toks, subject) {
            return true;
        }
        self.self_too.as_ref().is_some_and(|t| run(t, subject))
    }
}

fn tokens(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    while chars.peek() == Some(&'*') {
                        chars.next();
                    }
                    out.push(Tok::Deep);
                } else {
                    out.push(Tok::Star);
                }
            }
            '?' => out.push(Tok::One),
            c => out.push(Tok::Lit(c)),
        }
    }
    out
}

/// Dynamic-programming match: `reach[j]` = pattern prefix matches the text prefix.
fn run(toks: &[Tok], text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut reach = vec![false; chars.len() + 1];
    reach[0] = true;
    for tok in toks {
        let mut next = vec![false; chars.len() + 1];
        match tok {
            Tok::Lit(c) => {
                for j in 0..chars.len() {
                    next[j + 1] = reach[j] && chars[j] == *c;
                }
            }
            Tok::One => {
                for j in 0..chars.len() {
                    next[j + 1] = reach[j] && chars[j] != '/';
                }
            }
            Tok::Star | Tok::Deep => {
                next[0] = reach[0];
                for j in 0..chars.len() {
                    let crosses = matches!(tok, Tok::Deep) || chars[j] != '/';
                    next[j + 1] = reach[j + 1] || (next[j] && crosses);
                }
            }
        }
        reach = next;
    }
    reach[chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool {
        Glob::compile(p).expect("compile").is_match(s)
    }

    #[test]
    fn names_match_at_any_depth() {
        assert!(m("*.tmp", "a.tmp"));
        assert!(m("*.tmp", "x/y/a.tmp"));
        assert!(!m("*.tmp", "a.tmp.bak"));
        assert!(m("node_modules", "a/node_modules"));
    }

    #[test]
    fn paths_are_anchored_and_stars_stop_at_slashes() {
        assert!(m("build/*.o", "build/x.o"));
        assert!(!m("build/*.o", "build/sub/x.o"));
        assert!(m("build/**/*.o", "build/sub/x.o"));
        assert!(!m("build/*.o", "other/build/x.o"));
        assert!(m("a?c", "abc"));
    }

    #[test]
    fn directory_patterns_cover_the_directory_and_everything_below() {
        for p in ["build/**", "build/"] {
            assert!(m(p, "build"), "{p}");
            assert!(m(p, "build/x.o"), "{p}");
            assert!(m(p, "build/a/b"), "{p}");
            assert!(!m(p, "builder"), "{p}");
        }
    }

    #[test]
    fn hostile_patterns_are_bounded() {
        assert!(Glob::compile("").is_err());
        assert!(Glob::compile(&"a".repeat(MAX_GLOB_BYTES + 1)).is_err());
        let g = Glob::compile(&"*a".repeat(100)).expect("compile");
        assert!(!g.is_match(&"a".repeat(5_000).replace("aaaaa", "aaaab")));
    }
}
