//! Video cards: metadata from `ffprobe` and a poster frame from `ffmpeg`, both run as
//! subprocesses (no link-time dependency, a decoder crash cannot take the app down).
//! Without the tools, or for non-local files, a metadata card with a hint is returned.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fm_vfs::Entry;

use crate::platform::{Os, tool_candidates};
use crate::{ImageDoc, Limits, MetaDoc, Preview, base_meta, image_prev};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const POSTER_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_BYTES: u64 = 1 << 20;
const POSTER_BYTES: u64 = 32 << 20;

/// First existing `tool` from the sidecar / `PATH` / package-manager candidates.
pub fn find_tool(tool: &str) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let path_env = std::env::var("PATH").unwrap_or_default();
    tool_candidates(Os::current(), tool, exe_dir.as_deref(), &path_env)
        .into_iter()
        .find(|p| p.is_file())
}

/// A path that can never be read as an option or smuggle a protocol.
pub(crate) fn safe_path(path: &str) -> bool {
    let absolute =
        path.starts_with('/') || path.get(1..3) == Some(":\\") || path.starts_with("\\\\");
    absolute && !path.contains('\0')
}

pub(crate) fn command(tool: &Path) -> Command {
    let mut c = Command::new(tool);
    c.stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Run `cmd`, keep at most `max` bytes of stdout, and kill it after `timeout`.
fn run(
    mut cmd: Command,
    timeout: Duration,
    max: u64,
    cancelled: Option<&AtomicBool>,
) -> Option<Vec<u8>> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return None;
    }
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(max).read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None)
                if Instant::now() < deadline
                    && !cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) =>
            {
                std::thread::sleep(Duration::from_millis(15))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let buf = reader.join().ok()?;
    ok.then_some(buf)
}

struct Probe {
    rows: Vec<(String, String)>,
    duration: Option<f64>,
    /// Width, height and frame rate of the first video stream.
    video: Option<(u32, u32, f64)>,
    audio: Option<fm_media::AudioFormat>,
    audio_delay: f64,
    tracks: crate::Tracks,
}

impl Probe {
    fn playback_info(&self, path: &str) -> Option<crate::VideoInfo> {
        let (width, height, fps) = self.video?;
        Some(crate::VideoInfo {
            path: path.to_string(),
            width,
            height,
            fps,
            duration: self.duration.unwrap_or(0.0),
            audio: self.audio,
            audio_delay: self.audio_delay,
            tracks: crate::Tracks {
                selected_audio: self
                    .tracks
                    .audio
                    .iter()
                    .position(|track| track.format.is_some())
                    .unwrap_or(0),
                ..self.tracks.clone()
            },
        })
    }
}

/// Probe local video for playback without decoding a poster. Call only on a worker.
pub fn playback_info(
    uri: &fm_vfs::VfsUri,
    cancelled: &AtomicBool,
) -> Result<crate::VideoInfo, fm_vfs::FmError> {
    let fail = |message| fm_vfs::FmError::backend("video playback", message);
    let path = uri
        .local_path_string()
        .filter(|path| safe_path(path))
        .ok_or_else(|| fail("Choose a safe local video path."))?;
    let ffprobe = find_tool("ffprobe")
        .ok_or_else(|| fail("Install ffmpeg and ffprobe to play this video."))?;
    if find_tool("ffmpeg").is_none() {
        return Err(fail("Install ffmpeg and ffprobe to play this video."));
    }
    probe(&ffprobe, &path, Some(cancelled))
        .and_then(|probe| probe.playback_info(&path))
        .ok_or_else(|| fail("ffprobe could not read a supported video stream."))
}

