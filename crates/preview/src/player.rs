//! Bounded video/PCM decoding. A worker owns the children and native output; the UI
//! only reads the newest due frame. Dropping the handle signals cancellation.

mod pipe;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use fm_media::{AudioFormat, AudioProducer, AudioSink, Clock, DeviceSink, Gain, Pipeline};
use fm_vfs::FmError;

use crate::VideoInfo;
use crate::video::{DEINTERLACE, command, find_tool};
use pipe::Pipe;

pub const MAX_WIDTH: u32 = 1280;
pub const MIN_WIDTH: u32 = 160;
const DECODE_TIMEOUT: Duration = Duration::from_secs(10);
/// Picture subtitles shown at the seek point may have started this long before it.
const SUBTITLE_PREROLL: f64 = 2.0;
/// Hardware failures this close to the end are damaged data at the end of the file (a DVD's
/// VOB parts end mid-group); software decodes the rest quietly.
const END_TAIL: f64 = 2.0;
/// A changed subtitle overlay restarts the video decoder at most this often.
const OVERLAY_RESTART: Duration = Duration::from_millis(250);
/// Scale of picture subtitles relative to their authored size.
pub const OVERLAY_SCALE: std::ops::RangeInclusive<f64> = 0.5..=1.5;
/// Largest upward shift of picture subtitles, as a fraction of the picture height.
pub const OVERLAY_LIFT_MAX: f64 = 0.45;
static WORKERS: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());
/// One bit per [`HwAccel`] whose decode device could not be opened at all; later videos
/// skip that method.
static HARDWARE_UNAVAILABLE: AtomicU8 = AtomicU8::new(0);

/// Platform video decode acceleration, passed to ffmpeg as `-hwaccel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwAccel {
    Vaapi,
    D3d11va,
    VideoToolbox,
    /// A method ffmpeg does not know, to exercise the software fallback.
    #[cfg(test)]
    Missing,
}

impl HwAccel {
    /// VA-API on Linux and the BSDs, Direct3D 11 on Windows, VideoToolbox on macOS.
    pub fn for_platform() -> Self {
        if cfg!(windows) {
            Self::D3d11va
        } else if cfg!(target_os = "macos") {
            Self::VideoToolbox
        } else {
            Self::Vaapi
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Vaapi => "vaapi",
            Self::D3d11va => "d3d11va",
            Self::VideoToolbox => "videotoolbox",
            #[cfg(test)]
            Self::Missing => "fm-missing-hwaccel",
        }
    }

    fn bit(self) -> u8 {
        match self {
            Self::Vaapi => 1,
            Self::D3d11va => 2,
            Self::VideoToolbox => 4,
            #[cfg(test)]
            Self::Missing => 8,
        }
    }

    fn unavailable(self) -> bool {
        HARDWARE_UNAVAILABLE.load(Ordering::Acquire) & self.bit() != 0
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Vaapi => "VA-API",
            Self::D3d11va => "Direct3D 11",
            Self::VideoToolbox => "VideoToolbox",
            #[cfg(test)]
            Self::Missing => "missing",
        }
    }
}

/// How the running decoder produces frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoding {
    Hardware(HwAccel),
    Software,
}

/// Picture-based subtitles (PGS, VobSub) that ffmpeg draws into the frames. Their colours
/// are part of the picture; only their size and height can change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    /// Index among the video's subtitle streams (`0:s:N`).
    pub stream: usize,
    /// Size relative to the authored picture, within [`OVERLAY_SCALE`]; anchored at the bottom.
    pub scale: f64,
    /// Upward shift as a fraction of the picture height, `0.0..=OVERLAY_LIFT_MAX`.
    pub lift: f64,
}

impl Overlay {
    fn valid(&self, info: &VideoInfo) -> bool {
        self.stream < info.tracks.subtitles.len().min(crate::MAX_TRACKS)
            && self.scale.is_finite()
            && OVERLAY_SCALE.contains(&self.scale)
            && self.lift.is_finite()
            && (0.0..=OVERLAY_LIFT_MAX).contains(&self.lift)
    }
}

/// Try the decode devices again, for example after hardware decoding was switched back on.
pub fn reset_hardware() {
    HARDWARE_UNAVAILABLE.store(0, Ordering::Release);
}

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
    /// Try this decode acceleration first; `None` decodes in software.
    pub hardware: Option<HwAccel>,
    /// Picture subtitles drawn into the frames.
    pub overlay: Option<Overlay>,
}

