use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fm_vfs::FmError;

use crate::{AudioDecoder, AudioInfo, AudioSink, DeviceSink, Gain, Pipeline};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Loading,
    Playing,
    Paused,
    Finished,
    Failed,
}

#[derive(Clone, Debug)]
pub struct PlaybackStatus {
    pub state: PlaybackState,
    pub info: Option<Arc<AudioInfo>>,
    pub position: f64,
    pub error: Option<String>,
    /// Token given to [`AudioPlayer::queue_next`] for the item now audible; 0 for the first.
    pub token: u64,
}

#[derive(Debug)]
struct Shared {
    status: Mutex<PlaybackStatus>,
    cancelled: AtomicBool,
    paused: AtomicBool,
}

/// Non-blocking playback handle. Opening files/devices and decoding happen on its worker.
/// Dropping the handle signals cancellation, but never joins a worker on the UI thread.
#[derive(Debug)]
pub struct AudioPlayer {
    shared: Arc<Shared>,
    seeks: SyncSender<f64>,
    nexts: SyncSender<Next>,
    gain: Gain,
}

#[derive(Debug)]
enum Next {
    Set(PathBuf, u64),
    Clear,
}

impl AudioPlayer {
    pub fn start(path: PathBuf, gain: Gain) -> Result<Self, FmError> {
        Self::start_at(path, gain, 0.0)
    }

