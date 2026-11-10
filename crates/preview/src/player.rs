//! Bounded video/PCM decoding. A worker owns the children and native output; the UI
//! only reads the newest due frame. Dropping the handle signals cancellation.

mod pipe;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use fm_media::{AudioFormat, AudioProducer, AudioSink, Clock, DeviceSink, Gain, Pipeline};
use fm_vfs::FmError;

use crate::VideoInfo;
use crate::video::{command, find_tool, safe_path};
use pipe::Pipe;

pub const MAX_WIDTH: u32 = 1280;
pub const MIN_WIDTH: u32 = 160;
const DECODE_TIMEOUT: Duration = Duration::from_secs(10);
static WORKERS: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());

#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub seq: u64,
}

#[derive(Clone, Debug)]
pub struct PlaySpec {
    pub info: VideoInfo,
    pub from: f64,
    pub width: u32,
    pub speed: f64,
    pub still: bool,
}

#[derive(Debug, Default)]
struct Snapshot {
    latest: Option<Arc<Frame>>,
    position: f64,
    done: bool,
    error: Option<String>,
    hint: Option<String>,
}

#[derive(Debug, Default)]
struct Shared {
    snapshot: Mutex<Snapshot>,
    cancelled: AtomicBool,
}

#[derive(Debug)]
pub struct Player {
    shared: Arc<Shared>,
    pub still: bool,
}

type Output = (Box<dyn AudioSink>, AudioProducer, Clock);

