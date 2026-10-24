//! Video playback: one `ffmpeg` child decodes to raw RGBA frames on a pipe, paced at the
//! source frame rate (`-re`). A reader thread keeps only the newest frame, so memory stays
//! at a couple of frames however slow the UI is. Pausing or seeking kills the child and
//! starts a new one at the wanted position; dropping a [`Player`] kills it.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::VideoInfo;
use crate::video::{command, find_tool, safe_path};

/// Widest decode (ffmpeg scales down; the UI scales to the pane).
pub const MAX_WIDTH: u32 = 1280;
pub const MIN_WIDTH: u32 = 160;

/// One decoded frame.
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Increases with every frame of one player.
    pub seq: u64,
}

/// What to decode.
#[derive(Clone, Debug)]
pub struct PlaySpec {
    pub info: VideoInfo,
    /// Start position in seconds.
    pub from: f64,
    /// Decode width; the height follows the video's aspect ratio.
    pub width: u32,
    /// A single frame (used to show the frame at a paused seek position).
    pub still: bool,
}

#[derive(Default, Debug)]
struct Shared {
    latest: Mutex<Option<Arc<Frame>>>,
    frames: AtomicU64,
    done: AtomicBool,
}

#[derive(Debug)]
pub struct Player {
    child: Child,
    shared: Arc<Shared>,
    from: f64,
    fps: f64,
    pub still: bool,
}

fn ffmpeg() -> Option<&'static PathBuf> {
    static TOOL: OnceLock<Option<PathBuf>> = OnceLock::new();
    TOOL.get_or_init(|| find_tool("ffmpeg")).as_ref()
}

/// Even decode dimensions for `width` (clamped) that keep the source aspect ratio.
pub fn decode_size(info: &VideoInfo, width: u32) -> Option<(u32, u32)> {
    if info.width == 0 || info.height == 0 {
        return None;
    }
    let w = (width.clamp(MIN_WIDTH, MAX_WIDTH) / 2 * 2).min(info.width.max(2) / 2 * 2);
    let w = w.max(2);
    let h = ((u64::from(w) * u64::from(info.height) / u64::from(info.width)) / 2 * 2).max(2);
    Some((w, u32::try_from(h).ok()?))
}

impl Player {
    /// Start decoding; `None` without ffmpeg, for an unsafe path or if it cannot spawn.
    pub fn start(spec: &PlaySpec) -> Option<Player> {
        if !safe_path(&spec.info.path) {
            return None;
        }
        let (w, h) = decode_size(&spec.info, spec.width)?;
        let mut c = command(ffmpeg()?);
        c.args(["-v", "error", "-nostdin", "-protocol_whitelist", "file"]);
        if !spec.still {
            c.arg("-re");
        }
        c.arg("-ss")
            .arg(format!("{:.3}", spec.from.max(0.0)))
            .arg("-i")
            .arg(&spec.info.path)
            .args(["-an", "-sn", "-dn", "-vf"])
            .arg(format!("scale={w}:{h}"))
            .args(["-pix_fmt", "rgba", "-f", "rawvideo"]);
        if spec.still {
            c.args(["-frames:v", "1"]);
        }
        c.arg("-").stdout(Stdio::piped());
        let mut child = c.spawn().ok()?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        };
        let shared = Arc::new(Shared::default());
        let sh = shared.clone();
        let size = w as usize * h as usize * 4;
        std::thread::spawn(move || {
            loop {
                let mut buf = vec![0u8; size];
                if out.read_exact(&mut buf).is_err() {
                    break;
                }
                let seq = sh.frames.fetch_add(1, Ordering::SeqCst) + 1;
                let frame = Arc::new(Frame {
                    width: w,
                    height: h,
                    rgba: buf,
                    seq,
                });
                if let Ok(mut slot) = sh.latest.lock() {
                    *slot = Some(frame);
                }
            }
            sh.done.store(true, Ordering::SeqCst);
        });
        Some(Player {
            child,
            shared,
            from: spec.from,
            fps: if spec.info.fps > 0.0 {
                spec.info.fps
            } else {
                25.0
            },
            still: spec.still,
        })
    }

    /// The newest frame, if any arrived.
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.shared.latest.lock().ok().and_then(|l| l.clone())
    }

    /// Position of the newest frame in the video, in seconds.
    pub fn position(&self) -> f64 {
        let n = self.shared.frames.load(Ordering::SeqCst);
        self.from + n.saturating_sub(1) as f64 / self.fps
    }

    /// The decoder has no more frames (end of video, or it died).
    pub fn finished(&self) -> bool {
        self.shared.done.load(Ordering::SeqCst)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use std::process::Command;

    use super::*;

    fn info(w: u32, h: u32) -> VideoInfo {
        VideoInfo {
            path: "/v.mp4".into(),
            width: w,
            height: h,
            fps: 25.0,
            duration: 10.0,
        }
    }

    #[test]
    fn decode_size_is_even_clamped_and_keeps_the_aspect() {
        assert_eq!(decode_size(&info(1920, 1080), 640), Some((640, 360)));
        assert_eq!(decode_size(&info(1920, 1080), 99_999), Some((1280, 720)));
        assert_eq!(decode_size(&info(1920, 1080), 1), Some((160, 90)));
        // never upscales past the source
        assert_eq!(decode_size(&info(320, 240), 1280), Some((320, 240)));
        assert_eq!(decode_size(&info(0, 240), 640), None);
        let (w, h) = decode_size(&info(1001, 563), 641).expect("size");
        assert!(w % 2 == 0 && h % 2 == 0);
    }

    #[test]
    fn an_unsafe_path_is_refused() {
        let mut i = info(640, 480);
        i.path = "-i".into();
        let spec = PlaySpec {
            info: i,
            from: 0.0,
            width: 640,
            still: false,
        };
        assert!(Player::start(&spec).is_none());
    }

    /// Needs ffmpeg; skipped when it is not installed.
    #[test]
    fn plays_a_generated_clip_and_stops_on_drop() {
        let Some(ff) = ffmpeg() else { return };
        let dir = std::env::temp_dir().join(format!("fm-player-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let clip = dir.join("t.mp4");
        let made = Command::new(ff)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=size=320x240:rate=25:duration=2")
            .args(["-pix_fmt", "yuv420p", "-y"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let info = VideoInfo {
            path: clip.to_string_lossy().into_owned(),
            width: 320,
            height: 240,
            fps: 25.0,
            duration: 2.0,
        };
        let spec = PlaySpec {
            info: info.clone(),
            from: 0.0,
            width: 320,
            still: false,
        };
        let player = Player::start(&spec).expect("player");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while player.latest().is_none() {
            assert!(std::time::Instant::now() < deadline, "no frame arrived");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let f = player.latest().expect("frame");
        assert_eq!((f.width, f.height), (320, 240));
        assert_eq!(f.rgba.len(), 320 * 240 * 4);
        // Paced at 25 fps, a 2 s clip is not finished after the first frame.
        assert!(!player.finished());
        drop(player);

        let still = Player::start(&PlaySpec {
            from: 1.0,
            still: true,
            ..spec
        })
        .expect("still");
        while !still.finished() {
            assert!(std::time::Instant::now() < deadline, "still never ended");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(still.latest().is_some());
        assert!((still.position() - 1.0).abs() < 0.01);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