    /// Start playing at `position` seconds; the status reports it from the first poll, so
    /// callers never see a jump back to zero while the first seek is processed.
    pub fn start_at(path: PathBuf, gain: Gain, position: f64) -> Result<Self, FmError> {
        if !position.is_finite() || position < 0.0 {
            return Err(FmError::InvalidParams(
                "start position must be finite and non-negative".into(),
            ));
        }
        let shared = Arc::new(Shared {
            status: Mutex::new(PlaybackStatus {
                state: PlaybackState::Loading,
                info: None,
                position,
                error: None,
                token: 0,
            }),
            cancelled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        });
        let (seeks, rx) = sync_channel(8);
        if position > 0.0 {
            seeks
                .try_send(position)
                .map_err(|e| crate::error(format!("Cannot request audio seek: {e}")))?;
        }
        let (nexts, next_rx) = sync_channel(4);
        let sh = shared.clone();
        let worker_gain = gain.clone();
        std::thread::Builder::new()
            .name("media-audio".into())
            .spawn(move || {
                // Third-party decoders process hostile bytes. Report a decoder panic explicitly.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(path, &sh, rx, next_rx, worker_gain, |gain| {
                        let (sink, producer, clock) = DeviceSink::open(gain)?;
                        Ok((Box::new(sink), producer, clock))
                    })
                }))
                .unwrap_or_else(|_| Err(crate::error("Audio decoder failed unexpectedly")));
                if let Err(err) = result
                    && let Ok(mut status) = sh.status.lock()
                {
                    status.state = PlaybackState::Failed;
                    status.error = Some(err.to_string());
                }
            })?;
        Ok(Self {
            shared,
            seeks,
            nexts,
            gain,
        })
    }

    pub fn status(&self) -> Result<PlaybackStatus, FmError> {
        self.shared
            .status
            .lock()
            .map(|s| s.clone())
            .map_err(|_| crate::error("Audio status lock was poisoned"))
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
    }

    pub fn seek(&self, position: f64) -> Result<(), FmError> {
        if !position.is_finite() || position < 0.0 {
            return Err(FmError::InvalidParams(
                "seek position must be finite and non-negative".into(),
            ));
        }
        self.seeks
            .try_send(position)
            .map_err(|e| crate::error(format!("Cannot request audio seek: {e}")))
    }

    /// Have the worker open `path` ahead of the end and continue into it on the same sink,
    /// without a gap. Replaces any earlier request; `token` appears in [`PlaybackStatus`]
    /// once the item is audible.
    pub fn queue_next(&self, path: PathBuf, token: u64) -> Result<(), FmError> {
        self.nexts
            .try_send(Next::Set(path, token))
            .map_err(|e| crate::error(format!("Cannot queue the next audio item: {e}")))
    }

    pub fn clear_next(&self) {
        let _ = self.nexts.try_send(Next::Clear);
    }

    pub fn gain(&self) -> &Gain {
        &self.gain
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

/// Seconds of audio before the end at which the next item is opened.
const PRE_OPEN: f64 = 5.0;

struct Audible {
    start: f64,
    from: f64,
}

struct Boundary {
    at: f64,
    token: u64,
    info: Arc<AudioInfo>,
}

fn run(
    path: PathBuf,
    shared: &Shared,
    seeks: Receiver<f64>,
    nexts: Receiver<Next>,
    gain: Gain,
    open: impl Fn(Gain) -> Result<(Box<dyn AudioSink>, crate::AudioProducer, crate::Clock), FmError>,
) -> Result<(), FmError> {
    let mut decoder = AudioDecoder::open(&path)?;
    let mut info = Arc::new(decoder.info().clone());
    shared
        .status
        .lock()
        .map_err(|_| crate::error("Audio status lock was poisoned"))?
        .info = Some(info.clone());
    let mut from = 0.0;
    let mut want: Option<(PathBuf, u64)> = None;
    let mut next: Option<(AudioDecoder, u64)> = None;
    let mut next_failed: Option<u64> = None;
    'segment: loop {
        if shared.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let (sink, mut producer, clock) = open(gain.clone())?;
        let out = sink.format();
        let mut pipeline = Pipeline::new(info.format, out)?;
        sink.play()?;
        let mut audible = Audible { start: 0.0, from };
        let mut boundaries: std::collections::VecDeque<Boundary> = Default::default();
        let mut written_frames = 0_u64;
        let mut pcm = Vec::new();
        let mut offset = 0;
        let mut ended = false;
        loop {
            if shared.cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            sink.check_error()?;
            while let Ok(request) = nexts.try_recv() {
                match request {
                    Next::Set(path, id) => {
                        if next.as_ref().is_some_and(|(_, current)| *current != id) {
                            next = None;
                        }
                        next_failed = None;
                        want = Some((path, id));
                    }
                    Next::Clear => {
                        want = None;
                        next = None;
                        next_failed = None;
                    }
                }
            }
            // A seek during the last buffered seconds would address the wrong item; the
            // boundary is at most half a second away, so it waits.
            if boundaries.is_empty() {
                match seeks.try_recv() {
                    Ok(mut position) => {
                        // Coalesce scrubbing requests without queueing device restarts.
                        while let Ok(next) = seeks.try_recv() {
                            position = next;
                        }
                        from = info.duration.map_or(position, |d| position.min(d));
                        decoder.seek(from)?;
                        continue 'segment;
                    }
                    Err(TryRecvError::Disconnected) => return Ok(()),
                    Err(TryRecvError::Empty) => {}
                }
            }
            let paused = shared.paused.load(Ordering::Acquire);
            clock.set_paused(paused);
            let played = clock.position();
            let position = {
                let mut status = shared
                    .status
                    .lock()
                    .map_err(|_| crate::error("Audio status lock was poisoned"))?;
                while boundaries.front().is_some_and(|b| played >= b.at)
                    && let Some(b) = boundaries.pop_front()
                {
                    audible = Audible {
                        start: b.at,
                        from: 0.0,
                    };
                    status.info = Some(b.info);
                    status.token = b.token;
                }
                status.state = if paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                };
                status.position = audible.from + (played - audible.start).max(0.0);
                status.position
            };
            if paused {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            if next.is_none()
                && boundaries.is_empty()
                && let Some((path, id)) = &want
                && next_failed != Some(*id)
                && (ended || info.duration.is_none_or(|d| position + PRE_OPEN >= d))
            {
                match AudioDecoder::open(path).and_then(|d| {
                    Pipeline::new(d.info().format, out)?;
                    Ok(d)
                }) {
                    Ok(d) => next = Some((d, *id)),
                    Err(_) => next_failed = Some(*id),
                }
            }
            if offset < pcm.len() {
                let written = producer.write(
                    pcm.get(offset..)
                        .ok_or_else(|| crate::error("Invalid PCM offset"))?,
                )?;
                offset += written;
                written_frames += (written / out.channels()) as u64;
            } else if !ended {
                pcm = match decoder.next_packet()? {
                    Some(packet) => pipeline.process(&packet.samples)?,
                    None => {
                        ended = true;
                        pipeline.finish()?
                    }
                };
                offset = 0;
            } else if let Some((following, id)) = next.take() {
                // Same sink, same ring: the next item's first frame follows the last one.
                let following_info = Arc::new(following.info().clone());
                pipeline = Pipeline::new(following_info.format, out)?;
                boundaries.push_back(Boundary {
                    at: written_frames as f64 / f64::from(out.rate()),
                    token: id,
                    info: following_info.clone(),
                });
                decoder = following;
                info = following_info;
                want = None;
                ended = false;
                pcm.clear();
                offset = 0;
                continue;
            } else if producer.queued_frames() == 0 {
                // The final callback only submitted the samples, not played them yet.
                let until = std::time::Instant::now() + clock.latency();
                while std::time::Instant::now() < until {
                    if shared.cancelled.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    sink.check_error()?;
                    std::thread::sleep(Duration::from_millis(3));
                }
                let mut status = shared
                    .status
                    .lock()
                    .map_err(|_| crate::error("Audio status lock was poisoned"))?;
                status.state = PlaybackState::Finished;
                status.position = info
                    .duration
                    .unwrap_or(audible.from + (played - audible.start).max(0.0));
                return Ok(());
            }
            if offset < pcm.len() || ended {
                std::thread::sleep(Duration::from_millis(3));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use crate::{AudioConsumer, AudioFormat, audio_buffer};

    struct FakeSink {
        consumer: Mutex<AudioConsumer>,
        format: AudioFormat,
        out: Arc<Mutex<Vec<f32>>>,
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
            let mut block = [0.0_f32; 64];
            self.consumer.lock().expect("consumer").render(&mut block);
            self.out.lock().expect("out").extend_from_slice(&block);
            Ok(())
        }
    }

    fn shared() -> Shared {
        Shared {
            status: Mutex::new(PlaybackStatus {
                state: PlaybackState::Loading,
                info: None,
                position: 0.0,
                error: None,
                token: 0,
            }),
            cancelled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        }
    }

    fn no_nexts() -> Receiver<Next> {
        sync_channel(1).1
    }

    fn fixture() -> tempfile::NamedTempFile {
        tone(480, 0)
    }

    /// Mono 48 kHz WAV of `frames` frames, all with sample value `value`.
    fn tone(frames: u32, value: i16) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::Builder::new()
            .suffix(".wav")
            .tempfile()
            .expect("file");
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&96_000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(frames * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        file.write_all(&bytes).expect("fixture");
        file
    }

    #[test]
    fn playback_worker_drains_to_finished_with_fake_device() {
        let file = fixture();
        let shared = shared();
        let (_tx, rx) = sync_channel(8);
        run(
            file.path().into(),
            &shared,
            rx,
            no_nexts(),
            Gain::default(),
            |gain| {
                let format = AudioFormat::new(48_000, 1)?;
                let (writer, consumer, clock) = audio_buffer(format, 128, gain)?;
                Ok((
                    Box::new(FakeSink {
                        consumer: Mutex::new(consumer),
                        format,
                        out: Arc::default(),
                    }),
                    writer,
                    clock,
                ))
            },
        )
        .expect("play");
        let status = shared.status.lock().expect("status");
        assert_eq!(status.state, PlaybackState::Finished);
        assert_eq!(status.position, 0.01);
    }

    #[test]
    fn cancelled_playback_never_opens_a_device() {
        let file = fixture();
        let shared = shared();
        shared.cancelled.store(true, Ordering::Release);
        let (_tx, rx) = sync_channel(8);
        run(
            file.path().into(),
            &shared,
            rx,
            no_nexts(),
            Gain::default(),
            |_| Err(crate::error("must not open device")),
        )
        .expect("cancelled");
    }

    #[test]
    fn device_failure_reaches_the_caller() {
        let file = fixture();
        let shared = shared();
        let (_tx, rx) = sync_channel(8);
        let error = run(
            file.path().into(),
            &shared,
            rx,
            no_nexts(),
            Gain::default(),
            |_| Err(crate::error("no audio output device")),
        )
        .expect_err("missing device");
        assert!(error.to_string().contains("no audio output device"));
    }

    fn play_two(
        first: &std::path::Path,
        second: Option<&std::path::Path>,
    ) -> (Vec<f32>, PlaybackStatus) {
        let shared = shared();
        let (_tx, rx) = sync_channel(8);
        let (next_tx, next_rx) = sync_channel(4);
        if let Some(second) = second {
            next_tx
                .send(Next::Set(second.into(), 7))
                .expect("queue next");
        }
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink_out = out.clone();
        run(
            first.into(),
            &shared,
            rx,
            next_rx,
            Gain::default(),
            move |gain| {
                let format = AudioFormat::new(48_000, 1)?;
                let (writer, consumer, clock) = audio_buffer(format, 48_000, gain)?;
                Ok((
                    Box::new(FakeSink {
                        consumer: Mutex::new(consumer),
                        format,
                        out: sink_out.clone(),
                    }),
                    writer,
                    clock,
                ))
            },
        )
        .expect("play");
        let status = shared.status.lock().expect("status").clone();
        let samples = out.lock().expect("out").clone();
        (samples, status)
    }

    #[test]
    fn consecutive_items_play_back_to_back_on_one_sink_without_a_gap() {
        let (a, b) = (tone(6_000, 8_192), tone(4_000, 16_384));
        let (samples, status) = play_two(a.path(), Some(b.path()));
        let live: Vec<f32> = samples.iter().copied().skip_while(|s| *s == 0.0).collect();
        let live = &live[..live.iter().rposition(|s| *s != 0.0).expect("audio") + 1];
        assert_eq!(live.len(), 10_000, "exactly both items, no silence between");
        assert!(live.iter().all(|s| *s != 0.0));
        assert_eq!(live.iter().filter(|s| **s < 0.3).count(), 6_000);
        assert_eq!(status.token, 7);
        assert_eq!(status.state, PlaybackState::Finished);
        assert_eq!(
            status.info.expect("info").duration,
            Some(4_000.0 / 48_000.0)
        );
    }

    #[test]
    fn an_unplayable_next_item_lets_the_current_one_finish_normally() {
        let a = tone(2_000, 8_192);
        let junk = tempfile::Builder::new()
            .suffix(".mp3")
            .tempfile()
            .expect("junk");
        let (_, status) = play_two(a.path(), Some(junk.path()));
        assert_eq!(status.state, PlaybackState::Finished);
        assert_eq!(status.token, 0);
    }
}
