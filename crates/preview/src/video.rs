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

/// First filter of every picture chain: deinterlaces only frames the decoder marks as
/// interlaced (DVDs, TV recordings), so progressive video passes through untouched. It
/// must run before scaling, which would otherwise smear the fields together.
pub(crate) const DEINTERLACE: &str = "bwdif=mode=send_frame:parity=auto:deint=interlaced";

static MANAGED_TOOLS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Register the folder holding app-downloaded tools; it is searched after every system location.
pub fn set_managed_tools_dir(dir: PathBuf) {
    let _ = MANAGED_TOOLS.set(dir);
}

/// What to tell the user when ffmpeg or ffprobe is missing.
pub fn missing_hint() -> &'static str {
    if cfg!(windows) {
        "Install ffmpeg from Preferences > Media to play this video."
    } else {
        "Install ffmpeg and ffprobe (for example with your package manager) to play this video."
    }
}

/// First existing `tool` from the sidecar / `PATH` / package-manager / managed-folder candidates.
pub fn find_tool(tool: &str) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let path_env = std::env::var("PATH").unwrap_or_default();
    tool_candidates(
        Os::current(),
        tool,
        exe_dir.as_deref(),
        &path_env,
        MANAGED_TOOLS.get().map(PathBuf::as_path),
    )
    .into_iter()
    .find(|p| p.is_file())
}

/// ffmpeg input for `path`, or for bytes `start..end` of it via `subfile` (DVD images).
/// `None` for an unsafe path or an empty range.
pub(crate) fn input_for(path: &str, range: Option<(u64, u64)>) -> Option<(&'static str, String)> {
    if !safe_path(path) {
        return None;
    }
    match range {
        None => Some(("file", path.to_string())),
        // `,,` would end the subfile option list early.
        Some((start, end)) if start < end && !path.contains(",,") => Some((
            "subfile,file",
            format!("subfile,,start,{start},end,{end},,:{path}"),
        )),
        Some(_) => None,
    }
}

/// Whether `path` names a disc image the Media player reads as DVD-Video.
pub fn is_disc_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("iso"))
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

static RUNNING_TOOLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// ffmpeg/ffprobe helpers (probes, posters, thumbnails) running now (diagnostic log).
pub fn running_tools() -> usize {
    RUNNING_TOOLS.load(Ordering::Relaxed)
}

struct RunningTool;

