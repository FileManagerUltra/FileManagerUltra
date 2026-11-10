#![allow(clippy::expect_used)]

use super::*;
use fm_media::{AudioConsumer, audio_buffer};
use std::process::Command;
use std::sync::atomic::AtomicU64;

fn info(w: u32, h: u32) -> VideoInfo {
    VideoInfo {
        path: "/v.mp4".into(),
        width: w,
        height: h,
        fps: 25.0,
        duration: 2.0,
        audio: None,
        audio_delay: 0.0,
        tracks: Default::default(),
    }
}

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready() {
        assert!(Instant::now() < deadline, "playback did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn clip(audio: bool) -> Option<(tempfile::TempDir, PlaySpec)> {
    clip_with_delay(audio, 0.0)
}

fn clip_with_delay(audio: bool, delay: f64) -> Option<(tempfile::TempDir, PlaySpec)> {
    let ff = find_tool("ffmpeg")?;
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("clip.mkv");
    let mut c = Command::new(ff);
    c.args([
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=320x240:rate=25:duration=2",
    ]);
    if audio {
        c.arg("-itsoffset").arg(delay.to_string());
        c.args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=2",
            "-c:a",
            "pcm_s16le",
        ]);
    }
    let made = c
        .args([
            "-c:v", "mpeg4", "-pix_fmt", "yuv420p", "-threads", "1", "-y",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg");
    assert!(made.success(), "fixture generation failed");
    let mut video = info(320, 240);
    video.path = path.to_string_lossy().into_owned();
    video.audio = audio.then(|| AudioFormat::new(44_100, 1).expect("format"));
    video.audio_delay = delay;
    Some((
        dir,
        PlaySpec {
            info: video,
            from: 0.0,
            width: 320,
            speed: 1.0,
            still: false,
        },
    ))
}

struct FakeSink {
    consumer: Mutex<AudioConsumer>,
    last: Mutex<Instant>,
    consume: Arc<AtomicBool>,
    audible: Arc<AtomicU64>,
    format: AudioFormat,
}

impl AudioSink for FakeSink {
    fn format(&self) -> AudioFormat {
        self.format
    }
    fn play(&self) -> Result<(), FmError> {
        Ok(())
    }
    fn pause(&self) -> Result<(), FmError> {
        Ok(())
    }
    fn check_error(&self) -> Result<(), FmError> {
        let mut last = self.last.lock().expect("time");
        let frames = (last.elapsed().as_secs_f64() * f64::from(self.format.rate())) as usize;
        if frames == 0 {
            return Ok(());
        }
        *last = Instant::now();
        if self.consume.load(Ordering::Acquire) {
            let mut out = vec![0.0_f32; frames.min(4800) * self.format.channels()];
            self.consumer.lock().expect("consumer").render(&mut out);
            self.audible.fetch_add(
                out.iter().filter(|v| v.abs() > 0.001).count() as u64,
                Ordering::Relaxed,
            );
        }
        Ok(())
    }
}

fn fake_output(
    gain: Gain,
    consume: Arc<AtomicBool>,
    audible: Arc<AtomicU64>,
) -> Result<Output, FmError> {
    let format = AudioFormat::new(48_000, 2)?;
    let (writer, consumer, clock) = audio_buffer(format, 4800, gain)?;
    Ok((
        Box::new(FakeSink {
            consumer: Mutex::new(consumer),
            last: Mutex::new(Instant::now()),
            consume,
            audible,
            format,
        }),
        writer,
        clock,
    ))
}

#[test]
fn decode_size_is_even_clamped_and_rejects_extreme_aspects() {
    assert_eq!(decode_size(&info(1920, 1080), 640), Some((640, 360)));
    assert_eq!(decode_size(&info(1920, 1080), 99_999), Some((1280, 720)));
    assert_eq!(decode_size(&info(1920, 1080), 1), Some((160, 90)));
    assert_eq!(decode_size(&info(320, 240), 1280), Some((320, 240)));
    assert_eq!(decode_size(&info(0, 240), 640), None);
    assert_eq!(decode_size(&info(2, 16_384), 1280), Some((2, 16_384)));
    assert_eq!(decode_size(&info(500, 16_384), 1280), None);
}

#[test]
fn atempo_chain_covers_speed_range_without_pitch_changing_resampling() {
    for (speed, filter) in [
        (0.5, "atempo=0.500000"),
        (1.0, "atempo=1.000000"),
        (1.5, "atempo=1.500000"),
        (2.0, "atempo=2.000000"),
        (2.5, "atempo=2.000000,atempo=1.250000"),
        (3.0, "atempo=2.000000,atempo=1.500000"),
    ] {
        assert_eq!(atempo_chain(speed).as_deref(), Some(filter));
    }
    for speed in [0.0, 0.49, 3.01, f64::NAN, f64::INFINITY] {
        assert!(atempo_chain(speed).is_none());
    }
}

#[test]
fn invalid_input_is_refused_before_spawning() {
    let mut spec = PlaySpec {
        info: info(640, 480),
        from: 0.0,
        width: 640,
        speed: 1.0,
        still: false,
    };
    spec.info.path = "-i".into();
    assert!(Player::start(&spec, Gain::default()).is_err());
    spec.info.path = "/v.mp4".into();
    spec.from = f64::NAN;
    assert!(Player::start(&spec, Gain::default()).is_err());
    spec.from = 0.0;
    spec.speed = 3.1;
    assert!(Player::start(&spec, Gain::default()).is_err());
}

#[test]
fn silent_video_clock_and_frames_follow_increased_playback_speed() {
    let Some((_dir, mut spec)) = clip(false) else {
        return;
    };
    spec.speed = 2.0;
    let player = Player::start(&spec, Gain::default()).expect("player");
    wait(|| player.position() > 0.3 || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(player.position() > 0.3);
}

#[test]
fn sped_up_video_uses_filtered_audio_as_its_clock() {
    let Some((_dir, mut spec)) = clip(true) else {
        return;
    };
    spec.speed = 2.0;
    let samples = Arc::new(AtomicU64::new(0));
    let audible = samples.clone();
    let player = Player::start_with(&spec, Gain::default(), move |gain| {
        fake_output(gain, Arc::new(AtomicBool::new(true)), audible)
    })
    .expect("player");
    wait(|| player.position() > 0.4 || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(player.position() > 0.3);
    assert!(samples.load(Ordering::Relaxed) > 0);
}

#[test]
fn silent_video_is_paced_still_seek_is_exact_and_drop_cancels_worker() {
    let Some((_dir, spec)) = clip(false) else {
        return;
    };
    let player = Player::start(&spec, Gain::default()).expect("player");
    wait(|| player.latest().is_some() || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    assert_eq!(player.latest().expect("frame").rgba.len(), 320 * 240 * 4);
    std::thread::sleep(Duration::from_millis(150));
    assert!(!player.finished());
    assert!(player.position() < 0.5);
    let shared = player.shared.clone();
    let before = Instant::now();
    drop(player);
    assert!(before.elapsed() < Duration::from_millis(100));
    wait(|| shared.snapshot.lock().expect("snapshot").done);
    let still = Player::start(
        &PlaySpec {
            from: 1.0,
            still: true,
            ..spec
        },
        Gain::default(),
    )
    .expect("still");
    wait(|| still.finished());
    assert!(still.error().is_none(), "{:?}", still.error());
    assert!(still.latest().is_some());
    assert_eq!(still.position(), 1.0);
}

#[test]
fn video_follows_fake_audio_clock_even_when_ui_is_stalled() {
    let Some((_dir, spec)) = clip(true) else {
        return;
    };
    let consume = Arc::new(AtomicBool::new(false));
    let audible = Arc::new(AtomicU64::new(0));
    let control = consume.clone();
    let samples = audible.clone();
    let gain = Gain::default();
    let player = Player::start_with(&spec, gain.clone(), move |g| {
        fake_output(g, consume, audible)
    })
    .expect("player");
    wait(|| player.latest().is_some() || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    // Both decoders hit bounded backpressure; no early frame can replace frame zero.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(player.latest().expect("frame").seq, 0);
    assert_eq!(player.position(), 0.0);
    control.store(true, Ordering::Release);
    // Deliberately do not poll the player while its worker and fake device run.
    std::thread::sleep(Duration::from_millis(800));
    let frame = player.latest().expect("frame");
    let drift = (frame.seq as f64 / spec.info.fps - player.position()).abs();
    assert!(drift < 0.1, "A/V drift {drift}");
    assert!(frame.seq > 10, "stalled UI must not stall decoding");
    assert!(
        samples.load(Ordering::Relaxed) > 1000,
        "PCM must reach the native-rate fake device"
    );
    gain.set_muted(true);
    std::thread::sleep(Duration::from_millis(30));
    let muted_at = samples.load(Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(samples.load(Ordering::Relaxed), muted_at);
    wait(|| player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
}

#[test]
fn missing_audio_device_keeps_video_playing_with_a_hint() {
    let Some((_dir, spec)) = clip(true) else {
        return;
    };
    let player = Player::start_with(&spec, Gain::default(), |_| Err(error("No audio device")))
        .expect("player");
    wait(|| player.latest().is_some() || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(player.hint().expect("hint").contains("Playing silently"));
    assert!(player.hint().expect("hint").contains("No audio device"));
    std::thread::sleep(Duration::from_millis(100));
    assert!(player.position() > 0.05);
}

#[test]
fn delayed_audio_keeps_its_offset_and_seek_does_not_repeat_the_delay() {
    let Some((_dir, spec)) = clip_with_delay(true, 0.4) else {
        return;
    };
    for from in [0.0, 0.7] {
        let samples = Arc::new(AtomicU64::new(0));
        let audible = samples.clone();
        let seeked = PlaySpec {
            from,
            ..spec.clone()
        };
        let player = Player::start_with(&seeked, Gain::default(), move |g| {
            fake_output(g, Arc::new(AtomicBool::new(true)), audible)
        })
        .expect("player");
        wait(|| player.position() > from + 0.2 || player.finished());
        assert!(player.error().is_none(), "{:?}", player.error());
        if from == 0.0 {
            assert_eq!(samples.load(Ordering::Relaxed), 0);
            wait(|| player.position() > 0.55 || player.finished());
        }
        assert!(samples.load(Ordering::Relaxed) > 0);
    }
}

#[test]
fn failed_decoder_is_reported_instead_of_successful_eof() {
    if find_tool("ffmpeg").is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("directory");
    let mut spec = PlaySpec {
        info: info(320, 240),
        from: 0.0,
        width: 320,
        speed: 1.0,
        still: false,
    };
    spec.info.path = dir
        .path()
        .join("vanished.mp4")
        .to_string_lossy()
        .into_owned();
    let player = Player::start(&spec, Gain::default()).expect("player");
    wait(|| player.finished());
    assert!(player.error().expect("error").contains("Decoder failed"));
}

#[test]
#[cfg(unix)]
fn hung_pipe_times_out_and_is_killed_on_drop() {
    let mut c = command(std::path::Path::new("/bin/sleep"));
    c.arg("5");
    let start = Instant::now();
    let mut pipe = Pipe::spawn(c, 1024, Duration::from_millis(30)).expect("pipe");
    wait(|| pipe.poll().is_err());
    drop(pipe);
    assert!(start.elapsed() < Duration::from_secs(2));
}