#[derive(Debug, Default)]
struct Snapshot {
    latest: Option<Arc<Frame>>,
    position: f64,
    done: bool,
    error: Option<String>,
    hint: Option<String>,
    /// Known once the first frame has been decoded.
    decoding: Option<Decoding>,
    /// Why hardware decoding stopped during playback, until the owner takes it.
    fallback: Option<String>,
    /// The overlay the owner wants; the worker restarts its video decoder to match.
    overlay: Option<Overlay>,
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

/// Decoders still running (diagnostic log): started players whose worker has not finished.
pub fn live_players() -> usize {
    WORKERS.lock().map_or(0, |workers| {
        workers
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|shared| shared.snapshot.lock().is_ok_and(|s| !s.done))
            .count()
    })
}

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
        if spec.info.ffmpeg_input().is_none()
            || !spec.from.is_finite()
            || spec.from < 0.0
            || !spec.info.fps.is_finite()
            || !(0.0..=120.0).contains(&spec.info.fps)
            || !spec.info.audio_delay.is_finite()
            || spec.info.audio_delay < 0.0
            || !tracks_valid(&spec.info)
            || spec
                .overlay
                .is_some_and(|overlay| !overlay.valid(&spec.info))
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
        {
            let mut snapshot = shared
                .snapshot
                .lock()
                .map_err(|_| error("Video status unavailable"))?;
            snapshot.position = spec.from;
            snapshot.overlay = spec.overlay;
        }
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

    /// Hardware or software, once the first frame is decoded.
    pub fn decoding(&self) -> Option<Decoding> {
        self.shared.snapshot.lock().ok().and_then(|s| s.decoding)
    }

    /// Why hardware decoding stopped during playback; returned once.
    pub fn take_fallback(&self) -> Option<String> {
        self.shared
            .snapshot
            .lock()
            .ok()
            .and_then(|mut s| s.fallback.take())
    }

    /// Draw `overlay` from now on. A playing video restarts only its video decoder at the
    /// current position, so audio continues; invalid overlays are ignored.
    pub fn set_overlay(&self, overlay: Option<Overlay>, info: &VideoInfo) {
        if overlay.is_some_and(|overlay| !overlay.valid(info)) {
            return;
        }
        if let Ok(mut snapshot) = self.shared.snapshot.lock() {
            snapshot.overlay = overlay;
        }
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

/// A video decoder from `at` seconds. `exact` seeks after the input: slower, but works on
/// files whose only keyframe is at the start.
fn video_pipe(
    tool: &std::path::Path,
    spec: &PlaySpec,
    at: f64,
    exact: bool,
    hardware: Option<HwAccel>,
    overlay: Option<Overlay>,
) -> Result<Pipe, FmError> {
    let (w, h) = decode_size(&spec.info, spec.width).ok_or_else(|| error("Invalid video size"))?;
    let c = video_command(tool, spec, at, exact, hardware, overlay)?;
    Pipe::spawn(c, w as usize * h as usize * 4, DECODE_TIMEOUT)
}

fn video_command(
    tool: &std::path::Path,
    spec: &PlaySpec,
    at: f64,
    exact: bool,
    hardware: Option<HwAccel>,
    overlay: Option<Overlay>,
) -> Result<std::process::Command, FmError> {
    let (w, h) = decode_size(&spec.info, spec.width).ok_or_else(|| error("Invalid video size"))?;
    let fps = spec.info.fps.max(1.0);
    let (protocols, input) = spec
        .info
        .ffmpeg_input()
        .ok_or_else(|| error("Invalid video path"))?;
    let mut c = command(tool);
    c.stderr(Stdio::piped());
    c.args([
        "-v",
        "warning",
        "-nostdin",
        "-protocol_whitelist",
        protocols,
    ]);
    if let Some(hardware) = hardware {
        // Frames come back to system memory, so the software filters below still apply.
        c.args(["-hwaccel", hardware.flag()]);
    }
    // A subtitle on screen at `at` may have started earlier: decode a little before it.
    let preroll = if overlay.is_some() {
        at.min(SUBTITLE_PREROLL)
    } else {
        0.0
    };
    if !exact {
        c.args(["-ss"]).arg(format!("{:.6}", at - preroll));
    }
    c.arg("-i").arg(input);
    if exact {
        c.args(["-ss"]).arg(format!("{at:.6}"));
    } else if preroll > 0.0 {
        c.args(["-ss"]).arg(format!("{preroll:.6}"));
    }
    // Raw frames have no PTS: normalize VFR to the probed, bounded cadence.
    let cadence = format!("fps=fps={fps:.9}:start_time=0");
    match overlay {
        None => {
            c.args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
                .arg(format!("{DEINTERLACE},{cadence},scale={w}:{h}"));
        }
        Some(overlay) => {
            let ow = ((f64::from(w) * overlay.scale) as u32 / 2 * 2).max(2);
            let oh = ((f64::from(h) * overlay.scale) as u32 / 2 * 2).max(2);
            let lift = (f64::from(h) * overlay.lift).round() as u32;
            c.arg("-filter_complex")
                .arg(format!(
                    "[0:v:0]{DEINTERLACE},scale={w}:{h}[v];[0:s:{stream}]scale={ow}:{oh}[s];\
                     [v][s]overlay=x=(W-w)/2:y=H-h-{lift}:eof_action=pass,{cadence}[out]",
                    stream = overlay.stream
                ))
                .args(["-map", "[out]"]);
        }
    }
    c.args(["-pix_fmt", "rgba", "-f", "rawvideo", "-threads", "1"]);
    if spec.still {
        c.args(["-frames:v", "1"]);
    }
    c.arg("-");
    Ok(c)
}

/// `at` seconds lies in the last [`END_TAIL`] of a video `duration` long (when known).
fn near_end(at: f64, duration: f64) -> bool {
    duration.is_finite() && duration > 0.0 && at >= duration - END_TAIL
}

fn strip_prefixes(line: &str) -> &str {
    let mut rest = line.trim();
    while let Some(after) = rest.strip_prefix('[') {
        match after.split_once(']') {
            Some((_, tail)) => rest = tail.trim_start(),
            None => break,
        }
    }
    rest
}

/// One running video decoder and where its frames belong on the timeline.
struct Video {
    pipe: Pipe,
    /// Media seconds after `spec.from` that this decoder's first frame shows.
    origin: f64,
    /// Frames read from this decoder.
    frames: u64,
    exact: bool,
    hardware: Option<HwAccel>,
    overlay: Option<Overlay>,
    started: Instant,
}

impl Video {
    fn spawn(
        tool: &std::path::Path,
        spec: &PlaySpec,
        origin: f64,
        exact: bool,
        hardware: Option<HwAccel>,
        overlay: Option<Overlay>,
    ) -> Result<Self, FmError> {
        Ok(Self {
            pipe: video_pipe(tool, spec, spec.from + origin, exact, hardware, overlay)?,
            origin,
            frames: 0,
            exact,
            hardware,
            overlay,
            started: Instant::now(),
        })
    }

    /// Timeline position (after `spec.from`) of the next frame this decoder delivers.
    fn next_time(&self, fps: f64) -> f64 {
        self.origin + self.frames as f64 / fps
    }

    fn decoding(&self) -> Decoding {
        match self.hardware {
            // ffmpeg itself falls back to software for a codec the device cannot decode.
            Some(hardware) if !self.pipe.diagnostics().contains("Failed setup for format") => {
                Decoding::Hardware(hardware)
            }
            _ => Decoding::Software,
        }
    }
}

/// The decode device itself is missing (as opposed to one file failing).
fn device_unavailable(diagnostics: &str) -> bool {
    [
        "Device creation failed",
        "No device available",
        "Failed to initialise",
        "Unrecognized hwaccel",
        "hwaccel initialisation returned error",
    ]
    .iter()
    .any(|needle| diagnostics.contains(needle))
}

/// The last diagnostic line, credential-redacted and without ffmpeg's `[component @ 0x…]`
/// prefixes, for the message log.
fn fallback_reason(hardware: HwAccel, diagnostics: &str, err: &FmError) -> String {
    let lines: Vec<&str> = diagnostics
        .lines()
        .map(strip_prefixes)
        .filter(|line| !line.is_empty())
        .collect();
    let recent = lines
        .get(lines.len().saturating_sub(12)..)
        .unwrap_or_default();
    // ffmpeg ends with generic "Terminating thread…" lines; the cause comes before them.
    let generic = [
        "Terminating thread",
        "Task finished",
        "Error processing packet",
        "Error while processing",
    ];
    let detail = recent
        .iter()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            (lower.contains("fail") || lower.contains("error"))
                && !generic.iter().any(|g| line.starts_with(g))
        })
        .or_else(|| recent.last())
        .map_or_else(|| err.to_string(), |line| (*line).to_string());
    let detail: String = fm_vfs::redact::redact(&detail)
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    format!(
        "Hardware video decoding ({}) stopped; continuing in software: {detail}",
        hardware.label()
    )
}

