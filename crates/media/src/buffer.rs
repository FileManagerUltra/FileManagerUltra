use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use fm_vfs::FmError;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::eq::EqProcessor;
use crate::pipeline::limit;
use crate::{AudioFormat, Gain};

/// Sample-driven clock: underrun silence and paused callbacks never advance media time.
#[derive(Clone, Debug)]
pub struct Clock {
    state: Arc<ClockState>,
    rate: u32,
}

#[derive(Debug, Default)]
struct ClockState {
    frames: AtomicU64,
    latency: AtomicU64,
    paused: AtomicBool,
}

impl Clock {
    pub fn new(rate: u32) -> Result<Self, FmError> {
        AudioFormat::new(rate, 1)?;
        Ok(Self {
            state: Arc::default(),
            rate,
        })
    }

    pub fn position(&self) -> f64 {
        let frames = self.state.frames.load(Ordering::Acquire);
        let latency = self.state.latency.load(Ordering::Acquire);
        frames.saturating_sub(latency) as f64 / f64::from(self.rate)
    }

    pub fn set_paused(&self, paused: bool) {
        self.state.paused.store(paused, Ordering::Release);
    }

    pub fn paused(&self) -> bool {
        self.state.paused.load(Ordering::Acquire)
    }

    pub(crate) fn set_latency(&self, duration: std::time::Duration) {
        self.state.latency.store(
            (duration.as_secs_f64().min(2.0) * f64::from(self.rate)) as u64,
            Ordering::Release,
        );
    }

    /// Delay between callback submission and audible output, capped at two seconds.
    pub fn latency(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(
            self.state.latency.load(Ordering::Acquire) as f64 / f64::from(self.rate),
        )
    }
}

pub struct AudioProducer {
    ring: Producer<f32>,
    channels: usize,
}

pub struct AudioConsumer {
    ring: Consumer<f32>,
    channels: usize,
    clock: Clock,
    gain: Gain,
    eq: EqProcessor,
}

/// Create a lock-free, single-producer/single-consumer PCM buffer (at most two seconds).
pub fn audio_buffer(
    format: AudioFormat,
    capacity_frames: usize,
    gain: Gain,
) -> Result<(AudioProducer, AudioConsumer, Clock), FmError> {
    if capacity_frames == 0 || capacity_frames > format.rate() as usize * 2 {
        return Err(crate::error(
            "Audio buffer must hold between one frame and two seconds",
        ));
    }
    let (writer, reader) = RingBuffer::new(capacity_frames * format.channels());
    let clock = Clock::new(format.rate())?;
    Ok((
        AudioProducer {
            ring: writer,
            channels: format.channels(),
        },
        AudioConsumer {
            ring: reader,
            channels: format.channels(),
            clock: clock.clone(),
            eq: EqProcessor::new(gain.eq_handle(), format.rate()),
            gain,
        },
        clock,
    ))
}

impl AudioProducer {
    /// Write as many complete frames as fit without waiting; returns samples written.
    pub fn write(&mut self, pcm: &[f32]) -> Result<usize, FmError> {
        if !pcm.len().is_multiple_of(self.channels) || pcm.iter().any(|s| !s.is_finite()) {
            return Err(crate::error("Invalid PCM samples"));
        }
        let len = pcm
            .len()
            .min(self.ring.slots() / self.channels * self.channels);
        for &sample in pcm.iter().take(len) {
            self.ring
                .push(sample)
                .map_err(|_| crate::error("Audio buffer capacity changed"))?;
        }
        Ok(len)
    }

    pub fn queued_frames(&self) -> usize {
        (self.ring.buffer().capacity() - self.ring.slots()) / self.channels
    }
}

impl AudioConsumer {
    /// Fill a device buffer without locks or allocation, padding underruns with silence.
    /// Non-frame-aligned device tails are silent and never consume a partial PCM frame.
    pub fn render<T: cpal::Sample + cpal::FromSample<f32>>(&mut self, output: &mut [T]) {
        let zero = T::from_sample(0.0);
        if self.clock.paused() {
            output.fill(zero);
            return;
        }
        let level = self.gain.level();
        self.eq.refresh();
        let mut frames = 0;
        let mut chunks = output.chunks_exact_mut(self.channels);
        for frame in &mut chunks {
            if self.ring.slots() < self.channels {
                frame.fill(zero);
                continue;
            }
            for (channel, dst) in frame.iter_mut().enumerate() {
                // slots() above guarantees the entire frame; only this consumer can pop.
                match self.ring.pop() {
                    Ok(sample) => {
                        *dst = T::from_sample(limit(self.eq.process(channel, sample) * level));
                    }
                    Err(_) => *dst = zero,
                }
            }
            frames += 1;
        }
        chunks.into_remainder().fill(zero);
        self.clock.state.frames.fetch_add(frames, Ordering::Release);
    }
}
