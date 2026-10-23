//! Syntax highlighting into semantic [`Class`]es (colours live in the UI theme).

use std::sync::OnceLock;

use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Plain,
    Comment,
    String,
    Number,
    Keyword,
    Constant,
    Function,
    Type,
    Key,
    Tag,
    Punct,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub class: Class,
}

/// Longest line that is highlighted or kept whole.
pub const MAX_LINE_CHARS: usize = 4000;
const MAX_HIGHLIGHT_CHARS: usize = 1500;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn find<'a>(set: &'a SyntaxSet, lang: &str, first: &str) -> Option<&'a SyntaxReference> {
    let lang = lang.trim();
    let alias = match lang.to_ascii_lowercase().as_str() {
        "yml" => "yaml".to_string(),
        "htm" => "html".to_string(),
        "phtml" | "inc" | "module" => "php".to_string(),
        "svg" | "xsd" | "xsl" | "xslt" | "plist" => "xml".to_string(),
        "ts" | "tsx" | "jsx" | "mjs" | "cjs" => "js".to_string(),
        "conf" | "cfg" | "ini" | "env" | "properties" => "ini".to_string(),
        "jsonl" => "json".to_string(),
        "zsh" | "bash" | "sh" | "ksh" => "sh".to_string(),
        other => other.to_string(),
    };
    return_ext(set, &alias, first)
}

fn return_ext<'a>(set: &'a SyntaxSet, ext: &str, first: &str) -> Option<&'a SyntaxReference> {
    set.find_syntax_by_extension(ext)
        .or_else(|| set.find_syntax_by_token(ext))
        .or_else(|| set.find_syntax_by_first_line(first))
}

fn classify(stack: &ScopeStack) -> Class {
    for scope in stack.as_slice().iter().rev() {
        let s = scope.build_string();
        if s.starts_with("punctuation.definition.string")
            || s.starts_with("punctuation.definition.comment")
        {
            continue;
        }
        let c = match () {
            _ if s.starts_with("comment") => Class::Comment,
            _ if s.starts_with("constant.numeric") => Class::Number,
            _ if s.starts_with("constant") => Class::Constant,
            _ if s.starts_with("support.type.property-name")
                || s.starts_with("entity.other.attribute-name")
                || s.starts_with("meta.mapping.key")
                || s.starts_with("variable.other.key") =>
            {
                Class::Key
            }
            _ if s.starts_with("string") => Class::String,
            _ if s.starts_with("entity.name.tag") => Class::Tag,
            _ if s.starts_with("entity.name.function")
                || s.starts_with("support.function")
                || s.starts_with("variable.function") =>
            {
                Class::Function
            }
            _ if s.starts_with("entity.name")
                || s.starts_with("support.type")
                || s.starts_with("support.class") =>
            {
                Class::Type
            }
            _ if s.starts_with("keyword") || s.starts_with("storage") => Class::Keyword,
            _ if s.starts_with("punctuation") => Class::Punct,
            _ => continue,
        };
        return c;
    }
    Class::Plain
}

fn push(spans: &mut Vec<Span>, text: &str, class: Class) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.class == class
    {
        last.text.push_str(text);
        return;
    }
    spans.push(Span {
        text: text.to_string(),
        class,
    });
}

/// Cut `line` to [`MAX_LINE_CHARS`] on a char boundary.
pub fn clip(line: &str) -> String {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((ix, _)) => format!("{}…", &line[..ix]),
        None => line.to_string(),
    }
}

fn plain(text: &str) -> Vec<Vec<Span>> {
    text.lines()
        .map(|l| {
            vec![Span {
                text: clip(l),
                class: Class::Plain,
            }]
        })
        .collect()
}

