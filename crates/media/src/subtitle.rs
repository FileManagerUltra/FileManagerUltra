//! Text subtitles: SRT and WebVTT parsing into timed cues, plus matching external
//! subtitle files to a video by name. Pure data: callers read the bytes on a worker.

use fm_vfs::FmError;

/// Larger files are refused before decoding.
pub const MAX_SUBTITLE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CUES: usize = 100_000;
/// Longer lines are cut, so one hostile line cannot dominate a cue.
pub const MAX_CUE_LINE_CHARS: usize = 512;
pub const MAX_CUE_LINES: usize = 8;
/// Longest time a single cue stays on screen; longer cues are shortened.
pub const MAX_CUE_MS: u64 = 10 * 60 * 1000;
/// Delay range accepted by [`Cues::active`] callers (±10 minutes).
pub const MAX_DELAY_MS: i64 = 10 * 60 * 1000;
/// Most cues shown at the same time.
pub const MAX_ACTIVE: usize = 4;

/// File extensions accepted as external subtitles.
pub const SUBTITLE_EXTENSIONS: [&str; 4] = ["srt", "vtt", "ass", "ssa"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Plain text without markup; lines joined with `\n`.
    pub text: String,
}

/// Cues sorted by start time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cues {
    cues: Vec<Cue>,
    longest_ms: u64,
}

impl Cues {
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    pub fn as_slice(&self) -> &[Cue] {
        &self.cues
    }

    /// Cues on screen at `position_ms` with `delay_ms` applied (positive delay shows
    /// each cue later), oldest first and at most [`MAX_ACTIVE`].
    pub fn active(&self, position_ms: u64, delay_ms: i64) -> Vec<&Cue> {
        let delay = delay_ms.clamp(-MAX_DELAY_MS, MAX_DELAY_MS);
        let Some(at) = i128::from(position_ms)
            .checked_sub(i128::from(delay))
            .filter(|at| *at >= 0)
            .and_then(|at| u64::try_from(at).ok())
        else {
            return Vec::new();
        };
        let end = self.cues.partition_point(|cue| cue.start_ms <= at);
        let earliest = at.saturating_sub(self.longest_ms);
        let begin = self.cues.partition_point(|cue| cue.start_ms < earliest);
        let mut active: Vec<&Cue> = self
            .cues
            .get(begin..end)
            .unwrap_or_default()
            .iter()
            .filter(|cue| at < cue.end_ms)
            .collect();
        if active.len() > MAX_ACTIVE {
            active.drain(..active.len() - MAX_ACTIVE);
        }
        active
    }
}

fn invalid(message: &str) -> FmError {
    FmError::InvalidParams(format!("subtitle file: {message}"))
}

/// Decode subtitle bytes: UTF-8 (with or without BOM), UTF-16 with BOM, otherwise
/// Windows-1252-style single-byte text. Binary data (NUL bytes) is refused.
pub fn decode_subtitle_text(bytes: &[u8]) -> Result<String, FmError> {
    if bytes.len() > MAX_SUBTITLE_BYTES {
        return Err(invalid("larger than the 8 MiB limit"));
    }
    let utf16 = |big: bool, body: &[u8]| -> Result<String, FmError> {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if big {
                    u16::from_be_bytes(*pair)
                } else {
                    u16::from_le_bytes(*pair)
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| invalid("not valid UTF-16 text"))
    };
    let text = match bytes {
        [0xFF, 0xFE, body @ ..] => utf16(false, body)?,
        [0xFE, 0xFF, body @ ..] => utf16(true, body)?,
        [0xEF, 0xBB, 0xBF, body @ ..] => {
            String::from_utf8(body.to_vec()).map_err(|_| invalid("not valid UTF-8 text"))?
        }
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_string(),
            Err(_) => bytes.iter().map(|byte| char::from(*byte)).collect(),
        },
    };
    if text.contains('\0') {
        return Err(invalid("not a text file"));
    }
    Ok(text)
}

