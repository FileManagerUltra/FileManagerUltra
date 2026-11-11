#![allow(clippy::expect_used)]

use std::io::Write;
use std::process::Command;

use fm_media::{AudioDecoder, AudioFormat, Gain, Pipeline, audio_buffer};

fn wav(frames: u32) -> Vec<u8> {
    let bytes = frames * 2;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(bytes + 36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&48_000_u32.to_le_bytes());
    out.extend_from_slice(&96_000_u32.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for n in 0..frames {
        let value = ((n as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin() * 10_000.0) as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[test]
fn buffer_is_bounded_and_clock_counts_only_consumed_frames() {
    let format = AudioFormat::new(48_000, 2).expect("format");
    let gain = Gain::default();
    let (mut writer, mut reader, clock) = audio_buffer(format, 2, gain.clone()).expect("buffer");
    assert_eq!(writer.write(&[0.2; 6]).expect("write"), 4);
    assert_eq!(writer.queued_frames(), 2);
    clock.set_paused(true);
    let mut out = [9.0_f32; 6];
    reader.render(&mut out);
    assert_eq!(out, [0.0; 6]);
    assert_eq!(writer.queued_frames(), 2);
    assert_eq!(clock.position(), 0.0);
    clock.set_paused(false);
    gain.set_volume(50.0).expect("gain");
    reader.render(&mut out);
    assert_eq!(out, [0.1, 0.1, 0.1, 0.1, 0.0, 0.0]);
    assert!((clock.position() - 2.0 / 48_000.0).abs() < 1e-10);
    reader.render(&mut out);
    assert!((clock.position() - 2.0 / 48_000.0).abs() < 1e-10);
    assert!(writer.write(&[f32::NAN, 0.0]).is_err());
    assert!(writer.write(&[0.0]).is_err());
    assert!(audio_buffer(format, 96_001, gain).is_err());
}

#[test]
fn mapping_resampling_and_flush_preserve_duration() {
    for rate in [44_100, 48_000, 96_000] {
        let mut pipeline = Pipeline::new(
            AudioFormat::new(rate, 1).expect("source"),
            AudioFormat::new(48_000, 2).expect("output"),
        )
        .expect("pipeline");
        let samples: Vec<f32> = (0..rate)
            .map(|n| (n as f32 * std::f32::consts::TAU * 440.0 / rate as f32).sin() * 0.2)
            .collect();
        let mut output = Vec::new();
        for packet in samples.chunks(137) {
            output.extend(pipeline.process(packet).expect("resample"));
        }
        output.extend(pipeline.finish().expect("drain"));
        assert_eq!(output.len(), 96_000, "source rate {rate}");
        assert!(output.as_chunks::<2>().0.iter().all(|p| p[0] == p[1]));
        assert!(output.iter().all(|v| v.is_finite() && v.abs() < 0.3));
        assert!(pipeline.finish().expect("second drain").is_empty());
        assert!(pipeline.process(&[0.0]).is_err());
    }
}

#[test]
fn gain_mute_and_soft_limit_never_emit_invalid_device_samples() {
    let gain = Gain::default();
    assert!(gain.set_volume(f32::NAN).is_err());
    assert!(gain.set_preamp(f32::INFINITY).is_err());
    gain.set_preamp(12.0).expect("preamp");
    let (mut tx, mut rx, _) = audio_buffer(
        AudioFormat::new(48_000, 1).expect("format"),
        4,
        gain.clone(),
    )
    .expect("buffer");
    tx.write(&[1.0, -1.0, 0.2, 0.2]).expect("samples");
    let mut out = [0.0_f32; 2];
    rx.render(&mut out);
    assert!(out[0] > 0.9 && out[0] <= 1.0);
    assert!(out[1] < -0.9 && out[1] >= -1.0);
    gain.set_muted(true);
    rx.render(&mut out);
    assert_eq!(out, [0.0, 0.0]);
}

#[test]
fn decode_generated_wav_and_seek_to_a_sample_boundary() {
    let mut file = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .expect("file");
    file.write_all(&wav(48_000)).expect("fixture");
    let mut decoder = AudioDecoder::open(file.path()).expect("decoder");
    assert_eq!(decoder.info().format.rate(), 48_000);
    assert_eq!(decoder.info().duration, Some(1.0));
    let mut samples = 0;
    while let Some(packet) = decoder.next_packet().expect("packet") {
        samples += packet.samples.len();
    }
    assert_eq!(samples, 48_000);
    decoder.seek(0.5).expect("seek");
    let packet = decoder
        .next_packet()
        .expect("decode seek")
        .expect("samples");
    assert!((packet.position - 0.5).abs() < 1.0 / 48_000.0);
    assert!(decoder.seek(-1.0).is_err());
    assert!(decoder.seek(f64::NAN).is_err());
}

#[test]
fn corrupt_and_empty_audio_return_errors() {
    for bytes in [vec![], vec![0xff; 4096], wav(0)] {
        let mut file = tempfile::NamedTempFile::new().expect("file");
        file.write_all(&bytes).expect("write");
        if let Ok(mut decoder) = AudioDecoder::open(file.path()) {
            assert!(decoder.next_packet().is_err());
        }
    }
}

#[test]
fn truncated_pcm_is_reported_instead_of_successful_eof() {
    let mut bytes = wav(48_000);
    bytes.truncate(bytes.len() / 2);
    let mut file = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .expect("file");
    file.write_all(&bytes).expect("write");
    if let Ok(mut decoder) = AudioDecoder::open(file.path()) {
        let mut failed = false;
        loop {
            match decoder.next_packet() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "truncated PCM must not look like clean EOF");
    }
}

#[test]
fn decode_formats_generated_by_ffmpeg() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let source = dir.path().join("source.wav");
    std::fs::write(&source, wav(12_000)).expect("source");
    for (ext, codec) in [
        ("mp3", "libmp3lame"),
        ("flac", "flac"),
        ("ogg", "libvorbis"),
        ("m4a", "aac"),
        ("m4a", "alac"),
    ] {
        let path = dir.path().join(format!("{codec}.{ext}"));
        let result = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-i"])
            .arg(&source)
            .args(["-c:a", codec, "-metadata", "title=Generated tone"])
            .arg(&path)
            .output()
            .expect("ffmpeg");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut decoder = AudioDecoder::open(&path).expect("codec");
        assert_eq!(
            decoder.info().tags.title.as_deref(),
            Some("Generated tone"),
            "{codec}"
        );
        let mut total = 0;
        while let Some(packet) = decoder.next_packet().expect("decode packet") {
            total += packet.samples.len();
        }
        assert!(total >= 10_000, "{codec}: {total}");
    }
}

/// Test-only tool lookup: the first `name` on `PATH`.
fn on_path(name: &str) -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&exe))
        .find(|path| path.is_file())
}

