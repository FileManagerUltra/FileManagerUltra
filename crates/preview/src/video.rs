//! Video cards: metadata from `ffprobe` and a poster frame from `ffmpeg`, both run as
//! subprocesses (no link-time dependency, a decoder crash cannot take the app down).
//! Without the tools, or for non-local files, a metadata card with a hint is returned.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fm_vfs::{Entry, Scheme};

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
fn safe_path(path: &str) -> bool {
    let absolute =
        path.starts_with('/') || path.get(1..3) == Some(":\\") || path.starts_with("\\\\");
    absolute && !path.contains('\0')
}

fn command(tool: &Path) -> Command {
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
fn run(mut cmd: Command, timeout: Duration, max: u64) -> Option<Vec<u8>> {
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
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
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
}

fn probe(ffprobe: &Path, path: &str) -> Option<Probe> {
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
    let raw = run(c, PROBE_TIMEOUT, PROBE_BYTES)?;
    parse_probe(&raw)
}

fn parse_probe(raw: &[u8]) -> Option<Probe> {
    let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
    let text =
        |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let mut rows = Vec::new();
    let mut duration = None;
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
                let fps = text(s, "avg_frame_rate")
                    .and_then(|r| frame_rate(&r))
                    .map(|f| format!(" · {f:.2} fps"))
                    .unwrap_or_default();
                rows.push(("Video".to_string(), format!("{codec}{size}{fps}")));
            }
            Some("audio") => {
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
            Some("subtitle") => rows.push(("Subtitles".to_string(), codec)),
            _ => {}
        }
    }
    Some(Probe { rows, duration })
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

fn poster(ffmpeg: &Path, path: &str, at: f64, entry: &Entry, limits: &Limits) -> Option<ImageDoc> {
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
    .args([
        "-frames:v",
        "1",
        "-vf",
        "scale='min(1280,iw)':-2",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "-",
    ]);
    let png = run(c, POSTER_TIMEOUT, POSTER_BYTES)?;
    match image_prev::decode(entry, &png, limits) {
        Preview::Image(doc) => Some(doc),
        _ => None,
    }
}

pub fn render(entry: &Entry, limits: &Limits) -> Preview {
    let mut rows = base_meta(entry);
    let path = entry.uri.path_str();
    let local = entry.uri.scheme() == Scheme::File && entry.uri.authority().is_local();
    if !local {
        return Preview::Meta(MetaDoc {
            rows,
            hint: Some("Video previews need a local file; copy it locally to preview.".into()),
            poster: None,
        });
    }
    let (Some(ffprobe), Some(ffmpeg)) = (find_tool("ffprobe"), find_tool("ffmpeg")) else {
        return Preview::Meta(MetaDoc {
            rows,
            hint: Some("Install ffmpeg (ffprobe + ffmpeg on PATH) to see video details and a poster frame.".into()),
            poster: None,
        });
    };
    if !safe_path(path) {
        return Preview::Meta(MetaDoc {
            rows,
            hint: Some("This path cannot be passed safely to ffmpeg.".into()),
            poster: None,
        });
    }
    let info = probe(&ffprobe, path);
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
    Preview::Meta(MetaDoc {
        rows,
        hint,
        poster: poster(&ffmpeg, path, at, entry, limits).map(Box::new),
    })
}

#[cfg(test)]
mod tests {
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
    fn a_hanging_tool_is_killed() {
        if cfg!(windows) {
            return;
        }
        let mut c = Command::new("sleep");
        c.arg("30").stdout(Stdio::piped());
        let t = Instant::now();
        assert!(run(c, Duration::from_millis(100), 10).is_none());
        assert!(t.elapsed() < Duration::from_secs(5));
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
