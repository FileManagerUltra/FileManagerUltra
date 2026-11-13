#![allow(clippy::expect_used)]

use super::*;
use fm_media::{AudioConsumer, audio_buffer};
use std::process::Command;
use std::sync::atomic::AtomicU64;

fn info(w: u32, h: u32) -> VideoInfo {
    VideoInfo {
        path: "/v.mp4".into(),
        range: None,
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
            hardware: None,
            overlay: None,
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
        hardware: None,
        overlay: None,
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
        hardware: None,
        overlay: None,
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

/// A one-subtitle PGS (Blu-ray) stream on a 320×240 canvas: a white `w`×`h` box at
/// (`x`, `y`), shown from `from` to `to` seconds.
fn pgs(x: u16, y: u16, w: u16, h: u16, from: f64, to: f64) -> Vec<u8> {
    fn segment(out: &mut Vec<u8>, kind: u8, pts: u32, body: &[u8]) {
        out.extend_from_slice(b"PG");
        out.extend_from_slice(&pts.to_be_bytes());
        out.extend_from_slice(&0_u32.to_be_bytes());
        out.push(kind);
        out.extend_from_slice(&u16::try_from(body.len()).expect("segment").to_be_bytes());
        out.extend_from_slice(body);
    }
    let composition = |number: u16, state: u8, objects: &[u8]| {
        let mut pcs = Vec::new();
        pcs.extend_from_slice(&320_u16.to_be_bytes());
        pcs.extend_from_slice(&240_u16.to_be_bytes());
        pcs.extend_from_slice(&[0x10]);
        pcs.extend_from_slice(&number.to_be_bytes());
        pcs.extend_from_slice(&[state, 0, 0]);
        pcs.extend_from_slice(objects);
        pcs
    };
    let mut window = vec![1, 0];
    for value in [x, y, w, h] {
        window.extend_from_slice(&value.to_be_bytes());
    }
    let mut object = vec![1, 0, 0, 0, 0];
    object.extend_from_slice(&x.to_be_bytes());
    object.extend_from_slice(&y.to_be_bytes());
    // One run of palette entry 1 per line, then the end-of-line marker.
    let mut rle = Vec::new();
    for _ in 0..h {
        rle.extend_from_slice(&[0, 0xC0 | (w >> 8) as u8, w as u8, 1, 0, 0]);
    }
    let mut ods = vec![0, 0, 0, 0xC0];
    ods.extend_from_slice(&u32::try_from(rle.len() + 4).expect("ods").to_be_bytes()[1..]);
    ods.extend_from_slice(&w.to_be_bytes());
    ods.extend_from_slice(&h.to_be_bytes());
    ods.extend_from_slice(&rle);
    let (show, hide) = ((from * 90_000.0) as u32, (to * 90_000.0) as u32);
    let mut out = Vec::new();
    segment(&mut out, 0x16, show, &composition(0, 0x80, &object));
    segment(&mut out, 0x17, show, &window);
    segment(&mut out, 0x14, show, &[0, 0, 1, 235, 128, 128, 255]);
    segment(&mut out, 0x15, show, &ods);
    segment(&mut out, 0x80, show, &[]);
    segment(&mut out, 0x16, hide, &composition(1, 0, &[]));
    segment(&mut out, 0x17, hide, &window);
    segment(&mut out, 0x80, hide, &[]);
    out
}

/// Black 3 s video (optionally with a tone) and one PGS track: a 100×20 white box near
/// the bottom from 0.5 s to 2.5 s.
fn subtitled_clip(audio: bool) -> Option<(tempfile::TempDir, PlaySpec)> {
    let ff = find_tool("ffmpeg")?;
    let dir = tempfile::tempdir().expect("directory");
    let sup = dir.path().join("subs.sup");
    std::fs::write(&sup, pgs(110, 200, 100, 20, 0.5, 2.5)).expect("sup");
    let path = dir.path().join("subtitled.mkv");
    let mut c = Command::new(ff);
    c.args([
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=black:size=320x240:rate=25:duration=3",
    ]);
    if audio {
        c.args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=3",
        ]);
    }
    c.arg("-i").arg(&sup).args(["-map", "0:v"]);
    c.args(["-map", if audio { "2:s" } else { "1:s" }]);
    if audio {
        c.args(["-map", "1:a", "-c:a", "pcm_s16le"]);
    }
    let made = c
        .args(["-c:v", "mpeg4", "-c:s", "copy", "-threads", "1", "-y"])
        .arg(&path)
        .status()
        .expect("ffmpeg");
    assert!(made.success(), "fixture generation failed");
    let mut video = info(320, 240);
    video.path = path.to_string_lossy().into_owned();
    video.duration = 3.0;
    video.audio = audio.then(|| AudioFormat::new(44_100, 1).expect("format"));
    video.tracks.subtitles = vec![crate::SubtitleTrack {
        label: "PGS".into(),
        text: false,
        language: None,
    }];
    Some((
        dir,
        PlaySpec {
            info: video,
            from: 0.0,
            width: 320,
            speed: 1.0,
            still: false,
            hardware: None,
            overlay: None,
        },
    ))
}