/// Highlight at most `max_lines` lines of `text` for language token `lang`.
pub fn highlight(text: &str, lang: &str, max_lines: usize) -> Vec<Vec<Span>> {
    let set = syntaxes();
    let first = text.lines().next().unwrap_or("");
    let Some(syntax) = find(set, lang, first) else {
        return fallback(text, lang, max_lines);
    };
    if syntax.name == "Plain Text" {
        return fallback(text, lang, max_lines);
    }
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::new();
    for raw in text.split_inclusive('\n').take(max_lines) {
        let line = raw.trim_end_matches(['\n', '\r']);
        if line.chars().count() > MAX_HIGHLIGHT_CHARS {
            out.push(vec![Span {
                text: clip(line),
                class: Class::Plain,
            }]);
            continue;
        }
        let with_nl = format!("{line}\n");
        let Ok(ops) = state.parse_line(&with_nl, set) else {
            out.push(vec![Span {
                text: clip(line),
                class: Class::Plain,
            }]);
            state = ParseState::new(syntax);
            stack = ScopeStack::new();
            continue;
        };
        let mut spans = Vec::new();
        let mut at = 0;
        for (pos, op) in ops {
            let pos = pos.min(line.len());
            if pos > at && line.is_char_boundary(pos) && line.is_char_boundary(at) {
                push(&mut spans, &line[at..pos], classify(&stack));
                at = pos;
            }
            if stack.apply(&op).is_err() {
                stack = ScopeStack::new();
            }
        }
        if at < line.len() && line.is_char_boundary(at) {
            push(&mut spans, &line[at..], classify(&stack));
        }
        out.push(spans);
    }
    out
}

fn fallback(text: &str, lang: &str, max_lines: usize) -> Vec<Vec<Span>> {
    let toml = matches!(lang.to_ascii_lowercase().as_str(), "toml" | "lock");
    if !toml {
        return plain(text).into_iter().take(max_lines).collect();
    }
    text.lines().take(max_lines).map(toml_line).collect()
}

/// TOML is not in syntect's default pack: a small line tokenizer covers it.
fn toml_line(line: &str) -> Vec<Span> {
    let line = &clip(line)[..];
    let mut spans = Vec::new();
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        push(&mut spans, line, Class::Comment);
        return spans;
    }
    if trimmed.starts_with('[') {
        push(&mut spans, line, Class::Type);
        return spans;
    }
    let (key, rest) = match line.find('=') {
        Some(ix) => (&line[..ix], Some(&line[ix..])),
        None => (line, None),
    };
    match rest {
        None => push(&mut spans, key, Class::Plain),
        Some(rest) => {
            push(&mut spans, key, Class::Key);
            push(&mut spans, "=", Class::Punct);
            let value = &rest[1..];
            let (val, comment) = match value.find(" #") {
                Some(ix) if !value[..ix].contains('"') => (&value[..ix], Some(&value[ix..])),
                _ => (value, None),
            };
            let v = val.trim();
            let class = if v.starts_with('"') || v.starts_with('\'') {
                Class::String
            } else if v == "true" || v == "false" {
                Class::Constant
            } else if v.parse::<f64>().is_ok()
                || v.chars().next().is_some_and(|c| c.is_ascii_digit())
            {
                Class::Number
            } else {
                Class::Plain
            };
            push(&mut spans, val, class);
            if let Some(c) = comment {
                push(&mut spans, c, Class::Comment);
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(lines: &[Vec<Span>]) -> Vec<Class> {
        lines.iter().flatten().map(|s| s.class).collect()
    }

    #[test]
    fn json_has_keys_strings_and_numbers() {
        let out = highlight("{\"a\": \"b\", \"n\": 12}\n", "json", 10);
        let c = classes(&out);
        assert!(
            c.contains(&Class::Key) || c.contains(&Class::String),
            "{c:?}"
        );
        assert!(c.contains(&Class::Number), "{c:?}");
    }

    #[test]
    fn rust_comment_and_keyword() {
        let out = highlight("// hi\nfn main() {}\n", "rs", 10);
        assert_eq!(out[0][0].class, Class::Comment);
        assert!(out[1].iter().any(|s| s.class == Class::Keyword));
    }

    #[test]
    fn text_round_trips() {
        let src = "fn x() { let s = \"é\"; }\n";
        let out = highlight(src, "rs", 10);
        let joined: String = out[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, src.trim_end());
    }

    #[test]
    fn toml_fallback_and_unknown_language() {
        let out = highlight("# c\n[pkg]\nname = \"x\"\n", "toml", 10);
        assert_eq!(out[0][0].class, Class::Comment);
        assert_eq!(out[1][0].class, Class::Type);
        assert!(out[2].iter().any(|s| s.class == Class::String));
        let plain = highlight("hello\n", "zzz", 10);
        assert_eq!(plain[0][0].class, Class::Plain);
    }

    #[test]
    fn long_lines_are_clipped_and_line_cap_holds() {
        let long = "a".repeat(10_000);
        let out = highlight(&format!("{long}\nb\nc\n"), "rs", 2);
        assert_eq!(out.len(), 2);
        assert!(out[0][0].text.chars().count() <= MAX_LINE_CHARS + 1);
    }
}
