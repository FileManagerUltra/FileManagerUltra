//! Language detection and the feature-gated grammar registry.

use tree_sitter::Language;

/// Languages the editor can colour. Detection only reports languages whose grammar
/// feature is compiled in (`Lang::available`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Toml,
    Json,
    Yaml,
    Markdown,
    Bash,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    C,
    Cpp,
    Go,
    Java,
    Html,
    Css,
    Xml,
    Sql,
    Php,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Toml => "TOML",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Markdown => "Markdown",
            Self::Bash => "Shell",
            Self::Python => "Python",
            Self::JavaScript => "JavaScript",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Xml => "XML",
            Self::Sql => "SQL",
            Self::Php => "PHP",
        }
    }

    /// The line-comment prefix used by "toggle comment"; `None` for languages without one.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Self::Rust
            | Self::JavaScript
            | Self::TypeScript
            | Self::Tsx
            | Self::C
            | Self::Cpp
            | Self::Go
            | Self::Java
            | Self::Php => Some("//"),
            Self::Toml | Self::Yaml | Self::Bash | Self::Python => Some("#"),
            Self::Sql => Some("--"),
            Self::Json | Self::Markdown | Self::Html | Self::Css | Self::Xml => None,
        }
    }

    /// Python-like languages open a block with a trailing colon.
    pub fn colon_opens_block(self) -> bool {
        matches!(self, Self::Python | Self::Yaml)
    }

    /// Whether the grammar is compiled in (SQL is coloured by a small lexer instead).
    pub fn available(self) -> bool {
        self == Self::Sql || grammar(self).is_some()
    }
}

/// Picks a language from the file name, falling back to a `#!` first line.
/// `name` without backup suffixes, so a copy is treated like the file it was made from:
/// `notes.md.bak-20261123b`, `notes.md.bak`, `notes.md.orig` and `notes.md~` all become
/// `notes.md`. Repeated suffixes are stripped a few times at most, and a name that is
/// nothing but a suffix (`.bak`) is kept.
pub fn strip_backup_suffix(name: &str) -> &str {
    let mut rest = name;
    for _ in 0..4 {
        let stripped = if let Some(s) = rest.strip_suffix('~') {
            Some(s)
        } else {
            rest.rsplit_once('.')
                .filter(|(stem, ext)| !stem.is_empty() && is_backup_ext(ext))
                .map(|(stem, _)| stem)
        };
        match stripped {
            Some(s) if !s.is_empty() && s != rest => rest = s,
            _ => break,
        }
    }
    rest
}

/// `bak` with an optional date or number (`bak2`, `bak-20261123b`, `bak_1`), and the other
/// common copy suffixes.
fn is_backup_ext(ext: &str) -> bool {
    let ext = ext.to_ascii_lowercase();
    if let Some(tail) = ext.strip_prefix("bak") {
        return tail.is_empty()
            || tail.starts_with(['-', '_'])
            || tail.bytes().all(|b| b.is_ascii_digit());
    }
    matches!(
        ext.as_str(),
        "orig" | "old" | "backup" | "save" | "sav" | "prev"
    )
}

pub fn detect(file_name: &str, first_line: &str) -> Option<Lang> {
    let lower = strip_backup_suffix(file_name).to_ascii_lowercase();
    let by_name = match lower.as_str() {
        "cargo.lock" | "poetry.lock" | "pipfile" => Some(Lang::Toml),
        ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" | "pkgbuild" => Some(Lang::Bash),
        ".babelrc" | ".prettierrc" | ".eslintrc" | "composer.lock" => Some(Lang::Json),
        _ => None,
    };
    let by_ext = || {
        let (_, ext) = lower.rsplit_once('.')?;
        Some(match ext {
            "rs" => Lang::Rust,
            "toml" => Lang::Toml,
            "json" | "jsonc" | "geojson" | "webmanifest" => Lang::Json,
            "yaml" | "yml" => Lang::Yaml,
            "md" | "markdown" | "mdx" => Lang::Markdown,
            "sh" | "bash" | "zsh" | "ksh" => Lang::Bash,
            "py" | "pyw" | "pyi" => Lang::Python,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Lang::Cpp,
            "go" => Lang::Go,
            "java" => Lang::Java,
            "html" | "htm" | "xhtml" => Lang::Html,
            "css" => Lang::Css,
            "xml" | "svg" | "xsl" | "xslt" | "plist" | "csproj" | "xaml" => Lang::Xml,
            "sql" => Lang::Sql,
            "php" | "phtml" => Lang::Php,
            _ => return None,
        })
    };
    let found = by_name.or_else(by_ext).or_else(|| shebang(first_line));
    found.filter(|l| l.available())
}

fn shebang(line: &str) -> Option<Lang> {
    let rest = line.strip_prefix("#!")?;
    let mut words = rest.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?;
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-'))?;
    }
    match prog {
        "sh" | "bash" | "zsh" | "ksh" | "dash" => Some(Lang::Bash),
        p if p.starts_with("python") => Some(Lang::Python),
        "node" | "deno" | "bun" => Some(Lang::JavaScript),
        "php" => Some(Lang::Php),
        _ => None,
    }
}

