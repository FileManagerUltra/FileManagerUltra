//! A small lexer that colours SQL (no tree-sitter SQL grammar with a compatible version
//! exists): keywords, strings, quoted identifiers, numbers and comments.

use crate::syntax::{Kind, TokenSpan};

const KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "and",
    "or",
    "not",
    "null",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "alter",
    "drop",
    "index",
    "view",
    "join",
    "inner",
    "left",
    "right",
    "full",
    "outer",
    "cross",
    "on",
    "as",
    "group",
    "by",
    "order",
    "having",
    "limit",
    "offset",
    "union",
    "all",
    "distinct",
    "case",
    "when",
    "then",
    "else",
    "end",
    "in",
    "is",
    "like",
    "between",
    "exists",
    "primary",
    "key",
    "foreign",
    "references",
    "default",
    "constraint",
    "unique",
    "check",
    "begin",
    "commit",
    "rollback",
    "transaction",
    "with",
    "returning",
    "asc",
    "desc",
    "if",
    "cascade",
    "truncate",
    "grant",
    "revoke",
    "replace",
    "temporary",
    "trigger",
    "procedure",
    "function",
    "returns",
    "declare",
    "using",
    "natural",
];

const TYPES: &[&str] = &[
    "int",
    "integer",
    "bigint",
    "smallint",
    "tinyint",
    "serial",
    "varchar",
    "char",
    "text",
    "boolean",
    "bool",
    "date",
    "time",
    "timestamp",
    "datetime",
    "decimal",
    "numeric",
    "float",
    "double",
    "real",
    "blob",
    "json",
    "jsonb",
    "uuid",
];

/// Spans per line, byte offsets relative to the line start. A block comment that begins
/// above the first line given is not known here.
pub(crate) fn lex(lines: &[String]) -> Vec<Vec<TokenSpan>> {
    let mut in_block = false;
    lines
        .iter()
        .map(|line| lex_line(line, &mut in_block))
        .collect()
}

fn lex_line(line: &str, in_block: &mut bool) -> Vec<TokenSpan> {
    let len = line.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut push = |start: usize, end: usize, kind: Kind| {
        out.push(TokenSpan { start, end, kind });
    };
    while i < len {
        let rest = line.get(i..).unwrap_or("");
        if *in_block {
            match rest.find("*/") {
                Some(n) => {
                    push(i, i + n + 2, Kind::Comment);
                    i += n + 2;
                    *in_block = false;
                }
                None => {
                    push(i, len, Kind::Comment);
                    break;
                }
            }
            continue;
        }
        let Some(c) = rest.chars().next() else { break };
        if rest.starts_with("--") {
            push(i, len, Kind::Comment);
            break;
        } else if rest.starts_with("/*") {
            *in_block = true;
            push(i, i + 2, Kind::Comment);
            i += 2;
        } else if c == '\'' || c == '"' || c == '`' {
            let end = rest
                .get(1..)
                .and_then(|r| r.find(c))
                .map_or(len, |n| i + 1 + n + 1);
            let kind = if c == '\'' {
                Kind::String
            } else {
                Kind::Property
            };
            push(i, end, kind);
            i = end;
        } else if c.is_ascii_digit() {
            let n = rest
                .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .unwrap_or(rest.len());
            push(i, i + n, Kind::Number);
            i += n;
        } else if c.is_alphabetic() || c == '_' {
            let n = rest
                .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .unwrap_or(rest.len());
            let word = rest.get(..n).unwrap_or("").to_ascii_lowercase();
            if KEYWORDS.contains(&word.as_str()) {
                push(i, i + n, Kind::Keyword);
            } else if TYPES.contains(&word.as_str()) {
                push(i, i + n, Kind::Type);
            }
            i += n;
        } else {
            i += c.len_utf8();
        }
    }
    out
}
