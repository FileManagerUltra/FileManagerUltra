//! Decide what a file is from its bytes first and its name second.

use crate::log;
use crate::structured::Format;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Image(&'static str),
    Video,
    Audio,
    Pdf,
    Log,
    Markdown,
    /// An SVG drawing: rendered, with its XML source one toggle away.
    Svg,
    Structured(Format),
    /// Plain text with a highlighter language token (extension or name), possibly empty.
    Text(String),
    Binary,
}

impl Kind {
    pub fn hint(&self) -> &'static str {
        match self {
            Kind::Video => "Video playback needs ffmpeg; metadata is shown instead.",
            Kind::Audio => "Audio metadata is not available in this build.",
            Kind::Pdf => "PDF preview shows file information only.",
            _ => "",
        }
    }
}

/// Header object GUID that starts every ASF (WMA/WMV) file.
const ASF_GUID: [u8; 16] = [
    0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C,
];

fn magic(head: &[u8]) -> Option<Kind> {
    let h = head;
    if h.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(Kind::Image("png"));
    }
    if h.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Kind::Image("jpeg"));
    }
    if h.starts_with(b"GIF87a") || h.starts_with(b"GIF89a") {
        return Some(Kind::Image("gif"));
    }
    if h.len() >= 12 && &h[0..4] == b"RIFF" {
        match &h[8..12] {
            b"WEBP" => return Some(Kind::Image("webp")),
            b"AVI " => return Some(Kind::Video),
            b"WAVE" => return Some(Kind::Audio),
            _ => {}
        }
    }
    if h.len() >= 12 && &h[4..8] == b"ftyp" {
        return Some(Kind::Video);
    }
    if h.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(Kind::Video);
    }
    if h.starts_with(b"ID3") || h.starts_with(b"fLaC") || h.starts_with(b"OggS") {
        return Some(Kind::Audio);
    }
    // Monkey's Audio, WavPack, AIFF/AIFF-C.
    if h.starts_with(b"MAC ")
        || h.starts_with(b"wvpk")
        || (h.len() >= 12 && &h[0..4] == b"FORM" && matches!(&h[8..12], b"AIFF" | b"AIFC"))
    {
        return Some(Kind::Audio);
    }
    if h.starts_with(b"%PDF") {
        return Some(Kind::Pdf);
    }
    None
}

/// Does `name` look like a log file (`x.log`, `x.log.3`, `x.log.2.gz`, `x.log-20240101`)?
pub fn is_log_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".gz").unwrap_or(&lower);
    if stem.ends_with(".log") {
        return true;
    }
    if let Some(ix) = stem.rfind(".log") {
        let rest = &stem[ix + 4..];
        return rest.len() > 1
            && matches!(rest.as_bytes()[0], b'.' | b'-')
            && rest[1..].bytes().all(|b| b.is_ascii_digit() || b == b'-');
    }
    false
}

fn is_system_log_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".gz").unwrap_or(&lower);
    [
        "syslog", "messages", "secure", "maillog", "cron", "debug", "dmesg",
    ]
    .iter()
    .any(|base| {
        stem == *base
            || stem.strip_prefix(base).is_some_and(|suffix| {
                let bytes = suffix.as_bytes();
                bytes.len() > 1
                    && matches!(bytes.first(), Some(b'.' | b'-'))
                    && bytes
                        .iter()
                        .skip(1)
                        .all(|b| b.is_ascii_digit() || *b == b'-')
            })
    })
}

fn is_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte char cut at the end of the head is still text.
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > head.len(),
    }
}

