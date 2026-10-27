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

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
