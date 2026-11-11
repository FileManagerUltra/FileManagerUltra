//! Subtitle sources for a local video: external files next to it, and embedded text
//! tracks extracted with `ffmpeg`. Everything here does I/O; call it only on a worker.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use fm_media::{Cues, ExternalName, MAX_SUBTITLE_BYTES, match_external, parse_subtitles};
use fm_vfs::FmError;

use crate::video::{command, find_tool, missing_hint, run, safe_path};

/// Most external subtitle files listed for one video.
pub const MAX_EXTERNAL: usize = 32;
/// Directory entries examined while looking for them.
const MAX_SCAN: usize = 10_000;
/// Extracting an embedded track reads the whole container.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(120);

/// ffmpeg subtitle codecs that convert to SubRip text.
const TEXT_CODECS: [&str; 17] = [
    "subrip",
    "srt",
    "ass",
    "ssa",
    "webvtt",
    "mov_text",
    "text",
    "microdvd",
    "subviewer",
    "subviewer1",
    "sami",
    "realtext",
    "mpl2",
    "pjs",
    "vplayer",
    "jacosub",
    "stl",
];

pub fn is_text_codec(codec: &str) -> bool {
    TEXT_CODECS.contains(&codec.trim().to_ascii_lowercase().as_str())
}

/// A subtitle file found next to a video, or chosen by the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSubtitle {
    pub path: PathBuf,
    /// Menu text: tags such as "en.forced" and the format.
    pub label: String,
    /// Normalized language from the file name (`en`), if it carries one.
    pub language: Option<String>,
    /// ASS/SSA files are converted by ffmpeg (styling is dropped).
    pub needs_conversion: bool,
}

impl ExternalSubtitle {
    fn new(path: PathBuf, name: &ExternalName) -> Self {
        let format = name.extension.to_ascii_uppercase();
        let label = match &name.tags {
            Some(tags) => format!("{} · {format}", clean(tags)),
            None => format!("External · {format}"),
        };
        Self {
            path,
            label,
            language: name
                .language()
                .map(|lang| fm_media::normalize_language(&lang)),
            needs_conversion: name.needs_conversion(),
        }
    }

    /// An explicitly chosen file; its name need not match the video.
    pub fn chosen(path: &Path) -> Result<Self, FmError> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("choose a subtitle file"))?;
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        let matched = match_external(stem, name)
            .ok_or_else(|| invalid("subtitles must be .srt, .vtt, .ass or .ssa files"))?;
        let mut sub = Self::new(path.to_path_buf(), &matched);
        sub.label = format!(
            "{} · {}",
            clean(name),
            matched.extension.to_ascii_uppercase()
        );
        Ok(sub)
    }
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(48).collect()
}

fn invalid(message: &str) -> FmError {
    FmError::InvalidParams(format!("subtitle file: {message}"))
}

/// Subtitle files beside `video` whose names start with the video's file stem, sorted
/// by name. A missing or unreadable folder yields an empty list.
pub fn discover(video: &Path) -> Vec<ExternalSubtitle> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str()))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<ExternalSubtitle> = entries
        .take(MAX_SCAN)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let matched = match_external(stem, name)?;
            // Symlinks and folders named like subtitles are not followed.
            entry.file_type().ok().filter(|t| t.is_file())?;
            Some(ExternalSubtitle::new(entry.path(), &matched))
        })
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found.truncate(MAX_EXTERNAL);
    found
}

/// Read and parse an external subtitle file, converting ASS/SSA with ffmpeg.
pub fn load_external(sub: &ExternalSubtitle, cancelled: &AtomicBool) -> Result<Cues, FmError> {
    let path = sub
        .path
        .to_str()
        .filter(|path| safe_path(path))
        .ok_or_else(|| invalid("choose a safe local path"))?;
    if sub.needs_conversion {
        return convert(path, None, cancelled);
    }
    let meta = std::fs::symlink_metadata(&sub.path).map_err(FmError::Io)?;
    if !meta.is_file() {
        return Err(invalid("not a regular file"));
    }
    if meta.len() > MAX_SUBTITLE_BYTES as u64 {
        return Err(invalid("larger than the 8 MiB limit"));
    }
    let bytes = std::fs::read(&sub.path).map_err(FmError::Io)?;
    parse_subtitles(&bytes)
}

/// Extract embedded subtitle stream `index` (among subtitle streams) of a local video.
pub fn extract_embedded(
    video: &str,
    index: usize,
    cancelled: &AtomicBool,
) -> Result<Cues, FmError> {
    if !safe_path(video) {
        return Err(invalid("choose a safe local video path"));
    }
    convert(video, Some(index), cancelled)
}