/// The compiled grammar and its highlight query source.
#[allow(unreachable_patterns)]
pub(crate) fn grammar(lang: Lang) -> Option<(Language, String)> {
    Some(match lang {
        #[cfg(feature = "lang-rust")]
        Lang::Rust => (
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-toml")]
        Lang::Toml => (
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-json")]
        Lang::Json => (
            tree_sitter_json::LANGUAGE.into(),
            tree_sitter_json::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-yaml")]
        Lang::Yaml => (
            tree_sitter_yaml::LANGUAGE.into(),
            tree_sitter_yaml::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-markdown")]
        Lang::Markdown => (
            tree_sitter_md::LANGUAGE.into(),
            tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.to_string(),
        ),
        #[cfg(feature = "lang-bash")]
        Lang::Bash => (
            tree_sitter_bash::LANGUAGE.into(),
            tree_sitter_bash::HIGHLIGHT_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-python")]
        Lang::Python => (
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-javascript")]
        Lang::JavaScript => (
            tree_sitter_javascript::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
        ),
        // The TypeScript query only adds to the JavaScript one.
        #[cfg(all(feature = "lang-typescript", feature = "lang-javascript"))]
        Lang::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
        ),
        #[cfg(all(feature = "lang-typescript", feature = "lang-javascript"))]
        Lang::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            ),
        ),
        #[cfg(feature = "lang-c")]
        Lang::C => (
            tree_sitter_c::LANGUAGE.into(),
            tree_sitter_c::HIGHLIGHT_QUERY.to_string(),
        ),
        // The C++ query only adds to the C one.
        #[cfg(all(feature = "lang-cpp", feature = "lang-c"))]
        Lang::Cpp => (
            tree_sitter_cpp::LANGUAGE.into(),
            format!(
                "{}\n{}",
                tree_sitter_c::HIGHLIGHT_QUERY,
                tree_sitter_cpp::HIGHLIGHT_QUERY
            ),
        ),
        #[cfg(feature = "lang-go")]
        Lang::Go => (
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-java")]
        Lang::Java => (
            tree_sitter_java::LANGUAGE.into(),
            tree_sitter_java::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-html")]
        Lang::Html => (
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-css")]
        Lang::Css => (
            tree_sitter_css::LANGUAGE.into(),
            tree_sitter_css::HIGHLIGHTS_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-xml")]
        Lang::Xml => (
            tree_sitter_xml::LANGUAGE_XML.into(),
            tree_sitter_xml::XML_HIGHLIGHT_QUERY.to_string(),
        ),
        #[cfg(feature = "lang-php")]
        Lang::Php => (
            tree_sitter_php::LANGUAGE_PHP.into(),
            tree_sitter_php::HIGHLIGHTS_QUERY.to_string(),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_name_and_shebang() {
        assert_eq!(detect("main.RS", ""), Some(Lang::Rust));
        assert_eq!(detect("Cargo.lock", ""), Some(Lang::Toml));
        assert_eq!(detect("run", "#!/usr/bin/env python3"), Some(Lang::Python));
        assert_eq!(detect("run", "#!/bin/bash -e"), Some(Lang::Bash));
        assert_eq!(detect("notes.txt", "hello"), None);
        assert_eq!(detect("noext", ""), None);
        assert_eq!(detect("query.sql", ""), Some(Lang::Sql));
        assert_eq!(detect("a.b.tsx", ""), Some(Lang::Tsx));
        // Backup copies are coloured like the file they were made from.
        assert_eq!(
            detect("betterstack-disk-panel.md.bak-20261123b", ""),
            Some(Lang::Markdown)
        );
        assert_eq!(detect("main.rs~", ""), Some(Lang::Rust));
        assert_eq!(detect("config.yaml.orig", ""), Some(Lang::Yaml));
    }

    #[test]
    fn backup_suffixes_are_stripped_but_names_are_never_emptied() {
        for (name, want) in [
            ("notes.md.bak", "notes.md"),
            ("notes.md.BAK", "notes.md"),
            ("notes.md.bak-20261123b", "notes.md"),
            ("notes.md.bak2", "notes.md"),
            ("notes.md.bak_1", "notes.md"),
            ("notes.md.orig", "notes.md"),
            ("notes.md.old.bak", "notes.md"),
            ("notes.md~", "notes.md"),
            ("notes.md", "notes.md"),
            ("notes.bakery", "notes.bakery"),
            ("notes.bak2x", "notes.bak2x"),
            (".bak", ".bak"),
            ("~", "~"),
            ("bak", "bak"),
            ("a.bak.bak.bak.bak.bak.bak", "a.bak.bak"),
        ] {
            assert_eq!(strip_backup_suffix(name), want, "{name}");
        }
    }
}