/// Embedded chapters (MP4/m4b, MP3 CHAP, Matroska) of a local file, via `ffprobe`.
/// A file without chapters gives an empty list. Call only on a worker.
pub fn chapters(
    uri: &fm_vfs::VfsUri,
    cancelled: &AtomicBool,
) -> Result<Vec<fm_media::Chapter>, fm_vfs::FmError> {
    let fail = |message| fm_vfs::FmError::backend("chapters", message);
    let path = uri
        .local_path_string()
        .filter(|path| safe_path(path))
        .ok_or_else(|| fail("Choose a safe local path."))?;
    let Some(ffprobe) = find_tool("ffprobe") else {
        return Ok(Vec::new());
    };
    let mut c = command(&ffprobe);
    c.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-print_format",
        "json",
        "-show_chapters",
        "-i",
    ])
    .arg(&path);
    let raw = run(c, PROBE_TIMEOUT, PROBE_BYTES, Some(cancelled))
        .ok_or_else(|| fail("ffprobe could not read the chapters."))?;
    Ok(parse_chapters(&raw))
}

fn parse_chapters(raw: &[u8]) -> Vec<fm_media::Chapter> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let seconds = |o: &serde_json::Value, k: &str| {
        o.get(k)
            .and_then(|x| x.as_str())
            .and_then(|x| x.parse::<f64>().ok())
    };
    let mut chapters: Vec<fm_media::Chapter> = v
        .get("chapters")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .take(fm_media::MAX_CHAPTERS)
        .enumerate()
        .filter_map(|(i, c)| {
            let title = c
                .get("tags")
                .and_then(|t| t.get("title"))
                .and_then(|t| t.as_str())
                .map_or_else(|| format!("Chapter {}", i + 1), str::to_string);
            fm_media::Chapter::new(&title, seconds(c, "start_time")?, seconds(c, "end_time")?)
        })
        .collect();
    chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
    chapters
}

fn probe(ffprobe: &Path, path: &str, cancelled: Option<&AtomicBool>) -> Option<Probe> {
    let mut c = command(ffprobe);
    c.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        "-i",
    ])
    .arg(path);
    let raw = run(c, PROBE_TIMEOUT, PROBE_BYTES, cancelled)?;
    parse_probe(&raw)
}