/// Called once during application shutdown, not from a rendering frame.
/// Include cancelled/replaced players so their subprocesses cannot outlive a normal quit.
pub fn shutdown() -> Result<(), FmError> {
    let workers: Vec<_> = WORKERS
        .lock()
        .map_err(|_| error("Video worker list unavailable"))?
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
    for shared in &workers {
        shared.cancelled.store(true, Ordering::Release);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut done = true;
        for shared in &workers {
            done &= shared
                .snapshot
                .lock()
                .map_err(|_| error("Video status unavailable"))?
                .done;
        }
        if done {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(error("Timed out stopping video workers during shutdown"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn error(message: impl Into<String>) -> FmError {
    FmError::backend("video playback", message)
}

/// Even dimensions, bounded in both directions before any frame is allocated.
pub fn decode_size(info: &VideoInfo, width: u32) -> Option<(u32, u32)> {
    if info.width == 0 || info.height == 0 || info.width > 16_384 || info.height > 16_384 {
        return None;
    }
    let w = (width.clamp(MIN_WIDTH, MAX_WIDTH) / 2 * 2).min(info.width.max(2) / 2 * 2);
    let h = ((u64::from(w) * u64::from(info.height) / u64::from(info.width)) / 2 * 2).max(2);
    let h = u32::try_from(h).ok()?;
    (u64::from(w) * u64::from(h) <= 4_194_304).then_some((w, h))
}

pub fn atempo_chain(speed: f64) -> Option<String> {
    if !speed.is_finite() || !(0.5..=3.0).contains(&speed) {
        return None;
    }
    let mut remaining = speed;
    let mut stages = Vec::new();
    while remaining > 2.0 {
        stages.push(2.0);
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        stages.push(0.5);
        remaining /= 0.5;
    }
    stages.push(remaining);
    Some(
        stages
            .into_iter()
            .map(|stage| format!("atempo={stage:.6}"))
            .collect::<Vec<_>>()
            .join(","),
    )
}

impl Player {
    pub fn start(spec: &PlaySpec, gain: Gain) -> Result<Self, FmError> {
        Self::start_with(spec, gain, |gain| {
            let (sink, writer, clock) = DeviceSink::open(gain)?;
            Ok((Box::new(sink), writer, clock))
        })
    }

    fn start_with(
        spec: &PlaySpec,
        gain: Gain,
        open: impl FnOnce(Gain) -> Result<Output, FmError> + Send + 'static,
    ) -> Result<Self, FmError> {
        if !safe_path(&spec.info.path)
            || !spec.from.is_finite()
            || spec.from < 0.0
            || !spec.info.fps.is_finite()
            || !(0.0..=120.0).contains(&spec.info.fps)
            || !spec.info.audio_delay.is_finite()
            || spec.info.audio_delay < 0.0
            || !tracks_valid(&spec.info)
            || atempo_chain(spec.speed).is_none()
            || decode_size(&spec.info, spec.width).is_none()
        {
            return Err(error("Invalid video path, position or dimensions"));
        }
        let shared = Arc::new(Shared::default());
        {
            let mut workers = WORKERS
                .lock()
                .map_err(|_| error("Video worker list unavailable"))?;
            workers.retain(|worker| worker.strong_count() > 0);
            if workers.len() >= 64 {
                return Err(error(
                    "Too many video workers are still stopping. Wait and retry.",
                ));
            }
            workers.push(Arc::downgrade(&shared));
        }
        shared
            .snapshot
            .lock()
            .map_err(|_| error("Video status unavailable"))?
            .position = spec.from;
        let sh = shared.clone();
        let work = spec.clone();
        std::thread::Builder::new()
            .name("preview-video".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&work, gain, &sh, open)
                }))
                .unwrap_or_else(|_| Err(error("Video playback worker failed unexpectedly")));
                if let Ok(mut snapshot) = sh.snapshot.lock() {
                    snapshot.done = true;
                    if let Err(err) = result {
                        snapshot.error = Some(err.to_string());
                    }
                }
            })?;
        Ok(Self {
            shared,
            still: spec.still,
        })
    }

    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.shared
            .snapshot
            .lock()
            .ok()
            .and_then(|s| s.latest.clone())
    }

    pub fn position(&self) -> f64 {
        self.shared.snapshot.lock().map_or(0.0, |s| s.position)
    }

    pub fn finished(&self) -> bool {
        self.shared.snapshot.lock().is_ok_and(|s| s.done)
    }

    pub fn error(&self) -> Option<String> {
        self.shared
            .snapshot
            .lock()
            .ok()
            .and_then(|s| s.error.clone())
    }

    pub fn hint(&self) -> Option<String> {
        self.shared
            .snapshot
            .lock()
            .ok()
            .and_then(|s| s.hint.clone())
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

fn tracks_valid(info: &VideoInfo) -> bool {
    let tracks = &info.tracks;
    tracks.audio.len() <= crate::MAX_TRACKS
        && (tracks.audio.is_empty() || tracks.selected_audio < tracks.audio.len())
        && tracks
            .audio
            .iter()
            .all(|track| track.delay.is_finite() && track.delay >= 0.0)
}

fn video_pipe(tool: &std::path::Path, spec: &PlaySpec) -> Result<Pipe, FmError> {
    let (w, h) = decode_size(&spec.info, spec.width).ok_or_else(|| error("Invalid video size"))?;
    let fps = spec.info.fps.max(1.0);
    let mut c = command(tool);
    c.args([
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-ss",
    ])
    .arg(format!("{:.6}", spec.from))
    .arg("-i")
    .arg(&spec.info.path)
    .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
    // Raw frames have no PTS: normalize VFR to the probed, bounded cadence.
    .arg(format!("fps=fps={fps:.9}:start_time=0,scale={w}:{h}"))
    .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-threads", "1"]);
    if spec.still {
        c.args(["-frames:v", "1"]);
    }
    c.arg("-");
    Pipe::spawn(c, w as usize * h as usize * 4, DECODE_TIMEOUT)
}

fn audio_pipe(
    tool: &std::path::Path,
    spec: &PlaySpec,
    format: AudioFormat,
) -> Result<Pipe, FmError> {
    let mut c = command(tool);
    c.args([
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-ss",
    ])
    .arg(format!("{:.6}", spec.from))
    .arg("-i")
    .arg(&spec.info.path)
    .args(["-map"])
    .arg(format!("0:a:{}", spec.info.tracks.selected_audio))
    .args(["-vn", "-sn", "-dn"]);
    if (spec.speed - 1.0).abs() > f64::EPSILON {
        c.args(["-af"])
            .arg(atempo_chain(spec.speed).ok_or_else(|| error("Invalid playback speed"))?);
    }
    c.args(["-ac"]).arg(format.channels().to_string()).args([
        "-c:a",
        "pcm_s16le",
        "-f",
        "s16le",
        "-threads",
        "1",
        "-",
    ]);
    Pipe::spawn(c, 1024 * format.channels() * 2, DECODE_TIMEOUT)
}

struct Audio {
    pipe: Pipe,
    sink: Box<dyn AudioSink>,
    writer: AudioProducer,
    clock: Clock,
    pipeline: Pipeline,
    pending: Vec<f32>,
    offset: usize,
    flushed: bool,
    drained: Option<Instant>,
    last_position: f64,
    advanced: Instant,
    channels: usize,
    silence_frames: u64,
}

impl Audio {
    fn pump(&mut self) -> Result<bool, FmError> {
        self.sink.check_error()?;
        let position = self.clock.position();
        if position > self.last_position {
            self.last_position = position;
            self.advanced = Instant::now();
        }
        if self.advanced.elapsed() > DECODE_TIMEOUT && self.writer.queued_frames() > 0 {
            return Err(error(
                "Audio output stalled. Check the output device and retry.",
            ));
        }
        if self.offset < self.pending.len() {
            self.offset += self.writer.write(
                self.pending
                    .get(self.offset..)
                    .ok_or_else(|| error("Invalid PCM offset"))?,
            )?;
        } else if self.silence_frames > 0 {
            let frames = self.silence_frames.min(1024);
            self.silence_frames -= frames;
            self.pending = self
                .pipeline
                .process(&vec![0.0; frames as usize * self.channels])?;
            self.offset = 0;
        } else if let Some(bytes) = self.pipe.poll()? {
            if !bytes.len().is_multiple_of(self.channels * 2) {
                return Err(error("Truncated audio frame"));
            }
            let samples: Vec<_> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
                .collect();
            self.pending = self.pipeline.process(&samples)?;
            self.offset = 0;
        } else if self.pipe.ended() && !self.flushed {
            self.pending = self.pipeline.finish()?;
            self.offset = 0;
            self.flushed = true;
        }
        if self.flushed && self.offset == self.pending.len() && self.writer.queued_frames() == 0 {
            let until = self
                .drained
                .get_or_insert_with(|| Instant::now() + self.clock.latency());
            return Ok(Instant::now() >= *until);
        }
        Ok(false)
    }
}

fn run(
    spec: &PlaySpec,
    gain: Gain,
    shared: &Shared,
    open: impl FnOnce(Gain) -> Result<Output, FmError>,
) -> Result<(), FmError> {
    let tool: PathBuf =
        find_tool("ffmpeg").ok_or_else(|| error("Install ffmpeg to play this video"))?;
    if shared.cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut audio = None;
    if !spec.still
        && let Some((source, audio_delay)) = spec.info.audio_source()
    {
        match open(gain) {
            Ok((sink, writer, clock)) => {
                if shared.cancelled.load(Ordering::Acquire) {
                    return Ok(());
                }
                // ffmpeg knows the source speaker layout and performs multichannel downmix.
                let input = AudioFormat::new(source.rate(), source.channels().min(2))?;
                let pipeline = Pipeline::new(input, sink.format())?;
                let pipe = audio_pipe(&tool, spec, input)?;
                sink.play()?;
                audio = Some(Audio {
                    pipe,
                    sink,
                    writer,
                    clock,
                    pipeline,
                    pending: Vec::new(),
                    offset: 0,
                    flushed: false,
                    drained: None,
                    last_position: 0.0,
                    advanced: Instant::now(),
                    channels: input.channels(),
                    silence_frames: ((audio_delay - spec.from).max(0.0) / spec.speed
                        * f64::from(input.rate()))
                    .round() as u64,
                });
            }
            Err(err) => {
                shared
                    .snapshot
                    .lock()
                    .map_err(|_| error("Video status unavailable"))?
                    .hint = Some(format!("Playing silently: {err}"));
            }
        }
    }
    if shared.cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut video = video_pipe(&tool, spec)?;
    let (width, height) =
        decode_size(&spec.info, spec.width).ok_or_else(|| error("Invalid video size"))?;
    let fps = spec.info.fps.max(1.0);
    let mut next = None;
    let mut sequence = 0_u64;
    let mut wall = None;
    let mut tail = None;
    loop {
        if shared.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(a) = &mut audio
            && a.pump()?
        {
            tail = Some((
                (a.last_position + a.clock.latency().as_secs_f64()) * spec.speed,
                Instant::now(),
            ));
            audio = None;
        }
        if next.is_none()
            && let Some(rgba) = video.poll()?
        {
            if rgba.len() != width as usize * height as usize * 4 {
                return Err(error("Truncated video frame"));
            }
            next = Some(Arc::new(Frame {
                width,
                height,
                rgba,
                seq: sequence,
            }));
            sequence += 1;
            wall.get_or_insert_with(Instant::now);
        }
        let elapsed = if spec.still {
            0.0
        } else if let Some(a) = &audio {
            a.clock.position()
        } else if let Some((from, at)) = tail {
            from + at.elapsed().as_secs_f64()
        } else {
            wall.map_or(0.0, |at| at.elapsed().as_secs_f64())
        };
        let media_elapsed = elapsed * spec.speed;
        let due = next
            .as_ref()
            .is_some_and(|frame| spec.still || frame.seq as f64 / fps <= media_elapsed);
        {
            let mut s = shared
                .snapshot
                .lock()
                .map_err(|_| error("Video status unavailable"))?;
            s.position = spec.from + media_elapsed;
            if due {
                s.latest = next.take();
            }
        }
        if video.ended() && next.is_none() && audio.is_none() {
            return Ok(());
        }
        // Catch up without waiting for the UI, but hold a future frame until the clock.
        if !due {
            std::thread::sleep(Duration::from_millis(3));
        }
    }
}

#[cfg(test)]
#[path = "player/tests.rs"]
mod tests;
