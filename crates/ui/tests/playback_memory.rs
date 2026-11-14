//! Manual memory probe (ignored; Linux): plays a generated MP3 through the real session,
//! muted, at ~60 frames per second, and prints the process's resident memory every
//! 10 seconds. Phase 1 polls the engine only; phase 2 also draws the Media pane.
//!
//! `FM_PROBE_SECS=120 cargo test -p fm-ui --test playback_memory -- --ignored --nocapture`
#![allow(clippy::expect_used, clippy::print_stderr)]

use std::time::{Duration, Instant};

use egui_kittest::Harness;
use fm_engine::Session;
use fm_ui::Tokens;
use serde_json::json;

/// Resident set size in KiB (`VmRSS`), Linux only.
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

fn report(phase: &str, start: Instant, base: u64, samples: &mut Vec<(f64, u64)>) {
    let t = start.elapsed().as_secs_f64();
    let rss = rss_kib();
    samples.push((t, rss));
    eprintln!(
        "{phase:>7} t={t:6.1}s rss={:8.1} MiB  (+{:6.1} MiB)",
        rss as f64 / 1024.0,
        (rss.saturating_sub(base)) as f64 / 1024.0
    );
}

fn slope(samples: &[(f64, u64)]) -> f64 {
    // Least squares, MiB per minute, ignoring the first sample (warm-up).
    let pts: Vec<(f64, f64)> = samples
        .iter()
        .skip(1)
        .map(|(t, k)| (*t, *k as f64 / 1024.0))
        .collect();
    let n = pts.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (num, den) = pts.iter().fold((0.0, 0.0), |a, p| {
        (a.0 + (p.0 - mx) * (p.1 - my), a.1 + (p.0 - mx) * (p.0 - mx))
    });
    if den == 0.0 { 0.0 } else { num / den * 60.0 }
}

fn probe_phase(h: &mut Harness<'_, Session>, phase: &str, secs: u64) -> f64 {
    let frame = Duration::from_millis(16);
    let start = Instant::now();
    let base = rss_kib();
    let mut samples = Vec::new();
    let mut next = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        h.step();
        if Instant::now() >= next {
            report(phase, start, base, &mut samples);
            let entries = h.ctx.memory(|m| m.data.len());
            let textures = h.ctx.tex_manager().read().num_allocated();
            eprintln!("          egui data entries={entries} textures={textures}");
            next += Duration::from_secs(10);
        }
        std::thread::sleep(frame);
    }
    slope(&samples)
}

#[test]
#[ignore = "manual memory probe: plays audio for minutes"]
fn audio_playback_memory() {
    let secs: u64 = std::env::var("FM_PROBE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let dir = tempfile::tempdir().expect("tempdir");
    let mp3 = dir.path().join("tone.mp3");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={}", secs * 3 + 60))
        .args(["-ac", "2", "-c:a", "libmp3lame", "-q:a", "6"])
        .arg(&mp3)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("skipping: ffmpeg with libmp3lame is needed");
        return;
    }
    let uri = mp3.to_str().expect("utf8 path").to_string();
    let mut s = Session::new().expect("session");
    s.dispatch("media.mute", &json!({"on": true}))
        .expect("mute");
    s.dispatch("media.enqueue", &json!({ "uris": [uri] }))
        .expect("enqueue");
    let pane = s.media_pane().expect("media pane");
    let tokens = Tokens::dark();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                s.poll();
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                egui::CentralPanel::default().show(ui, |ui| {
                    fm_ui::media::show(ui, s, pane, &tokens);
                });
                fm_ui::frame_cache::sweep(ui.ctx());
            },
            s,
        );
    let idle = probe_phase(&mut h, "idle", secs);
    h.state_mut()
        .dispatch("media.play", &json!({"index": 0}))
        .expect("play");
    let playing = probe_phase(&mut h, "playing", secs);
    eprintln!("position {:.1}s", h.state().media.position);
    h.state_mut()
        .dispatch("media.stop", &serde_json::Value::Null)
        .expect("stop");
    let stopped = probe_phase(&mut h, "stopped", secs);
    eprintln!("slope MiB/min: idle {idle:+.2}, playing {playing:+.2}, stopped {stopped:+.2}");
}
