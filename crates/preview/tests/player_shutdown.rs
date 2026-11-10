#![allow(clippy::expect_used)]

use std::process::Command;
use std::time::{Duration, Instant};

use fm_media::Gain;
use fm_preview::player::{PlaySpec, Player, shutdown};
use fm_preview::{VideoInfo, video::find_tool};

#[test]
fn quitting_waits_for_active_and_replaced_video_workers() {
    let Some(ffmpeg) = find_tool("ffmpeg") else {
        return;
    };
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("clip.mkv");
    assert!(
        Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=25:duration=5",
                "-c:v",
                "mpeg4",
                "-pix_fmt",
                "yuv420p",
                "-threads",
                "1",
            ])
            .arg(&path)
            .status()
            .expect("fixture")
            .success()
    );
    let spec = PlaySpec {
        info: VideoInfo {
            path: path.to_string_lossy().into_owned(),
            width: 320,
            height: 240,
            fps: 25.0,
            duration: 5.0,
            audio: None,
            audio_delay: 0.0,
            tracks: Default::default(),
        },
        from: 0.0,
        width: 320,
        speed: 1.0,
        still: false,
    };
    let first = Player::start(&spec, Gain::default()).expect("first");
    let second = Player::start(&spec, Gain::default()).expect("second");
    let deadline = Instant::now() + Duration::from_secs(8);
    while first.latest().is_none() || second.latest().is_none() {
        assert!(Instant::now() < deadline, "no frames");
        assert!(first.error().is_none() && second.error().is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(first);
    shutdown().expect("worker cleanup");
    assert!(second.finished());
    assert!(second.error().is_none());
}