fn parse_probe(raw: &[u8]) -> Option<Probe> {
    let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
    let text =
        |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let mut rows = Vec::new();
    let mut duration = None;
    let mut video = None;
    let mut audio = None;
    let mut audio_delay = 0.0;
    let mut tracks = crate::Tracks::default();
    let timestamp = |value: &serde_json::Value| {
        value
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let start = v
        .get("format")
        .and_then(|v| v.get("start_time"))
        .and_then(timestamp)
        .unwrap_or(0.0);
    if let Some(f) = v.get("format") {
        if let Some(n) = text(f, "format_long_name") {
            rows.push(("Container".to_string(), n));
        }
        duration = text(f, "duration")
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d >= 0.0);
        if let Some(d) = duration {
            rows.push(("Duration".to_string(), clock(d)));
        }
        if let Some(b) = text(f, "bit_rate").and_then(|b| b.parse::<u64>().ok()) {
            rows.push(("Bitrate".to_string(), format!("{} kb/s", b / 1000)));
        }
    }
    let streams = v.get("streams").and_then(|s| s.as_array())?;
    for s in streams.iter().take(16) {
        let codec = text(s, "codec_name").unwrap_or_else(|| "?".into());
        match text(s, "codec_type").as_deref() {
            Some("video") => {
                let size = match (
                    s.get("width").and_then(|x| x.as_u64()),
                    s.get("height").and_then(|x| x.as_u64()),
                ) {
                    (Some(w), Some(h)) => format!(" · {w}×{h}"),
                    _ => String::new(),
                };
                let rate = text(s, "avg_frame_rate").and_then(|r| frame_rate(&r));
                if video.is_none()
                    && let (Some(w), Some(h)) = (
                        s.get("width").and_then(|x| x.as_u64()),
                        s.get("height").and_then(|x| x.as_u64()),
                    )
                    && let (Ok(w), Ok(h)) = (u32::try_from(w), u32::try_from(h))
                    && (1..=16_384).contains(&w)
                    && (1..=16_384).contains(&h)
                {
                    video = Some((w, h, rate.unwrap_or(25.0).min(120.0)));
                }
                let fps = rate.map(|f| format!(" · {f:.2} fps")).unwrap_or_default();
                rows.push(("Video".to_string(), format!("{codec}{size}{fps}")));
            }
            Some("audio") => {
                let format = text(s, "sample_rate")
                    .and_then(|rate| rate.parse::<u32>().ok())
                    .zip(s.get("channels").and_then(|v| v.as_u64()))
                    .and_then(|(rate, channels)| {
                        usize::try_from(channels)
                            .ok()
                            .and_then(|channels| fm_media::AudioFormat::new(rate, channels).ok())
                    });
                let delay = s
                    .get("start_time")
                    .and_then(timestamp)
                    .map_or(0.0, |time| (time - start).max(0.0));
                if audio.is_none() && format.is_some() {
                    audio = format;
                    audio_delay = delay;
                }
                if tracks.audio.len() < crate::MAX_TRACKS {
                    let channels = s.get("channels").and_then(|x| x.as_u64());
                    tracks.audio.push(crate::AudioTrack {
                        label: track_label(s, &codec, channels),
                        format,
                        delay,
                    });
                }
                let ch = s
                    .get("channels")
                    .and_then(|x| x.as_u64())
                    .map(|c| format!(" · {c} ch"))
                    .unwrap_or_default();
                let hz = text(s, "sample_rate")
                    .map(|r| format!(" · {r} Hz"))
                    .unwrap_or_default();
                rows.push(("Audio".to_string(), format!("{codec}{ch}{hz}")));
            }
            Some("subtitle") => {
                if tracks.subtitles.len() < crate::MAX_TRACKS {
                    tracks.subtitles.push(crate::SubtitleTrack {
                        label: track_label(s, &codec, None),
                    });
                }
                rows.push(("Subtitles".to_string(), codec));
            }
            _ => {}
        }
    }
    Some(Probe {
        tracks,
        rows,
        duration,
        video,
        audio,
        audio_delay,
    })
}

/// Tag text is hostile: drop control characters and cap the length before it is displayed.
fn clean_tag(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_string()
}

fn track_label(stream: &serde_json::Value, codec: &str, channels: Option<u64>) -> String {
    let tag = |name: &str| {
        stream
            .get("tags")
            .and_then(|tags| tags.get(name))
            .and_then(|value| value.as_str())
            .map(clean_tag)
            .filter(|value| !value.is_empty())
    };
    let mut parts: Vec<String> = [tag("title"), tag("language")]
        .into_iter()
        .flatten()
        .collect();
    let mut detail = clean_tag(codec);
    if let Some(channels) = channels {
        detail.push_str(&format!(" {channels} ch"));
    }
    parts.push(detail);
    parts.join(" · ")
}

fn frame_rate(r: &str) -> Option<f64> {
    let (n, d) = r.split_once('/')?;
    let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then_some(n / d)
}

fn clock(secs: f64) -> String {
    let t = secs.round() as u64;
    format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
}

fn poster(
    ffmpeg: &Path,
    path: &str,
    at: f64,
    width: u32,
    entry: &Entry,
    limits: &Limits,
) -> Option<ImageDoc> {
    let mut c = command(ffmpeg);
    c.args([
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-ss",
    ])
    .arg(format!("{at:.3}"))
    .arg("-i")
    .arg(path)
    .args(["-frames:v", "1", "-vf"])
    .arg(format!("scale='min({width},iw)':-2"))
    .args(["-f", "image2pipe", "-c:v", "png", "-"]);
    let png = run(c, POSTER_TIMEOUT, POSTER_BYTES, None)?;
    match image_prev::decode(entry, &png, limits) {
        Preview::Image(doc) => Some(doc),
        _ => None,
    }
}

