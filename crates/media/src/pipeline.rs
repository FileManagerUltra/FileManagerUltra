use fm_vfs::FmError;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const CHUNK_FRAMES: usize = 1024;
/// Largest PCM block accepted from a decoder or caller.
pub const MAX_BLOCK_FRAMES: usize = 65_536;

/// Interleaved PCM format. Limits bound allocations before a decoder reaches the sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    rate: u32,
    channels: usize,
}

impl AudioFormat {
    pub fn new(rate: u32, channels: usize) -> Result<Self, FmError> {
        if !(8_000..=384_000).contains(&rate) || !(1..=8).contains(&channels) {
            return Err(crate::error(
                "Unsupported audio sample rate or channel count",
            ));
        }
        Ok(Self { rate, channels })
    }

    pub fn rate(self) -> u32 {
        self.rate
    }

    pub fn channels(self) -> usize {
        self.channels
    }
}

/// Gain controls shared with the output callback without a lock.
#[derive(Clone, Debug)]
pub struct Gain(Arc<GainState>);

#[derive(Debug)]
struct GainState {
    volume: AtomicU32,
    preamp: AtomicU32,
    muted: AtomicBool,
    fade: AtomicU32,
    eq: crate::eq::EqHandle,
}

impl Default for Gain {
    fn default() -> Self {
        Self(Arc::new(GainState {
            volume: AtomicU32::new(1.0_f32.to_bits()),
            preamp: AtomicU32::new(1.0_f32.to_bits()),
            muted: AtomicBool::new(false),
            fade: AtomicU32::new(1.0_f32.to_bits()),
            eq: crate::eq::EqHandle::default(),
        }))
    }
}

impl Gain {
    pub fn set_volume(&self, percent: f32) -> Result<(), FmError> {
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err(FmError::InvalidParams("volume must be 0–100".into()));
        }
        self.0
            .volume
            .store((percent / 100.0).to_bits(), Ordering::Relaxed);
        Ok(())
    }

    pub fn set_preamp(&self, db: f32) -> Result<(), FmError> {
        if !db.is_finite() || !(-24.0..=12.0).contains(&db) {
            return Err(FmError::InvalidParams("preamp must be -24–12 dB".into()));
        }
        self.0
            .preamp
            .store(10.0_f32.powf(db / 20.0).to_bits(), Ordering::Relaxed);
        Ok(())
    }

    pub fn set_muted(&self, muted: bool) {
        self.0.muted.store(muted, Ordering::Relaxed);
    }

    /// Linear multiplier (0–1) for fade-outs, independent of the user's volume.
    pub fn set_fade(&self, factor: f32) {
        let factor = if factor.is_finite() {
            factor.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.0.fade.store(factor.to_bits(), Ordering::Relaxed);
    }

    pub fn fade(&self) -> f32 {
        f32::from_bits(self.0.fade.load(Ordering::Relaxed))
    }

    /// Applies the bands and the on/off switch; live and without interrupting playback.
    pub fn set_eq(&self, settings: &crate::EqSettings) -> Result<(), FmError> {
        self.0.eq.set(settings)
    }

    pub fn eq(&self) -> crate::EqSettings {
        self.0.eq.get()
    }

    pub(crate) fn eq_handle(&self) -> crate::eq::EqHandle {
        self.0.eq.clone()
    }

    pub(crate) fn level(&self) -> f32 {
        if self.0.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.0.volume.load(Ordering::Relaxed))
                * f32::from_bits(self.0.preamp.load(Ordering::Relaxed))
                * f32::from_bits(self.0.fade.load(Ordering::Relaxed))
        }
    }
}

// Transparent at normal levels; a smooth knee prevents overshoots clipping the device.
pub(crate) fn limit(sample: f32) -> f32 {
    let a = sample.abs();
    if a <= 0.9 {
        sample
    } else {
        sample.signum() * (0.9 + 0.1 * ((a - 0.9) * 10.0).tanh())
    }
}

