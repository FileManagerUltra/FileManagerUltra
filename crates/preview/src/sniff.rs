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
        return Kind::Binary;
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