/// Bright pixels in rows `rows` of `frame`.
fn bright(frame: &Frame, rows: std::ops::Range<u32>) -> usize {
    let stride = frame.width as usize * 4;
    rows.filter_map(|row| {
        frame
            .rgba
            .get(row as usize * stride..(row as usize + 1) * stride)
    })
    .map(|line| {
        line.as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[0] > 200)
            .count()
    })
    .sum()
}

fn still_frame(spec: &PlaySpec) -> Arc<Frame> {
    let player = Player::start(spec, Gain::default()).expect("still");
    wait(|| player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    player.latest().expect("frame")
}

const PGS_TRACK: Overlay = Overlay {
    stream: 0,
    scale: 1.0,
    lift: 0.0,
};

#[test]
fn picture_subtitles_are_drawn_where_and_as_large_as_asked() {
    let Some((_dir, spec)) = subtitled_clip(false) else {
        return;
    };
    // 1 s is after the subtitle started (0.5 s): the decoder must start early enough.
    let at = |overlay| PlaySpec {
        from: 1.0,
        still: true,
        overlay,
        ..spec.clone()
    };
    let plain = still_frame(&at(None));
    assert_eq!(bright(&plain, 0..240), 0, "no overlay, no subtitle");

    let drawn = still_frame(&at(Some(PGS_TRACK)));
    assert!(bright(&drawn, 198..222) > 1500, "box at the authored place");

    let lifted = still_frame(&at(Some(Overlay {
        lift: 0.25,
        ..PGS_TRACK
    })));
    assert_eq!(bright(&lifted, 190..240), 0);
    assert!(bright(&lifted, 138..162) > 1500, "box 60 px higher");

    // Anchored at the bottom centre: 150×30 at y 180..210.
    let larger = still_frame(&at(Some(Overlay {
        scale: 1.5,
        ..PGS_TRACK
    })));
    assert!(bright(&larger, 178..212) > 3500, "box 1.5× as large");

    let after = still_frame(&PlaySpec {
        from: 2.8,
        ..at(Some(PGS_TRACK))
    });
    assert_eq!(bright(&after, 0..240), 0, "cleared at 2.5 s");
}

#[test]
fn switching_picture_subtitles_while_playing_keeps_the_audio_going() {
    let Some((_dir, spec)) = subtitled_clip(true) else {
        return;
    };
    let audible = Arc::new(AtomicU64::new(0));
    let sink_audible = audible.clone();
    // `open` runs once: a second audio output would need a new player.
    let player = Player::start_with(&spec, Gain::default(), move |g| {
        fake_output(g, Arc::new(AtomicBool::new(true)), sink_audible)
    })
    .expect("player");
    wait(|| player.position() > 0.7);
    let latest = player.latest().expect("frame");
    assert_eq!(bright(&latest, 0..240), 0);
    let before = audible.load(Ordering::Relaxed);
    let position = player.position();
    let switched = Instant::now();
    player.set_overlay(Some(PGS_TRACK), &spec.info);
    wait(|| {
        player
            .latest()
            .is_some_and(|frame| bright(&frame, 198..222) > 1500)
    });
    assert!(
        switched.elapsed() < Duration::from_millis(1500),
        "subtitles appeared after {:?}",
        switched.elapsed()
    );
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(player.position() > position, "the clock kept running");
    assert!(
        audible.load(Ordering::Relaxed) > before,
        "audio kept playing"
    );
    // Turning them off again works the same way.
    player.set_overlay(None, &spec.info);
    wait(|| {
        player
            .latest()
            .is_some_and(|frame| bright(&frame, 0..240) == 0)
            || player.finished()
    });
    assert!(player.error().is_none(), "{:?}", player.error());
}

#[test]
fn failing_hardware_decoding_plays_on_in_software_from_the_same_position() {
    let Some((_dir, spec)) = clip(false) else {
        return;
    };
    let spec = PlaySpec {
        from: 1.0,
        hardware: Some(HwAccel::Missing),
        ..spec
    };
    let player = Player::start(&spec, Gain::default()).expect("player");
    wait(|| player.latest().is_some() || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    assert_eq!(player.decoding(), Some(Decoding::Software));
    assert!(player.position() >= 1.0);
    // Failing before the first frame is not news: an unsupported file just plays.
    assert_eq!(player.take_fallback(), None);
    // The missing device is remembered for that method only, until reset.
    assert!(HwAccel::Missing.unavailable());
    reset_hardware();
    assert!(!HwAccel::Missing.unavailable());
}

#[test]
fn video_command_asks_for_hardware_and_draws_picture_subtitles() {
    let Some((_dir, spec)) = subtitled_clip(false) else {
        return;
    };
    let tool = std::path::Path::new("ffmpeg");
    let args = |at, exact, hardware, overlay| -> Vec<String> {
        video_command(tool, &spec, at, exact, hardware, overlay)
            .expect("command")
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    };
    let position = |args: &[String], value: &str| args.iter().position(|arg| arg == value);

    let plain = args(5.0, false, None, None);
    assert!(position(&plain, "-hwaccel").is_none());
    assert!(position(&plain, "-sn").is_some());
    assert_eq!(
        plain.get(position(&plain, "-ss").expect("seek") + 1),
        Some(&"5.000000".to_string())
    );

    let full = args(5.0, false, Some(HwAccel::Vaapi), Some(PGS_TRACK));
    let input = position(&full, "-i").expect("input");
    let hwaccel = position(&full, "-hwaccel").expect("hwaccel");
    assert!(hwaccel < input, "-hwaccel is an input option");
    assert_eq!(full.get(hwaccel + 1), Some(&"vaapi".to_string()));
    // Fast seek to 3 s, then decode and drop 2 s so earlier subtitles are not missed.
    let seeks: Vec<_> = full
        .iter()
        .enumerate()
        .filter(|(_, arg)| *arg == "-ss")
        .filter_map(|(i, _)| full.get(i + 1).map(|value| (i < input, value.clone())))
        .collect();
    assert_eq!(
        seeks,
        vec![(true, "3.000000".into()), (false, "2.000000".into())]
    );
    let graph = full
        .get(position(&full, "-filter_complex").expect("graph") + 1)
        .expect("graph");
    assert!(graph.contains("[0:s:0]scale=320:240"), "{graph}");
    assert!(graph.contains("overlay=x=(W-w)/2:y=H-h-0"), "{graph}");
    assert!(position(&full, "-sn").is_none());

    // Close to the start the preroll shrinks; it never seeks before 0.
    let early = args(0.5, false, None, Some(PGS_TRACK));
    assert_eq!(
        early.get(position(&early, "-ss").expect("seek") + 1),
        Some(&"0.000000".to_string())
    );
}

#[test]
fn invalid_overlays_are_refused() {
    let Some((_dir, spec)) = subtitled_clip(false) else {
        return;
    };
    for overlay in [
        Overlay {
            stream: 1,
            ..PGS_TRACK
        },
        Overlay {
            scale: 3.0,
            ..PGS_TRACK
        },
        Overlay {
            lift: f64::NAN,
            ..PGS_TRACK
        },
        Overlay {
            lift: 0.9,
            ..PGS_TRACK
        },
    ] {
        let bad = PlaySpec {
            overlay: Some(overlay),
            ..spec.clone()
        };
        assert!(Player::start(&bad, Gain::default()).is_err(), "{overlay:?}");
    }
}

#[test]
fn fallback_reasons_are_short_readable_and_redacted() {
    let err = error("Decoder failed");
    let reason = fallback_reason(
        HwAccel::Vaapi,
        "frame=1\n[vist#0:0/h264 @ 0x5cd1] [dec:h264 @ 0x5cd2] Hardware device setup failed for smb://alice:hunter2@nas/v.mkv\n\n",
        &err,
    );
    assert!(
        reason.starts_with("Hardware video decoding (VA-API) stopped"),
        "{reason}"
    );
    assert!(reason.contains("Hardware device setup failed"), "{reason}");
    assert!(!reason.contains("0x5cd"), "{reason}");
    assert!(!reason.contains("hunter2"), "{reason}");
    assert!(fallback_reason(HwAccel::D3d11va, "", &err).contains("Decoder failed"));
    // The cause, not ffmpeg's closing "Terminating thread" lines (seen on a DVD VOB).
    let vob = "[AVHWFramesContext @ 0x7b63] Failed to sync surface 0: 23 (internal decoding error).
[mpeg2video @ 0x5e8d] Failed to transfer data to output frame: -5.
[vist#0:1/mpeg2video @ 0x5e8d] [dec:mpeg2video @ 0x5e8d] Error while processing the decoded data
[vist#0:1/mpeg2video @ 0x5e8d] [dec:mpeg2video @ 0x5e8d] Error processing packet in decoder: Input/output error
[vist#0:1/mpeg2video @ 0x5e8d] [dec:mpeg2video @ 0x5e8d] Task finished with error code: -5 (Input/output error)
[vist#0:1/mpeg2video @ 0x5e8d] [dec:mpeg2video @ 0x5e8d] Terminating thread with return code -5 (Input/output error)
";
    let reason = fallback_reason(HwAccel::Vaapi, vob, &err);
    assert!(
        reason.ends_with("Failed to sync surface 0: 23 (internal decoding error)."),
        "{reason}"
    );
    assert!(near_end(271.0, 272.9) && !near_end(260.0, 272.9));
    assert!(!near_end(5.0, 0.0), "unknown duration: always news");
    assert!(fallback_reason(HwAccel::Vaapi, &"x".repeat(5000), &err).len() < 300);

    assert!(device_unavailable(
        "[VAAPI @ 0x1] Failed to initialise VAAPI connection: -1\nDevice creation failed: -5."
    ));
    assert!(device_unavailable(
        "Unrecognized hwaccel: fm-missing-hwaccel."
    ));
    assert!(!device_unavailable(
        "Invalid data found when processing input"
    ));
}

/// Plays H.264 (decodable by every hardware decoder) with the platform's hardware
/// decoding requested. Without a usable device this still plays in software; set
/// `FM_EXPECT_HWACCEL=1` on a machine with one to require the hardware path.
#[test]
fn platform_hardware_decoding_plays_and_reports_its_mode() {
    let Some((dir, spec)) = clip(false) else {
        return;
    };
    let h264 = dir.path().join("h264.mkv");
    let encoded = Command::new(find_tool("ffmpeg").expect("ffmpeg"))
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=25:duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-y",
        ])
        .arg(&h264)
        .status()
        .is_ok_and(|status| status.success());
    if !encoded {
        // An ffmpeg build without libx264 cannot make the fixture.
        return;
    }
    let mut spec = PlaySpec {
        hardware: Some(HwAccel::for_platform()),
        ..spec
    };
    spec.info.path = h264.to_string_lossy().into_owned();
    let player = Player::start(&spec, Gain::default()).expect("player");
    wait(|| player.latest().is_some() || player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    let decoding = player.decoding().expect("mode known after the first frame");
    eprintln!("decoding: {decoding:?}");
    if std::env::var_os("FM_EXPECT_HWACCEL").is_some_and(|v| v == "1") {
        assert_eq!(decoding, Decoding::Hardware(HwAccel::for_platform()));
    }
}

/// Interlaced PAL MPEG-2 (top field first, like a DVD) of a white bar racing across black:
/// without deinterlacing every frame shows combs along the bar's edges.
pub(crate) fn interlaced_clip(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let ff = find_tool("ffmpeg")?;
    let path = dir.join("interlaced.mpg");
    let made = Command::new(ff)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:size=720x576:rate=50:duration=2",
            "-f",
            "lavfi",
            "-i",
            "color=white:size=80x376:rate=50:duration=2",
            "-filter_complex",
            "[0][1]overlay=x='mod(t*1200,600)':y=100,tinterlace=mode=interleave_top,setfield=tff",
            "-c:v",
            "mpeg2video",
            "-flags",
            "+ilme+ildct",
            "-b:v",
            "8M",
            "-threads",
            "1",
            "-y",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg");
    assert!(made.success(), "fixture generation failed");
    Some(path)
}

/// How strongly neighbouring lines alternate (0 for a clean picture), over the middle rows.
pub(crate) fn combing(rgba: &[u8], width: u32, height: u32) -> u64 {
    let (w, h) = (width as usize, height as usize);
    let at = |x: usize, y: usize| i64::from(rgba.get((y * w + x) * 4).copied().unwrap_or(0));
    (h / 5..h * 4 / 5)
        .flat_map(|y| (0..w).step_by(2).map(move |x| (x, y)))
        .map(|(x, y)| (2 * at(x, y) - at(x, y - 1) - at(x, y + 1)).unsigned_abs())
        .sum()
}

#[test]
fn interlaced_video_is_deinterlaced_without_asking() {
    let Some(dir) = tempfile::tempdir().ok() else {
        return;
    };
    let Some(path) = interlaced_clip(dir.path()) else {
        return;
    };
    let mut video = info(720, 576);
    video.path = path.to_string_lossy().into_owned();
    let spec = PlaySpec {
        info: video,
        from: 0.8,
        width: 720,
        speed: 1.0,
        still: true,
        hardware: None,
        overlay: None,
    };
    let still = still_frame(&spec);
    assert_eq!(
        (still.width, still.height),
        (720, 576),
        "full height, so combs would show"
    );
    // The bar is on screen and its edges are clean.
    assert!(bright(&still, 280..290) > 600, "bar drawn (80 px wide)");
    assert_eq!(
        combing(&still.rgba, still.width, still.height),
        0,
        "still frame"
    );

    let player = Player::start(
        &PlaySpec {
            from: 0.0,
            still: false,
            ..spec.clone()
        },
        Gain::default(),
    )
    .expect("player");
    let mut checked = 0;
    let mut seen = None;
    wait(|| {
        if let Some(frame) = player.latest()
            && seen != Some(frame.seq)
        {
            seen = Some(frame.seq);
            assert_eq!(
                combing(&frame.rgba, frame.width, frame.height),
                0,
                "playback frame {}",
                frame.seq
            );
            checked += 1;
        }
        checked >= 10 || player.finished()
    });
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(checked >= 5, "only {checked} frames checked");
}

/// A DVD VOB part cut mid-group, like `VTS_01_1.VOB`: its last frames are damaged, which
/// makes VA-API give up just before the end while software only warns.
#[test]
fn a_damaged_end_of_file_finishes_in_software_without_a_log_line() {
    let Some(ff) = find_tool("ffmpeg") else {
        return;
    };
    let dir = tempfile::tempdir().expect("directory");
    let whole = dir.path().join("whole.vob");
    let made = Command::new(ff)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=720x576:rate=25:duration=4",
            "-c:v",
            "mpeg2video",
            "-b:v",
            "6M",
            "-g",
            "12",
            "-threads",
            "1",
            "-f",
            "vob",
            "-y",
        ])
        .arg(&whole)
        .status()
        .expect("ffmpeg");
    assert!(made.success(), "fixture generation failed");
    let bytes = std::fs::read(&whole).expect("read");
    // Cut on a 2 KiB DVD sector boundary, inside the last group of pictures.
    let cut = bytes.len() * 93 / 100 / 2048 * 2048;
    let path = dir.path().join("VTS_01_1.VOB");
    std::fs::write(&path, bytes.get(..cut).expect("cut")).expect("write");
    let mut video = info(720, 576);
    video.path = path.to_string_lossy().into_owned();
    video.duration = 3.7;
    let spec = PlaySpec {
        info: video,
        from: 2.6,
        width: 720,
        speed: 1.0,
        still: false,
        hardware: Some(HwAccel::for_platform()),
        overlay: None,
    };
    let player = Player::start(&spec, Gain::default()).expect("player");
    wait(|| player.finished());
    assert!(player.error().is_none(), "{:?}", player.error());
    assert!(
        player.position() > 3.4,
        "played to the end: {}",
        player.position()
    );
    assert_eq!(
        player.take_fallback(),
        None,
        "the file's fault, not the GPU's"
    );
    if std::env::var_os("FM_EXPECT_HWACCEL").is_some_and(|v| v == "1") {
        assert_eq!(
            player.decoding(),
            Some(Decoding::Software),
            "hardware gave up on the damaged frames and software finished"
        );
    }
}