/// `ffmpeg -i <path> [-map 0:s:N] -f srt -`, capped and cancellable.
fn convert(path: &str, stream: Option<usize>, cancelled: &AtomicBool) -> Result<Cues, FmError> {
    let fail = |message: &str| FmError::backend("subtitles", message);
    let ffmpeg = find_tool("ffmpeg").ok_or_else(|| fail(missing_hint()))?;
    let mut c = command(&ffmpeg);
    c.args([
        "-nostdin",
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-i",
    ])
    .arg(path);
    if let Some(index) = stream {
        c.arg("-map").arg(format!("0:s:{index}"));
    }
    c.args(["-f", "srt", "-"]);
    let raw = run(
        c,
        EXTRACT_TIMEOUT,
        MAX_SUBTITLE_BYTES as u64 + 1,
        Some(cancelled),
    )
    .ok_or_else(|| fail("ffmpeg could not read these subtitles."))?;
    parse_subtitles(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_codecs_are_recognized() {
        assert!(is_text_codec("subrip"));
        assert!(is_text_codec("ASS"));
        assert!(is_text_codec("mov_text"));
        assert!(!is_text_codec("hdmv_pgs_subtitle"));
        assert!(!is_text_codec("dvd_subtitle"));
    }

    #[test]
    fn discovery_lists_matching_files_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let video = dir.path().join("Film.mkv");
        for name in [
            "Film.mkv",
            "Film.srt",
            "film.en.srt",
            "Film.nl.forced.vtt",
            "Film.ass",
            "Film.txt",
            "Film2.srt",
            "Other.srt",
        ] {
            std::fs::write(dir.path().join(name), b"x").expect("write");
        }
        std::fs::create_dir(dir.path().join("Film.de.srt")).expect("dir");
        let found = discover(&video);
        let names: Vec<_> = found
            .iter()
            .filter_map(|sub| sub.path.file_name()?.to_str().map(str::to_string))
            .collect();
        assert_eq!(
            names,
            ["Film.ass", "Film.nl.forced.vtt", "Film.srt", "film.en.srt"]
        );
        let en = found
            .iter()
            .find(|s| s.label.starts_with("en"))
            .expect("en");
        assert_eq!(en.language.as_deref(), Some("en"));
        assert!(found[0].needs_conversion);
        assert_eq!(found[2].label, "External · SRT");
        assert!(discover(&dir.path().join("missing/x.mkv")).is_empty());
    }

    #[test]
    fn real_embedded_text_track_is_probed_and_extracted() {
        let (Some(ffmpeg), Some(_)) = (find_tool("ffmpeg"), find_tool("ffprobe")) else {
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        let srt = dir.path().join("in.srt");
        std::fs::write(
            &srt,
            "1\n00:00:00,500 --> 00:00:01,500\n<i>Embedded</i> line\n",
        )
        .expect("write");
        let video = dir.path().join("t.mkv");
        let made = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=size=64x48:rate=5:duration=2")
            .arg("-i")
            .arg(&srt)
            .args(["-map", "0:v", "-map", "1:s", "-c:s", "srt"])
            .args(["-metadata:s:s:0", "language=eng"])
            .arg(&video)
            .status();
        if !made.is_ok_and(|status| status.success()) {
            return; // ffmpeg without a video encoder for Matroska
        }
        let uri = fm_vfs::VfsUri::parse(&video.to_string_lossy()).expect("uri");
        let cancelled = AtomicBool::new(false);
        let info = crate::video::playback_info(&uri, &cancelled).expect("probe");
        let track = info.tracks.subtitles.first().expect("subtitle track");
        assert!(track.text);
        assert_eq!(track.language.as_deref(), Some("eng"));
        let path = video.to_str().expect("utf8 path");
        let cues = extract_embedded(path, 0, &cancelled).expect("extract");
        assert_eq!(cues.as_slice()[0].text, "Embedded line");
        assert_eq!(cues.as_slice()[0].start_ms, 500);
        assert!(
            extract_embedded(path, 3, &cancelled).is_err(),
            "missing stream"
        );
    }

    #[test]
    fn external_files_load_and_reject_bad_input() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = dir.path().join("a.srt");
        std::fs::write(&good, "00:00:01,000 --> 00:00:02,000\nhi\n").expect("write");
        let cancelled = AtomicBool::new(false);
        let sub = ExternalSubtitle::chosen(&good).expect("chosen");
        assert_eq!(load_external(&sub, &cancelled).expect("cues").len(), 1);
        let empty = dir.path().join("b.srt");
        std::fs::write(&empty, "").expect("write");
        let sub = ExternalSubtitle::chosen(&empty).expect("chosen");
        assert!(load_external(&sub, &cancelled).is_err());
        let big = dir.path().join("c.vtt");
        std::fs::write(&big, vec![b'a'; MAX_SUBTITLE_BYTES + 1]).expect("write");
        let sub = ExternalSubtitle::chosen(&big).expect("chosen");
        assert!(load_external(&sub, &cancelled).is_err());
        assert!(ExternalSubtitle::chosen(&dir.path().join("x.txt")).is_err());
        let missing = ExternalSubtitle::chosen(&dir.path().join("gone.srt")).expect("chosen");
        assert!(load_external(&missing, &cancelled).is_err());
        assert!(extract_embedded("relative.mkv", 0, &cancelled).is_err());
        assert!(extract_embedded("-i", 0, &cancelled).is_err());
    }
}