impl Drop for RunningTool {
    fn drop(&mut self) {
        RUNNING_TOOLS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Run `cmd`, keep at most `max` bytes of stdout, and kill it after `timeout`.
pub(crate) fn run(
    mut cmd: Command,
    timeout: Duration,
    max: u64,
    cancelled: Option<&AtomicBool>,
) -> Option<Vec<u8>> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return None;
    }
    let mut child = cmd.spawn().ok()?;
    RUNNING_TOOLS.fetch_add(1, Ordering::Relaxed);
    let _running = RunningTool;
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
    fn playback_info(&self, path: &str, range: Option<(u64, u64)>) -> Option<crate::VideoInfo> {
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
            range,
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
    let ffprobe = find_tool("ffprobe").ok_or_else(|| fail(missing_hint()))?;
    if find_tool("ffmpeg").is_none() {
        return Err(fail(missing_hint()));
    }
    // A DVD image plays its main title: one contiguous range of title parts.
    let range = if is_disc_image(&path) {
        let title = crate::dvd_iso::main_title_of(Path::new(&path))?;
        Some((title.start, title.end))
    } else {
        None
    };
    probe(&ffprobe, &path, range, Some(cancelled))
        .and_then(|probe| probe.playback_info(&path, range))
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

fn probe(
    ffprobe: &Path,
    path: &str,
    range: Option<(u64, u64)>,
    cancelled: Option<&AtomicBool>,
) -> Option<Probe> {
    let (protocols, input) = input_for(path, range)?;
    let mut c = command(ffprobe);
    c.args(["-v", "error", "-protocol_whitelist", protocols])
        .args([
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-i",
        ])
        .arg(input);
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
                    // Anamorphic video: stretch to the display aspect ratio, not the coded one.
                    let w = (f64::from(w) * pixel_aspect(text(s, "sample_aspect_ratio").as_deref()))
                        .round()
                        .clamp(1.0, 16_384.0) as u32;
                    // Phone videos filmed upright are stored sideways with a rotation that
                    // ffmpeg applies while decoding: the picture is then h wide and w high.
                    let (w, h) = if quarter_turn(s) { (h, w) } else { (w, h) };
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
                    let language = s
                        .get("tags")
                        .and_then(|tags| tags.get("language"))
                        .and_then(|value| value.as_str())
                        .map(clean_tag)
                        .filter(|value| !value.is_empty() && value != "und");
                    tracks.subtitles.push(crate::SubtitleTrack {
                        label: track_label(s, &codec, None),
                        text: crate::subtitles::is_text_codec(&codec),
                        language,
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

/// Whether a video stream is shown turned by 90 or 270 degrees: the display matrix
/// rotation (`side_data_list[].rotation`, ffprobe 5 and later) or the older `rotate` tag.
fn quarter_turn(stream: &serde_json::Value) -> bool {
    let side = stream
        .get("side_data_list")
        .and_then(|list| list.as_array())
        .into_iter()
        .flatten()
        .take(16)
        .find_map(|data| data.get("rotation").and_then(serde_json::Value::as_f64));
    let tag = || {
        stream
            .get("tags")
            .and_then(|tags| tags.get("rotate"))
            .and_then(|rotate| rotate.as_str())
            .and_then(|rotate| rotate.trim().parse::<f64>().ok())
    };
    side.or_else(tag)
        .filter(|degrees| degrees.is_finite())
        .is_some_and(|degrees| (degrees.round() as i64).rem_euclid(180) == 90)
}

/// `sample_aspect_ratio` such as "16:11"; 1.0 when missing, unknown ("0:1") or absurd.
fn pixel_aspect(r: Option<&str>) -> f64 {
    r.and_then(|r| r.split_once(':'))
        .and_then(|(n, d)| Some((n.parse::<f64>().ok()?, d.parse::<f64>().ok()?)))
        .filter(|(n, d)| *n > 0.0 && *d > 0.0)
        .map(|(n, d)| n / d)
        .filter(|a| (0.25..=4.0).contains(a))
        .unwrap_or(1.0)
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
    .arg(format!("{DEINTERLACE},scale='min({width},iw)':-2"))
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
    // Looked up each time (not cached) so a tool installed later is picked up.
    let ffmpeg = find_tool("ffmpeg")?;
    let duration = find_tool("ffprobe")
        .as_ref()
        .and_then(|p| probe(p, path, None, None))
        .and_then(|p| p.duration);
    let at = duration.map_or(0.0, |d| (d * 0.1).min(10.0));
    let limits = Limits {
        image_edge: edge,
        ..Limits::default()
    };
    poster(&ffmpeg, path, at, edge, entry, &limits)
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
            hint: Some(missing_hint().into()),
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
    let info = probe(&ffprobe, path, None, None);
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
    let video = info.as_ref().and_then(|p| p.playback_info(path, None));
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
    fn anamorphic_video_uses_the_display_aspect() {
        let raw = br#"{"streams":[{"codec_type":"video","codec_name":"mpeg2video","width":720,"height":576,"sample_aspect_ratio":"64:45","avg_frame_rate":"25/1"}]}"#;
        let (w, h, _) = parse_probe(raw).and_then(|p| p.video).expect("video");
        assert_eq!((w, h), (1024, 576));
        assert_eq!(pixel_aspect(Some("0:1")), 1.0);
        assert_eq!(pixel_aspect(None), 1.0);
    }

    #[test]
    fn upright_phone_videos_are_taller_than_wide() {
        // A phone video filmed upright: stored 1920×1080, shown turned a quarter.
        let side = |rotation: &str| {
            format!(
                r#"{{"streams":[{{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"sample_aspect_ratio":"1:1","side_data_list":[{{"side_data_type":"Display Matrix","rotation":{rotation}}}]}}]}}"#
            )
        };
        let size = |raw: &str| {
            parse_probe(raw.as_bytes())
                .and_then(|p| p.video)
                .map(|(w, h, _)| (w, h))
        };
        for rotation in ["-90", "90", "270", "-270", "90.0"] {
            assert_eq!(size(&side(rotation)), Some((1080, 1920)), "{rotation}");
        }
        for rotation in ["0", "180", "-180", "45"] {
            assert_eq!(size(&side(rotation)), Some((1920, 1080)), "{rotation}");
        }
        // Older ffprobe: a "rotate" tag.
        let tag = |rotate: &str| {
            format!(
                r#"{{"streams":[{{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"tags":{{"rotate":"{rotate}"}}}}]}}"#
            )
        };
        assert_eq!(size(&tag("90")), Some((1080, 1920)));
        assert_eq!(size(&tag("180")), Some((1920, 1080)));
        assert_eq!(size(&tag("sideways")), Some((1920, 1080)));
        // An anamorphic upright video: the pixel aspect widens the stored width first.
        let anamorphic = r#"{"streams":[{"codec_type":"video","codec_name":"mpeg2video","width":720,"height":576,"sample_aspect_ratio":"64:45","side_data_list":[{"rotation":90}]}]}"#;
        assert_eq!(size(anamorphic), Some((576, 1024)));
    }

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
    fn interlaced_video_thumbnails_are_deinterlaced() {
        let Ok(dir) = tempfile::tempdir() else {
            return;
        };
        let Some(path) = crate::player::tests::interlaced_clip(dir.path()) else {
            return;
        };
        let uri = fm_vfs::VfsUri::parse(&path.to_string_lossy()).expect("uri");
        let entry = Entry::new(uri, fm_vfs::EntryKind::File);
        let Some(doc) = thumbnail(&entry, 720) else {
            // Without ffprobe the poster cannot be placed; nothing to check.
            return;
        };
        let (rgba, _) = doc.frames.first().expect("frame");
        assert_eq!((doc.width, doc.height), (720, 576));
        assert_eq!(
            crate::player::tests::combing(rgba, doc.width, doc.height),
            0
        );
    }

    /// A 320×180 clip, left half red and right half white, stored with a 90° display
    /// rotation like an upright phone video: shown 180×320 with the red half at the bottom.
    fn upright_phone_clip(dir: &Path) -> Option<std::path::PathBuf> {
        let ff = find_tool("ffmpeg")?;
        let plain = dir.join("plain.mp4");
        let rotated = dir.join("upright.mp4");
        let run = |args: &[&str], out: &Path| {
            command(&ff)
                .args(["-v", "error", "-y"])
                .args(args)
                .arg(out)
                .status()
                .is_ok_and(|status| status.success())
        };
        // H.264 where this ffmpeg can encode it, so hardware decoders take part too.
        let source = [
            "-f",
            "lavfi",
            "-i",
            "color=white:size=320x180:duration=1:rate=25,\
             drawbox=x=0:y=0:w=160:h=180:color=red:t=fill",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
        ];
        let encoded = |codec: &str| {
            let mut args = source.to_vec();
            args.push(codec);
            run(&args, &plain)
        };
        let made = (encoded("libx264") || encoded("mpeg4"))
            && run(
                &[
                    "-display_rotation:v:0",
                    "90",
                    "-i",
                    plain.to_str()?,
                    "-c",
                    "copy",
                ],
                &rotated,
            );
        made.then_some(rotated)
    }

    /// Red pixels in the top and bottom quarter of an RGBA picture.
    fn red_top_and_bottom(rgba: &[u8], width: u32, height: u32) -> (usize, usize) {
        let red = |rows: std::ops::Range<u32>| {
            rows.filter_map(|row| {
                let stride = width as usize * 4;
                rgba.get(row as usize * stride..(row as usize + 1) * stride)
            })
            .flat_map(|line| line.as_chunks::<4>().0.iter())
            .filter(|px| px[0] > 200 && px[1] < 80)
            .count()
        };
        (red(0..height / 4), red(height * 3 / 4..height))
    }

    #[test]
    fn upright_phone_videos_play_and_thumbnail_upright_and_unstretched() {
        let Ok(dir) = tempfile::tempdir() else {
            return;
        };
        let Some(path) = upright_phone_clip(dir.path()) else {
            return;
        };
        let uri = fm_vfs::VfsUri::parse(&path.to_string_lossy()).expect("uri");
        let Ok(info) = playback_info(&uri, &AtomicBool::new(false)) else {
            // Without ffprobe nothing can be checked.
            return;
        };
        assert_eq!((info.width, info.height), (180, 320));
        for hardware in [None, Some(crate::player::HwAccel::for_platform())] {
            let spec = crate::player::PlaySpec {
                info: info.clone(),
                from: 0.5,
                width: 180,
                speed: 1.0,
                still: true,
                hardware,
                overlay: None,
            };
            let player =
                crate::player::Player::start(&spec, fm_media::Gain::default()).expect("player");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            while !player.finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let frame = player.latest().expect("frame");
            eprintln!("{hardware:?}: decoded with {:?}", player.decoding());
            assert_eq!((frame.width, frame.height), (180, 320), "{hardware:?}");
            let (top, bottom) = red_top_and_bottom(&frame.rgba, frame.width, frame.height);
            assert!(
                top == 0 && bottom > 10_000,
                "{hardware:?}: red {top} / {bottom}"
            );
        }
        let entry = Entry::new(uri, fm_vfs::EntryKind::File);
        if let Some(doc) = thumbnail(&entry, 320) {
            assert!(doc.height > doc.width, "{}×{}", doc.width, doc.height);
        }
    }

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
    fn real_dvd_vob_probes_for_playback() {
        let (Some(ffmpeg), Some(_)) = (find_tool("ffmpeg"), find_tool("ffprobe")) else {
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("VTS_01_1.VOB");
        let made = Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=size=720x576:rate=25:duration=2")
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args(["-c:v", "mpeg2video", "-c:a", "mp2", "-f", "vob"])
            .arg(&file)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // ffmpeg without the MPEG-2 encoder
        }
        assert_eq!(
            crate::thumbs::classify("VTS_01_1.VOB"),
            crate::thumbs::ThumbKind::Video
        );
        let uri = fm_vfs::VfsUri::parse(&file.to_string_lossy()).expect("uri");
        let info = playback_info(&uri, &AtomicBool::new(false)).expect("probe");
        assert_eq!((info.width, info.height), (720, 576));
        assert!(info.audio.is_some(), "the soundtrack is found");
        assert!(info.duration > 1.0);
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