#[test]
fn formats_symphonia_cannot_read_play_through_ffmpeg() {
    fm_media::set_tool_locator(on_path);
    if on_path("ffmpeg").is_none() || on_path("ffprobe").is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let source = dir.path().join("source.wav");
    std::fs::write(&source, wav(96_000)).expect("source"); // 2 s, mono, 48 kHz
    // Raw AC-3 has no container, so no tags.
    for (ext, codec, tagged) in [
        ("wma", "wmav2", true),
        ("opus", "libopus", true),
        ("ac3", "ac3", false),
    ] {
        let path = dir.path().join(format!("tone.{ext}"));
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-i"])
            .arg(&source)
            .args(["-c:a", codec, "-metadata", "title=Generated tone"])
            .arg(&path)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            continue; // this ffmpeg lacks the encoder
        }
        let mut decoder = AudioDecoder::open(&path).expect(ext);
        assert!(decoder.uses_ffmpeg(), "{ext} is not a Symphonia format");
        let info = decoder.info().clone();
        if tagged {
            assert_eq!(info.tags.title.as_deref(), Some("Generated tone"), "{ext}");
        }
        assert!(
            info.duration.is_some_and(|d| (d - 2.0).abs() < 0.2),
            "{ext}: {:?}",
            info.duration
        );
        let frames = |decoder: &mut AudioDecoder| {
            let mut frames = 0;
            let mut first = None;
            while let Some(packet) = decoder.next_packet().expect("decode") {
                first.get_or_insert(packet.position);
                assert!(packet.samples.iter().all(|s| s.is_finite()));
                frames += packet.samples.len() / info.format.channels();
            }
            (frames, first)
        };
        let (all, _) = frames(&mut decoder);
        let rate = f64::from(info.format.rate());
        assert!((all as f64 / rate - 2.0).abs() < 0.2, "{ext}: {all} frames");
        // Seeking restarts the decoder at the new position.
        decoder.seek(1.0).expect("seek");
        let (rest, first) = frames(&mut decoder);
        assert_eq!(first, Some(1.0), "{ext}");
        assert!(
            (rest as f64 / rate - 1.0).abs() < 0.2,
            "{ext}: {rest} frames after seek"
        );
        assert!(decoder.seek(f64::NAN).is_err());
    }
    // Not audio at all: a clear error, not a hang or panic.
    let junk = dir.path().join("junk.wma");
    std::fs::write(&junk, vec![0x42; 4096]).expect("junk");
    assert!(AudioDecoder::open(&junk).is_err());
    assert!(AudioDecoder::open(&dir.path().join("missing.wma")).is_err());
}