/// Whether the document's root element is `<svg`: only an XML declaration, comments, a
/// doctype and whitespace may come before it (unlike [`crate::svg::looks_like_svg`], an HTML
/// page with an inline drawing does not count).
pub fn is_svg_document(head: &[u8]) -> bool {
    let mut rest = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    loop {
        rest = rest.trim_ascii_start();
        if rest.starts_with(b"<svg") {
            return rest
                .get(4)
                .is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'));
        }
        let end: &[u8] = if rest.starts_with(b"<?") {
            b"?>"
        } else if rest.starts_with(b"<!--") {
            b"-->"
        } else if rest.starts_with(b"<!") {
            b">"
        } else {
            return false;
        };
        let Some(at) = rest.windows(end.len()).position(|w| w == end) else {
            return false;
        };
        rest = rest.get(at + end.len()..).unwrap_or_default();
    }
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Classify `path` (any separator) given the first bytes of the file.
pub fn sniff(path: &str, head: &[u8]) -> Kind {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if is_log_name(name) {
        return Kind::Log;
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return if is_system_log_name(name) {
            Kind::Log
        } else {
            Kind::Binary
        };
    }
    if let Some(k) = magic(head) {
        return k;
    }
    let ext = ext_of(name);
    // ASF holds both WMA (audio) and WMV (video); the extension tells them apart.
    if head.starts_with(&ASF_GUID) {
        return if ext == "wma" {
            Kind::Audio
        } else {
            Kind::Video
        };
    }
    if ext == "ico" && head.starts_with(&[0, 0, 1, 0]) {
        return Kind::Image("ico");
    }
    if ext == "bmp" && head.starts_with(b"BM") {
        return Kind::Image("bmp");
    }
    if !is_text(head) {
        return Kind::Binary;
    }
    if is_system_log_name(name) {
        return Kind::Log;
    }
    let norm = path.replace('\\', "/");
    if norm.contains("/var/report/") && head.first() == Some(&b'{') {
        return Kind::Log;
    }
    if ext == "svg" || is_svg_document(head) {
        return Kind::Svg;
    }
    if let Some(f) = Format::from_ext(&ext) {
        return Kind::Structured(f);
    }
    if matches!(ext.as_str(), "md" | "markdown") {
        return Kind::Markdown;
    }
    let text = String::from_utf8_lossy(head);
    if log::detect_text(&text).is_some() && !name.contains('.') {
        return Kind::Log;
    }
    let lang = if ext.is_empty() {
        name.to_string()
    } else {
        ext
    };
    Kind::Text(lang)
}

/// Content fallback for configured filename associations. Does not reapply built-in
/// filename associations that the user may have removed.
pub fn content(path: &str, head: &[u8]) -> Kind {
    if let Some(kind) = magic(head) {
        return kind;
    }
    if head.starts_with(&[0, 0, 1, 0]) {
        return Kind::Image("ico");
    }
    if head.starts_with(b"BM") {
        return Kind::Image("bmp");
    }
    if head.starts_with(&[0x1f, 0x8b]) || !is_text(head) {
        return Kind::Binary;
    }
    if is_svg_document(head) {
        return Kind::Svg;
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let text = String::from_utf8_lossy(head);
    if (!name.contains('.') && log::detect_text(&text).is_some())
        || (path.replace('\\', "/").contains("/var/report/") && head.first() == Some(&b'{'))
    {
        return Kind::Log;
    }
    let ext = ext_of(name);
    Kind::Text(if ext.is_empty() {
        name.to_string()
    } else {
        ext
    })
}

/// Associations choose a renderer, but do not turn binary input into text.
pub fn configured(path: &str, head: &[u8], association: Option<Kind>) -> Kind {
    match association {
        Some(Kind::Log) if head.starts_with(&[0x1f, 0x8b]) => Kind::Log,
        Some(
            kind @ (Kind::Log | Kind::Markdown | Kind::Svg | Kind::Structured(_) | Kind::Text(_)),
        ) => {
            if is_text(head) {
                kind
            } else {
                content(path, head)
            }
        }
        Some(kind) => kind,
        None => content(path, head),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_documents() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
        let prolog = b"\xEF\xBB\xBF<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x\">\n<svg>";
        for head in [&svg[..], &prolog[..], b"<svg>", b"  <svg\nwidth=\"1\">"] {
            assert!(is_svg_document(head), "{}", String::from_utf8_lossy(head));
        }
        for head in [
            &b"<html><body><svg/></body></html>"[..],
            b"<svgfoo/>",
            b"<?xml version=\"1.0\"?><root/>",
            b"<!-- never closed <svg",
            b"",
        ] {
            assert!(!is_svg_document(head), "{}", String::from_utf8_lossy(head));
        }
        assert_eq!(sniff("a.svg", b"<svg/>"), Kind::Svg);
        assert_eq!(sniff("a.xml", svg), Kind::Svg);
        assert_eq!(sniff("a.xml", b"<root/>"), Kind::Structured(Format::Xml));
        assert_eq!(content("drawing", svg), Kind::Svg);
        assert_eq!(configured("a.svg", svg, Some(Kind::Svg)), Kind::Svg);
        assert_eq!(
            configured("a.svg", &[0x89, b'P', b'N', b'G', 0], Some(Kind::Svg)),
            Kind::Image("png"),
            "binary data is never parsed as SVG text"
        );
    }

    #[test]
    fn bytes_beat_extension() {
        assert_eq!(
            sniff("a.txt", &[0x89, b'P', b'N', b'G', 0]),
            Kind::Image("png")
        );
        assert_eq!(sniff("a.png", b"hello"), Kind::Text("png".into()));
    }

    #[test]
    fn log_names() {
        for n in [
            "error.log",
            "access.log.1",
            "system.log.2.gz",
            "x.LOG",
            "a.log-20240101",
        ] {
            assert!(is_log_name(n), "{n}");
        }
        for n in ["catalog", "log.txt", "a.logic", "b.log.bak"] {
            assert!(!is_log_name(n), "{n}");
        }
    }

    #[test]
    fn extensionless_system_logs_and_rotations_are_logs() {
        for name in [
            "/var/log/syslog",
            "/var/log/syslog.1",
            "/var/log/messages",
            "/var/log/messages-20261027",
            "/var/log/secure",
            "/var/log/maillog",
            "/var/log/cron",
            "/var/log/debug",
            "/var/log/dmesg",
            "/var/log/auth.log",
            "/var/log/kern.log",
        ] {
            assert_eq!(
                sniff(name, b"Oct 27 13:00:00 host service: started\n"),
                Kind::Log,
                "{name}"
            );
        }
        assert_eq!(sniff("/var/log/syslog.2.gz", &[0x1f, 0x8b]), Kind::Log);
        assert_eq!(
            sniff("/var/log/messages-20261027.gz", &[0x1f, 0x8b]),
            Kind::Log
        );
        for name in [
            "/var/log/syslog.conf",
            "/var/log/messages.backup",
            "/tmp/cron.sh",
        ] {
            assert_ne!(sniff(name, b"plain text"), Kind::Log, "{name}");
        }
    }

    #[test]
    fn binary_login_databases_are_not_text_logs() {
        for name in [
            "/var/log/lastlog",
            "/var/log/wtmp",
            "/var/log/btmp",
            "/var/log/syslog",
        ] {
            assert_eq!(sniff(name, &[0, 0, 1, 2, 0, 0]), Kind::Binary, "{name}");
        }
    }

    #[test]
    fn structured_markdown_binary() {
        assert_eq!(sniff("c.json", b"{}"), Kind::Structured(Format::Json));
        assert_eq!(sniff("README.md", b"# hi"), Kind::Markdown);
        assert_eq!(sniff("blob.bin", &[1, 0, 2, 0]), Kind::Binary);
        assert_eq!(sniff("a.xyz", b"\xff\xfeh\0i\0"), Kind::Text("xyz".into()));
    }

    #[test]
    fn magento_report_is_a_log() {
        assert_eq!(sniff("/srv/var/report/123", b"{\"0\":\"x\"}"), Kind::Log);
    }

    #[test]
    fn video_and_audio() {
        let mut mp4 = vec![0, 0, 0, 24];
        mp4.extend_from_slice(b"ftypisom");
        assert_eq!(sniff("a.mp4", &mp4), Kind::Video);
        assert_eq!(sniff("a.mp3", b"ID3\x03"), Kind::Audio);
        let asf = [
            0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62,
            0xCE, 0x6C, 0, 0,
        ];
        assert_eq!(sniff("song.WMA", &asf), Kind::Audio);
        assert_eq!(sniff("clip.wmv", &asf), Kind::Video);
        assert_eq!(sniff("a.ape", b"MAC \x96\x0f"), Kind::Audio);
        assert_eq!(sniff("a.wv", b"wvpk...."), Kind::Audio);
        assert_eq!(sniff("a.aiff", b"FORM\0\0\0\x10AIFFCOMM"), Kind::Audio);
    }

    #[test]
    fn configured_associations_fall_back_to_content_not_removed_names() {
        assert_eq!(
            configured("a.log", b"plain text", None),
            Kind::Text("log".into())
        );
        assert_eq!(
            configured("syslog", b"plain text", None),
            Kind::Text("syslog".into())
        );
        assert_eq!(
            configured("Makefile", b"all:\n\techo done\n", None),
            Kind::Text("Makefile".into())
        );
        assert_eq!(configured("app.trace", b"line", Some(Kind::Log)), Kind::Log);
        assert_eq!(
            configured("app.trace.gz", &[0x1f, 0x8b], Some(Kind::Log)),
            Kind::Log
        );
        assert_eq!(
            configured("wtmp", &[0, 0, 1, 2], Some(Kind::Log)),
            Kind::Binary
        );
        assert_eq!(
            configured("a.json", &[0, 1, 0], Some(Kind::Structured(Format::Json))),
            Kind::Binary
        );
        assert_eq!(
            configured("photo.blob", &[0x89, b'P', b'N', b'G', 0], None),
            Kind::Image("png")
        );
        assert_eq!(configured("data.blob", b"ID3", None), Kind::Audio);
        assert_eq!(
            configured("data", b"[2024-05-01 10:00:00] main.INFO: started\n", None),
            Kind::Log
        );
    }
}
