//! Following links clicked in a Markdown preview: web and mail links go to the default
//! application, relative links jump to the target in a Files pane.

use fm_vfs::{FmError, VfsUri};

use crate::registry::{CommandSpec, Params};

const MAX_URL: usize = 2048;

#[derive(Debug, PartialEq)]
enum Link<'a> {
    /// An `http`, `https` or `mailto` URL.
    External(&'a str),
    /// A path relative to the document, still percent-encoded and possibly with `?`/`#` parts.
    Relative(&'a str),
}

/// Links in a document are untrusted: only plain web/mail links are launched and everything
/// else must be a path relative to the document, never another scheme.
fn classify(url: &str) -> Result<Link<'_>, FmError> {
    let refuse = |why: &str| Err(FmError::InvalidParams(why.into()));
    if url.len() > MAX_URL || url.chars().any(char::is_control) {
        return refuse("The link is too long or contains control characters");
    }
    let scheme_end = url
        .find([':', '/', '?', '#'])
        .filter(|i| url[*i..].starts_with(':'));
    if let Some(end) = scheme_end {
        let scheme = &url[..end];
        let valid = scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if valid {
            let web = ["http", "https", "mailto"]
                .iter()
                .any(|s| scheme.eq_ignore_ascii_case(s));
            if web && !url.chars().any(char::is_whitespace) {
                return Ok(Link::External(url));
            }
            return refuse("Only http, https, mailto and relative file links can be opened");
        }
    }
    if url.starts_with("//") || url.starts_with('/') || url.starts_with('\\') {
        return refuse("Only links relative to the document can be opened");
    }
    Ok(Link::Relative(url))
}

fn percent_decode(s: &str) -> Result<String, FmError> {
    let bad = || FmError::InvalidParams("The link is not valid percent-encoded text".into());
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or_else(bad)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| bad())?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| bad())
}

/// Resolve a relative link against the folder holding the document.
fn resolve(document: &VfsUri, link: &str) -> Result<VfsUri, FmError> {
    let path = link.split(['?', '#']).next().unwrap_or_default();
    if path.is_empty() {
        return Err(FmError::InvalidParams(
            "Links within the same document are not supported".into(),
        ));
    }
    let mut at = document
        .parent()
        .ok_or_else(|| FmError::InvalidParams("The document has no folder".into()))?;
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                at = at.parent().ok_or_else(|| {
                    FmError::InvalidParams("The link leaves the filesystem root".into())
                })?;
            }
            name => at = at.join(&percent_decode(name)?)?,
        }
    }
    Ok(at)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "preview.open_link",
        label: "Open Preview Link",
        menu: &["Preview", "Open Link"],
        shortcut: None,
        params: "{url: string, pane?: number}: http/https/mailto, or a path relative to the previewed file",
        enabled: |_| true,
        run: |s, params| {
            let url = params
                .as_object()
                .and_then(|p| p.get("url"))
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("url must be a string".into()))?
                .to_string();
            match classify(&url)? {
                Link::External(url) => {
                    let url = url.to_string();
                    s.jobs.submit(crate::JobLane::Local, move |_| {
                        opener::open_browser(url).map_err(|e| {
                            FmError::backend("preview", format!("Cannot open link: {e}"))
                        })
                    });
                    Ok(())
                }
                Link::Relative(link) => {
                    let pane = s.preview_pane(params)?;
                    let document = s
                        .preview_slot(pane)
                        .and_then(|slot| slot.subject.as_ref())
                        .map(|entry| entry.uri.clone())
                        .ok_or_else(|| FmError::InvalidParams("Nothing is previewed".into()))?;
                    let target = resolve(&document, link)?;
                    s.dispatch(
                        "file.reveal",
                        &serde_json::json!({"uri": target.to_string()}),
                    )
                }
            }
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_mail_and_relative_links_pass() {
        for ok in ["https://example.org/a?b=c#d", "HTTP://x.y", "mailto:a@b.c"] {
            assert!(matches!(classify(ok), Ok(Link::External(_))), "{ok}");
        }
        for ok in [
            "docs/install.md",
            "./a b.md",
            "../x#y",
            "a.md?x=1:2",
            "#top",
        ] {
            assert!(matches!(classify(ok), Ok(Link::Relative(_))), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://x",
            "https://a b",
            "https://a\nb",
            "//host/share",
            "/etc/passwd",
            "\\\\host\\share",
            "C:\\x",
            "a\nb.md",
        ] {
            assert!(classify(bad).is_err(), "{bad}");
        }
        assert!(classify(&format!("https://{}", "a".repeat(MAX_URL))).is_err());
        assert!(classify(&"a".repeat(MAX_URL + 1)).is_err());
    }

    #[test]
    fn relative_links_resolve_next_to_the_document() {
        let doc = VfsUri::parse("/home/a/README.md").expect("uri");
        let at = |link: &str| resolve(&doc, link).map(|u| u.path_str().to_string());
        assert_eq!(
            at("docs/install.md").expect("ok"),
            "/home/a/docs/install.md"
        );
        assert_eq!(at("./docs//x.md#top").expect("ok"), "/home/a/docs/x.md");
        assert_eq!(at("../b/c.md?raw=1").expect("ok"), "/home/b/c.md");
        assert_eq!(at("my%20file.md").expect("ok"), "/home/a/my file.md");
        assert_eq!(at("docs/").expect("ok"), "/home/a/docs");
        assert!(at("../../../../x").is_err(), "cannot climb above the root");
        assert!(at("#section").is_err());
        assert!(at("").is_err());
        assert!(at("%zz").is_err());
        assert!(at("%ff").is_err(), "not UTF-8");
        assert!(at("a%2Fb").is_err(), "an encoded slash is not a segment");
        assert!(at("%00").is_err());
    }
}