fn audio_pipe(
    tool: &std::path::Path,
    spec: &PlaySpec,
    format: AudioFormat,
) -> Result<Pipe, FmError> {
    let (protocols, input) = spec
        .info
        .ffmpeg_input()
        .ok_or_else(|| error("Invalid video path"))?;
    let mut c = command(tool);
    c.args([
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        protocols,
        "-ss",
    ])
    .arg(format!("{:.6}", spec.from))
    .arg("-i")
    .arg(input)
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
    let tool: PathBuf = find_tool("ffmpeg").ok_or_else(|| error(crate::video::missing_hint()))?;
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
    let fps = spec.info.fps.max(1.0);
    let hardware = spec.hardware.filter(|accel| !accel.unavailable());
    let mut video = Video::spawn(&tool, spec, 0.0, false, hardware, spec.overlay)?;
    let (width, height) =
        decode_size(&spec.info, spec.width).ok_or_else(|| error("Invalid video size"))?;
    // The decoded frame waiting for its time, and that time (seconds after `spec.from`).
    let mut next: Option<(Arc<Frame>, f64)> = None;
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
        if next.is_none() {
            let polled = video.pipe.poll();
            let rgba = match polled {
                Ok(frame) => frame,
                Err(err) => {
                    let origin = video.next_time(fps);
                    if let Some(hw) = video.hardware {
                        let diagnostics = video.pipe.diagnostics();
                        if sequence == 0 && device_unavailable(&diagnostics) {
                            HARDWARE_UNAVAILABLE.fetch_or(hw.bit(), Ordering::AcqRel);
                        }
                        {
                            let mut snapshot = shared
                                .snapshot
                                .lock()
                                .map_err(|_| error("Video status unavailable"))?;
                            // Software from here on, even if no frame follows (end of file).
                            if sequence > 0 {
                                snapshot.decoding = Some(Decoding::Software);
                            }
                            // Only a failure during playback is news; an unsupported file
                            // simply plays in software, and a damaged end of file is the
                            // file's fault, not the graphics card's.
                            if sequence > 0 && !near_end(spec.from + origin, spec.info.duration) {
                                snapshot.fallback = Some(fallback_reason(hw, &diagnostics, &err));
                            }
                        }
                        video =
                            Video::spawn(&tool, spec, origin, video.exact, None, video.overlay)?;
                        continue;
                    }
                    // Fast seek can land on a non-keyframe and decode nothing; retry exactly.
                    if !video.exact && video.frames == 0 && spec.from + origin > 0.0 {
                        video = Video::spawn(&tool, spec, origin, true, None, video.overlay)?;
                        continue;
                    }
                    return Err(err);
                }
            };
            if let Some(rgba) = rgba {
                if rgba.len() != width as usize * height as usize * 4 {
                    return Err(error("Truncated video frame"));
                }
                if video.frames == 0 {
                    shared
                        .snapshot
                        .lock()
                        .map_err(|_| error("Video status unavailable"))?
                        .decoding = Some(video.decoding());
                }
                let at = video.next_time(fps);
                video.frames += 1;
                next = Some((
                    Arc::new(Frame {
                        width,
                        height,
                        rgba,
                        seq: sequence,
                    }),
                    at,
                ));
                sequence += 1;
                wall.get_or_insert_with(Instant::now);
            }
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
            .is_some_and(|(_, at)| spec.still || *at <= media_elapsed);
        let wanted = {
            let mut s = shared
                .snapshot
                .lock()
                .map_err(|_| error("Video status unavailable"))?;
            s.position = spec.from + media_elapsed;
            if due {
                s.latest = next.take().map(|(frame, _)| frame);
            }
            s.overlay
        };
        if wanted != video.overlay
            && !spec.still
            && !video.pipe.ended()
            && video.started.elapsed() >= OVERLAY_RESTART
        {
            // New picture subtitles: restart only the video decoder, here and now.
            let origin = media_elapsed.max(video.origin);
            video = Video::spawn(&tool, spec, origin, false, video.hardware, wanted)?;
            next = None;
            continue;
        }
        if video.pipe.ended() && next.is_none() && audio.is_none() {
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
pub(crate) mod tests;