/// A small still of a local video for directory thumbnails; `None` without ffmpeg.
pub fn thumbnail(entry: &Entry, edge: u32) -> Option<ImageDoc> {
    let native = entry.uri.local_path_string()?;
    let path = native.as_str();
    if !safe_path(path) {
        return None;
    }
    static TOOLS: std::sync::OnceLock<(Option<PathBuf>, Option<PathBuf>)> =
        std::sync::OnceLock::new();
    let (ffmpeg, ffprobe) = TOOLS.get_or_init(|| (find_tool("ffmpeg"), find_tool("ffprobe")));
    let ffmpeg = ffmpeg.as_ref()?;
    let duration = ffprobe
        .as_ref()
        .and_then(|p| probe(p, path, None))
        .and_then(|p| p.duration);
    let at = duration.map_or(0.0, |d| (d * 0.1).min(10.0));
    let limits = Limits {
        image_edge: edge,
        ..Limits::default()
    };
    poster(ffmpeg, path, at, edge, entry, &limits)
}

pub fn render(entry: &Entry, limits: &Limits) -> Preview {
    let mut rows = base_meta(entry);
    let native = entry.uri.local_path_string();
    let Some(path) = native.as_deref() else {
        return Preview::Meta(MetaDoc {
            rows,
            hint: Some("Video previews need a local file; copy it locally to preview.".into()),
            poster: None,
            video: None,
            audio: None,
        });
    };
    let (Some(ffprobe), Some(ffmpeg)) = (find_tool("ffprobe"), find_tool("ffmpeg")) else {
        return Preview::Meta(MetaDoc {
            rows,
            hint: Some("Install ffmpeg (ffprobe + ffmpeg on PATH) to see video details and a poster frame.".into()),
            poster: None,
            video: None,
            audio: None,
        });
    };
    if !safe_path(path) {
        return Preview::Meta(MetaDoc {
            rows,
            hint: Some("This path cannot be passed safely to ffmpeg.".into()),
            poster: None,
            video: None,
            audio: None,
        });
    }
    let info = probe(&ffprobe, path, None);
    let at = info
        .as_ref()
        .and_then(|p| p.duration)
        .map_or(0.0, |d| (d * 0.1).min(10.0));
    let hint = match &info {
        Some(p) => {
            rows.extend(p.rows.iter().cloned());
            None
        }
        None => Some("ffprobe could not read this file.".to_string()),
    };
    let video = info.as_ref().and_then(|p| p.playback_info(path));
    Preview::Meta(MetaDoc {
        rows,
        hint,
        poster: poster(&ffmpeg, path, at, 1280, entry, limits).map(Box::new),
        video,
        audio: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn chapters_parse_sorted_and_skip_garbage() {
        let raw = br#"{"chapters":[{"start_time":"10.0","end_time":"20.0","tags":{"title":"B"}},{"start_time":"0.0","end_time":"10.0"},{"start_time":"x","end_time":"1"},{"start_time":"5","end_time":"1"}]}"#;
        let c = super::parse_chapters(raw);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].title, "Chapter 2");
        assert_eq!(c[1].title, "B");
        assert!(super::parse_chapters(b"nope").is_empty());
    }

    use super::*;

    #[test]
    fn rejects_option_like_and_relative_paths() {
        assert!(safe_path("/home/a/b.mp4"));
        assert!(safe_path("C:\\v\\a.mp4"));
        assert!(safe_path("\\\\srv\\share\\a.mp4"));
        assert!(!safe_path("-i"));
        assert!(!safe_path("movie.mp4"));
        assert!(!safe_path("/a\0b"));
        assert!(!safe_path("concat:/a|/b") && !safe_path("http://x/y"));
    }

    #[test]
    fn probe_json_becomes_rows_and_survives_garbage() {
        let j = br#"{"format":{"format_long_name":"QuickTime","duration":"3725.5","bit_rate":"2500000"},
            "streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30000/1001"},
                       {"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"}]}"#;
        let p = parse_probe(j).expect("probe");
        assert_eq!(p.duration, Some(3725.5));
        assert_eq!(
            p.audio,
            Some(fm_media::AudioFormat::new(48_000, 2).expect("audio"))
        );
        let get = |k: &str| p.rows.iter().find(|r| r.0 == k).map(|r| r.1.clone());
        assert_eq!(get("Duration").as_deref(), Some("1:02:06"));
        assert_eq!(
            get("Video").as_deref(),
            Some("h264 · 1920×1080 · 29.97 fps")
        );
        assert_eq!(get("Audio").as_deref(), Some("aac · 2 ch · 48000 Hz"));
        assert!(parse_probe(b"not json").is_none());
        assert!(parse_probe(br#"{"format":{"duration":"-1"}}"#).is_none());
        assert_eq!(frame_rate("0/0"), None);
    }

    #[test]
    fn probe_preserves_the_audio_start_relative_to_the_container() {
        let p = parse_probe(
            br#"{"format":{"start_time":"2.0"},"streams":[
            {"codec_type":"audio","channels":2,"sample_rate":"44100","start_time":"2.4"}]}"#,
        )
        .expect("probe");
        assert!((p.audio_delay - 0.4).abs() < 0.0001);
    }

    #[test]
    fn probe_lists_every_audio_and_subtitle_stream_with_sanitized_labels() {
        let p = parse_probe(
            br#"{"format":{"duration":"10"},"streams":[
            {"codec_type":"video","codec_name":"h264","width":640,"height":360,"r_frame_rate":"25/1"},
            {"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000","tags":{"language":"eng"}},
            {"codec_type":"audio","codec_name":"ac3","channels":6,"sample_rate":"48000","tags":{"title":"Comm\u0007entary"}},
            {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"nld"}}]}"#,
        )
        .expect("probe");
        assert_eq!(p.tracks.audio.len(), 2);
        assert_eq!(p.tracks.subtitles.len(), 1);
        assert!(p.tracks.audio.iter().all(|t| t.format.is_some()));
        assert!(
            p.tracks
                .audio
                .iter()
                .all(|t| !t.label.chars().any(char::is_control))
        );
    }

    #[test]
    fn a_hanging_tool_is_killed() {
        if cfg!(windows) {
            return;
        }
        let mut c = Command::new("sleep");
        c.arg("30").stdout(Stdio::piped());
        let t = Instant::now();
        assert!(run(c, Duration::from_millis(100), 10, None).is_none());
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cancelled_probe_kills_the_running_tool_without_waiting_for_its_timeout() {
        if cfg!(windows) {
            return;
        }
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Release);
        });
        let mut command = Command::new("sleep");
        command.arg("30").stdout(Stdio::piped());
        let start = Instant::now();
        assert!(run(command, Duration::from_secs(30), 10, Some(&cancelled)).is_none());
        cancel.join().expect("cancel thread");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn real_video_gets_rows_and_a_poster() {
        let (Some(ffmpeg), Some(_)) = (find_tool("ffmpeg"), find_tool("ffprobe")) else {
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("t.mp4");
        let made = Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10:duration=2",
            ])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&file)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // ffmpeg without the h264 encoder
        }
        let uri = fm_vfs::VfsUri::parse(&file.to_string_lossy()).expect("uri");
        let entry = Entry::new(uri, fm_vfs::EntryKind::File);
        let Preview::Meta(m) = render(&entry, &Limits::default()) else {
            panic!("expected a meta card");
        };
        assert!(
            m.rows
                .iter()
                .any(|r| r.0 == "Video" && r.1.contains("320×240"))
        );
        assert!(m.poster.is_some_and(|p| p.width == 320));
    }
}