/// One rate conversion and channel mapping, retaining resampler state across packets.
/// Call `finish` once at EOF to drain the filter without inserting its startup delay.
pub struct Pipeline {
    input: AudioFormat,
    output: AudioFormat,
    resampler: Option<SincFixedIn<f32>>,
    pending: Vec<Vec<f32>>,
    skip: usize,
    input_frames: u64,
    output_frames: u64,
    finished: bool,
}

impl Pipeline {
    pub fn new(input: AudioFormat, output: AudioFormat) -> Result<Self, FmError> {
        if input.channels > 2 && input.channels != output.channels {
            return Err(crate::error(
                "Multichannel downmix requires a speaker layout; choose a matching output device",
            ));
        }
        let resampler = if input.rate == output.rate {
            None
        } else {
            Some(
                SincFixedIn::new(
                    f64::from(output.rate) / f64::from(input.rate),
                    1.0,
                    SincInterpolationParameters {
                        sinc_len: 256,
                        f_cutoff: 0.95,
                        interpolation: SincInterpolationType::Cubic,
                        oversampling_factor: 128,
                        window: WindowFunction::BlackmanHarris2,
                    },
                    CHUNK_FRAMES,
                    output.channels,
                )
                .map_err(|e| crate::error(format!("Cannot initialize audio resampling: {e}")))?,
            )
        };
        let skip = resampler.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            input,
            output,
            resampler,
            pending: vec![Vec::with_capacity(CHUNK_FRAMES); output.channels],
            skip,
            input_frames: 0,
            output_frames: 0,
            finished: false,
        })
    }

    pub fn process(&mut self, pcm: &[f32]) -> Result<Vec<f32>, FmError> {
        if self.finished
            || !pcm.len().is_multiple_of(self.input.channels)
            || pcm.len() / self.input.channels > MAX_BLOCK_FRAMES
            || pcm.iter().any(|s| !s.is_finite())
        {
            return Err(crate::error("Invalid or oversized decoded audio block"));
        }
        self.input_frames += (pcm.len() / self.input.channels) as u64;
        let mut out = Vec::new();
        for frame in pcm.chunks_exact(self.input.channels) {
            for (ch, pending) in self.pending.iter_mut().enumerate() {
                let sample = if self.input.channels == self.output.channels {
                    frame
                        .get(ch)
                        .copied()
                        .ok_or_else(|| crate::error("Missing channel"))?
                } else if self.output.channels == 1 {
                    frame.iter().sum::<f32>() / self.input.channels as f32
                } else if ch < 2 {
                    *frame
                        .get(ch % self.input.channels)
                        .ok_or_else(|| crate::error("Missing channel"))?
                } else {
                    0.0
                };
                pending.push(sample);
            }
            if self
                .pending
                .first()
                .is_some_and(|p| p.len() == CHUNK_FRAMES)
            {
                self.flush_chunk(&mut out)?;
            }
        }
        Ok(out)
    }

    pub fn finish(&mut self) -> Result<Vec<f32>, FmError> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        let target = self.input_frames * u64::from(self.output.rate) / u64::from(self.input.rate);
        let mut out = Vec::new();
        while self.output_frames < target {
            for ch in &mut self.pending {
                ch.resize(CHUNK_FRAMES, 0.0);
            }
            self.flush_chunk(&mut out)?;
        }
        let excess = (self.output_frames - target) as usize * self.output.channels;
        out.truncate(out.len().saturating_sub(excess));
        self.output_frames = target;
        Ok(out)
    }

    fn flush_chunk(&mut self, out: &mut Vec<f32>) -> Result<(), FmError> {
        let result = if let Some(resampler) = &mut self.resampler {
            resampler
                .process(&self.pending, None)
                .map_err(|e| crate::error(format!("Audio resampling failed: {e}")))?
        } else {
            self.pending.clone()
        };
        for ch in &mut self.pending {
            ch.clear();
        }
        let frames = result.first().map_or(0, Vec::len);
        let start = self.skip.min(frames);
        self.skip -= start;
        for frame in start..frames {
            for ch in &result {
                out.push(
                    *ch.get(frame)
                        .ok_or_else(|| crate::error("Incomplete resampled frame"))?,
                );
            }
        }
        self.output_frames += (frames - start) as u64;
        Ok(())
    }
}