/// Parse SRT or WebVTT (detected by the `WEBVTT` header). Invalid blocks are skipped;
/// a file without any usable cue is an error.
pub fn parse_subtitles(bytes: &[u8]) -> Result<Cues, FmError> {
    let text = decode_subtitle_text(bytes)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let vtt = text.trim_start().starts_with("WEBVTT");
    let mut cues = Vec::new();
    let mut lines = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .peekable();
    if vtt {
        // Header block: everything up to the first blank line.
        for line in lines.by_ref() {
            if line.trim().is_empty() {
                break;
            }
        }
    }
    while lines.peek().is_some() {
        let block: Vec<&str> = lines
            .by_ref()
            .skip_while(|line| line.trim().is_empty())
            .take_while(|line| !line.trim().is_empty())
            .collect();
        if block.is_empty() {
            break;
        }
        if vtt
            && block.first().is_some_and(|first| {
                first.starts_with("NOTE") || *first == "STYLE" || *first == "REGION"
            })
        {
            continue;
        }
        let Some(timing_at) = block.iter().take(2).position(|line| line.contains("-->")) else {
            continue;
        };
        let Some((start_ms, end_ms)) = block.get(timing_at).and_then(|line| timing(line)) else {
            continue;
        };
        let body = block.get(timing_at + 1..).unwrap_or_default();
        let text = clean_cue(body);
        if text.is_empty() || end_ms <= start_ms {
            continue;
        }
        if cues.len() >= MAX_CUES {
            return Err(invalid("more cues than the 100 000 limit"));
        }
        cues.push(Cue {
            start_ms,
            end_ms: end_ms.min(start_ms.saturating_add(MAX_CUE_MS)),
            text,
        });
    }
    if cues.is_empty() {
        return Err(invalid("no subtitle cues found"));
    }
    cues.sort_by_key(|cue| cue.start_ms);
    let longest_ms = cues
        .iter()
        .map(|cue| cue.end_ms - cue.start_ms)
        .max()
        .unwrap_or(0);
    Ok(Cues { cues, longest_ms })
}

/// `00:01:02,500 --> 00:01:04.000 position:10%` → milliseconds.
fn timing(line: &str) -> Option<(u64, u64)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((timestamp(start.trim())?, timestamp(end)?))
}

/// `[hh:]mm:ss[,.]mmm`; hours may have any number of digits.
fn timestamp(text: &str) -> Option<u64> {
    let (clock, fraction) = text
        .rsplit_once([',', '.'])
        .filter(|(_, fraction)| !fraction.is_empty() && fraction.len() <= 3)?;
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let millis: u64 = format!("{fraction:0<3}").parse().ok()?;
    let parts: Vec<&str> = clock.split(':').collect();
    let number = |part: &str, max: Option<u64>| -> Option<u64> {
        if part.is_empty() || part.len() > 9 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse::<u64>()
            .ok()
            .filter(|value| max.is_none_or(|max| *value <= max))
    };
    let (hours, minutes, seconds) = match parts.as_slice() {
        [m, s] => (0, number(m, Some(59))?, number(s, Some(59))?),
        [h, m, s] => (number(h, None)?, number(m, Some(59))?, number(s, Some(59))?),
        _ => return None,
    };
    hours
        .checked_mul(3_600_000)?
        .checked_add(minutes * 60_000 + seconds * 1000 + millis)
}

/// Strip HTML-like tags and ASS override blocks, decode common entities and cap size.
fn clean_cue(lines: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in lines.iter().take(MAX_CUE_LINES) {
        let mut text = String::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '<' => {
                    // Only strip what looks like a tag, so "a < b" survives.
                    let looks_like_tag = chars.peek().is_some_and(|next| {
                        next.is_ascii_alphabetic() || matches!(next, '/' | '0'..='9')
                    });
                    if looks_like_tag && line.contains('>') {
                        for c in chars.by_ref() {
                            if c == '>' {
                                break;
                            }
                        }
                    } else {
                        text.push(c);
                    }
                }
                '{' if chars.peek() == Some(&'\\') => {
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                    }
                }
                '\\' if matches!(chars.peek(), Some('N' | 'n')) => {
                    chars.next();
                    text.push('\n');
                }
                c if c.is_control() => {}
                c => text.push(c),
            }
        }
        let text = decode_entities(&text);
        for part in text.split('\n') {
            let part = part.trim();
            if !part.is_empty() && out.len() < MAX_CUE_LINES {
                out.push(part.chars().take(MAX_CUE_LINE_CHARS).collect());
            }
        }
    }
    out.join("\n")
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&lrm;", "\u{200e}")
        .replace("&rlm;", "\u{200f}")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// How an external file relates to a video: `movie.en.forced.srt` next to `movie.mkv`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalName {
    /// Lower-case extension, one of [`SUBTITLE_EXTENSIONS`].
    pub extension: String,
    /// Dot-separated tags between the video name and the extension (`en.forced`).
    pub tags: Option<String>,
}

impl ExternalName {
    /// First tag when it looks like a language code (`en`, `nld`, `pt-BR`).
    pub fn language(&self) -> Option<String> {
        let first = self.tags.as_deref()?.split('.').next()?;
        let (base, region) = first.split_once(['-', '_']).unwrap_or((first, ""));
        let base_ok =
            (2..=3).contains(&base.len()) && base.bytes().all(|b| b.is_ascii_alphabetic());
        let region_ok = region.len() <= 4 && region.bytes().all(|b| b.is_ascii_alphanumeric());
        (base_ok && region_ok).then(|| first.to_ascii_lowercase())
    }

    /// Whether ffmpeg must convert the file before it can be parsed as text.
    pub fn needs_conversion(&self) -> bool {
        matches!(self.extension.as_str(), "ass" | "ssa")
    }
}

/// Match `file_name` against the video's file stem (case-insensitive).
pub fn match_external(video_stem: &str, file_name: &str) -> Option<ExternalName> {
    if video_stem.is_empty() || file_name.len() > 1024 {
        return None;
    }
    let (rest, extension) = file_name.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    if !SUBTITLE_EXTENSIONS.contains(&extension.as_str()) {
        return None;
    }
    let head = rest.get(..video_stem.len())?;
    if !head.eq_ignore_ascii_case(video_stem) && head != video_stem {
        return None;
    }
    let tail = rest.get(video_stem.len()..)?;
    let tags = match tail.strip_prefix('.') {
        Some(tags) if !tags.is_empty() && tags.len() <= 64 => Some(tags.to_string()),
        Some(_) => return None,
        None if tail.is_empty() => None,
        None => return None,
    };
    Some(ExternalName { extension, tags })
}

/// Normalize a language tag for comparison: ISO 639-2 codes common in containers map
/// to their two-letter form (`eng` → `en`, `nld`/`dut` → `nl`).
pub fn normalize_language(tag: &str) -> String {
    let tag = tag.trim().to_ascii_lowercase();
    let base = tag.split(['-', '_']).next().unwrap_or(&tag);
    let short = match base {
        "eng" => "en",
        "nld" | "dut" => "nl",
        "deu" | "ger" => "de",
        "fra" | "fre" => "fr",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "rus" => "ru",
        "jpn" => "ja",
        "zho" | "chi" => "zh",
        "kor" => "ko",
        "swe" => "sv",
        "nor" | "nob" => "no",
        "dan" => "da",
        "fin" => "fi",
        "pol" => "pl",
        "tur" => "tr",
        "ara" => "ar",
        other => other,
    };
    short.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hello</i> &amp; welcome\r\n\r\n2\r\n00:00:02,000 --> 00:00:04,000\r\nSecond\r\nline two\r\n\r\n";

    #[test]
    fn srt_cues_parse_with_markup_removed() {
        let cues = parse_subtitles(SRT.as_bytes()).expect("srt");
        assert_eq!(cues.len(), 2);
        let first = &cues.as_slice()[0];
        assert_eq!((first.start_ms, first.end_ms), (1000, 2500));
        assert_eq!(first.text, "Hello & welcome");
        assert_eq!(cues.as_slice()[1].text, "Second\nline two");
    }

    #[test]
    fn webvtt_skips_header_notes_and_settings() {
        let vtt = "WEBVTT - title\nKind: captions\n\nNOTE a comment\nspanning\n\nSTYLE\n::cue { color: red }\n\nintro\n01:02.500 --> 01:04.000 align:start position:10%\n<v Roger>Hi <c.loud>there</c>\n\n00:01:05.000 --> 00:01:06.000\n<00:01:05.500>timed\n";
        let cues = parse_subtitles(vtt.as_bytes()).expect("vtt");
        assert_eq!(cues.len(), 2);
        assert_eq!(cues.as_slice()[0].start_ms, 62_500);
        assert_eq!(cues.as_slice()[0].text, "Hi there");
        assert_eq!(cues.as_slice()[1].text, "timed");
    }

    #[test]
    fn delay_shifts_cues_later_and_earlier() {
        let cues = parse_subtitles(SRT.as_bytes()).expect("srt");
        let text = |at, delay| -> Vec<String> {
            cues.active(at, delay)
                .into_iter()
                .map(|cue| cue.text.clone())
                .collect()
        };
        assert!(text(900, 0).is_empty());
        assert_eq!(text(1000, 0), ["Hello & welcome"]);
        assert_eq!(text(2200, 0).len(), 2, "overlapping cues both show");
        // +500 ms: the first cue appears at 1.5 s instead of 1.0 s.
        assert!(text(1400, 500).is_empty());
        assert_eq!(text(1500, 500), ["Hello & welcome"]);
        assert_eq!(text(2999, 500).len(), 2);
        // -500 ms: shown earlier.
        assert_eq!(text(500, -500), ["Hello & welcome"]);
        // Before zero with a large delay, and an absurd delay, never panic.
        assert!(text(0, 5000).is_empty());
        assert!(text(u64::MAX, i64::MIN).is_empty());
        assert!(text(u64::MAX, i64::MAX).is_empty());
    }

    #[test]
    fn long_cues_stay_findable() {
        let srt = "00:00:00,000 --> 00:05:00,000\nlong\n\n00:00:10,000 --> 00:00:11,000\nshort\n";
        let cues = parse_subtitles(srt.as_bytes()).expect("srt");
        assert_eq!(cues.active(200_000, 0).len(), 1);
        assert_eq!(cues.active(10_500, 0).len(), 2);
    }

    #[test]
    fn malformed_files_are_errors_not_panics() {
        for bad in [
            &b""[..],
            b"\n\n\n",
            b"not a subtitle at all",
            b"1\n00:00:05,000 --> 00:00:01,000\nbackwards\n",
            b"1\n99:99:99,999 --> 00:00:01,000\nbad clock\n",
            b"1\n00:00:01,000 --> \nno end\n",
            b"1\n00:00:01,0000 --> 00:00:02,000\nlong fraction\n",
            b"1\n18446744073709551615:00:00,000 --> 18446744073709551615:00:01,000\nhuge\n",
            b"\x00\x01binary\x00",
            b"\xFF\xFE\x00\xD8",
        ] {
            assert!(parse_subtitles(bad).is_err(), "{bad:?}");
        }
        let huge = vec![b'a'; MAX_SUBTITLE_BYTES + 1];
        assert!(parse_subtitles(&huge).is_err());
    }

    #[test]
    fn hostile_cue_text_is_bounded() {
        let mut srt = String::from("00:00:01,000 --> 00:00:02,000\n");
        for _ in 0..50 {
            srt.push_str(&"x".repeat(5000));
            srt.push('\n');
        }
        let cues = parse_subtitles(srt.as_bytes()).expect("srt");
        let text = &cues.as_slice()[0].text;
        assert_eq!(text.lines().count(), MAX_CUE_LINES);
        assert!(
            text.lines()
                .all(|line| line.chars().count() <= MAX_CUE_LINE_CHARS)
        );
        let tags = "00:00:01,000 --> 00:00:02,000\n<<<<b>>a < b {\\an8}top\\Nnext\n";
        let cues = parse_subtitles(tags.as_bytes()).expect("tags");
        assert_eq!(cues.as_slice()[0].text, "<<<>a < b top\nnext");
    }

    #[test]
    fn too_many_cues_are_refused() {
        let mut srt = String::new();
        for i in 0..=MAX_CUES {
            srt.push_str(&format!("00:00:01,{:03} --> 00:00:02,000\nc\n\n", i % 1000));
        }
        assert!(srt.len() < MAX_SUBTITLE_BYTES);
        assert!(parse_subtitles(srt.as_bytes()).is_err());
    }

    #[test]
    fn encodings_decode() {
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in SRT.encode_utf16() {
            utf16.extend(unit.to_le_bytes());
        }
        assert_eq!(parse_subtitles(&utf16).expect("utf16").len(), 2);
        let latin1 = b"00:00:01,000 --> 00:00:02,000\ncaf\xe9\n";
        assert_eq!(
            parse_subtitles(latin1).expect("latin1").as_slice()[0].text,
            "café"
        );
        let bom = [&[0xEF, 0xBB, 0xBF][..], SRT.as_bytes()].concat();
        assert_eq!(parse_subtitles(&bom).expect("bom").len(), 2);
    }

    #[test]
    fn external_files_match_the_video_name() {
        let m = |name| match_external("Movie (2020)", name);
        assert_eq!(
            m("Movie (2020).srt"),
            Some(ExternalName {
                extension: "srt".into(),
                tags: None
            })
        );
        let en = m("movie (2020).en.forced.SRT").expect("tagged");
        assert_eq!(en.tags.as_deref(), Some("en.forced"));
        assert_eq!(en.language().as_deref(), Some("en"));
        assert_eq!(
            m("Movie (2020).pt-BR.vtt")
                .and_then(|n| n.language())
                .as_deref(),
            Some("pt-br")
        );
        assert_eq!(
            m("Movie (2020).Director commentary.srt").and_then(|n| n.language()),
            None
        );
        assert!(m("Movie (2020).ass").is_some_and(|n| n.needs_conversion()));
        assert_eq!(m("Movie (2020)2.srt"), None);
        assert_eq!(m("Movie (2020)..srt"), None);
        assert_eq!(m("Movie (2020).txt"), None);
        assert_eq!(m("Other.srt"), None);
        assert_eq!(match_external("", ".srt"), None);
        assert_eq!(
            match_external("é", "É.srt"),
            None,
            "only ASCII case folding"
        );
    }

    #[test]
    fn languages_normalize() {
        assert_eq!(normalize_language("eng"), "en");
        assert_eq!(normalize_language("DUT"), "nl");
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("xx"), "xx");
    }
}
