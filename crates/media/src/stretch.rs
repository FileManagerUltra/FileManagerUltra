//! Pitch-preserving time stretch (WSOLA) for audio played faster or slower than 1×.
//!
//! The input is interleaved f32 PCM at the device rate. Output frames are 30 ms Hann
//! windows overlapped by half (the windows then sum to exactly 1). Frame `k` is read from
//! the input near `k × hop × speed`, shifted by up to ±12 ms to the spot that best matches
//! the natural continuation of the previous frame (normalised cross-correlation on a mono
//! mix), so waveforms line up and the pitch stays the same. The search is coarse to fine,
//! which keeps it cheap even in unoptimised builds.
//!
//! Results depend only on absolute input positions, never on how the input was chunked.
//! Memory is bounded: input older than the next frame's search window is dropped.

use fm_vfs::FmError;

/// Output frame length.
const FRAME_SECONDS: f64 = 0.030;
/// Largest shift searched either side of the ideal read position.
const TOLERANCE_SECONDS: f64 = 0.012;
/// Coarse search step, and the decimation of the correlation in that pass.
const COARSE: usize = 4;

pub const MIN_SPEED: f64 = 0.5;
pub const MAX_SPEED: f64 = 3.0;

#[derive(Debug)]
pub struct Stretcher {
    channels: usize,
    speed: f64,
    /// Frame length and output hop, in frames.
    frame: usize,
    hop: usize,
    tolerance: usize,
    window: Vec<f32>,
    /// Interleaved input not yet consumed; `input[0]` is absolute frame `base`.
    input: Vec<f32>,
    /// Mono mix of `input`, one value per frame, for the similarity search.
    mix: Vec<f32>,
    base: usize,
    /// Output frames produced so far.
    emitted: usize,
    /// Absolute input frames seen so far.
    seen: usize,
    /// Ideal absolute read position of the next frame.
    next: f64,
    /// Absolute start of the frame read last.
    prev: Option<usize>,
    /// Overlap-add accumulator, `frame` interleaved frames.
    acc: Vec<f32>,
}

impl Stretcher {
    pub fn new(rate: u32, channels: usize, speed: f64) -> Result<Self, FmError> {
        if !speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(&speed) {
            return Err(FmError::InvalidParams(format!(
                "speed must be between {MIN_SPEED} and {MAX_SPEED}"
            )));
        }
        if channels == 0 || rate == 0 {
            return Err(FmError::InvalidParams("invalid audio format".into()));
        }
        let half = ((f64::from(rate) * FRAME_SECONDS / 2.0).round() as usize).max(8);
        let frame = half * 2;
        let window = (0..frame)
            .map(|n| {
                let phase = std::f64::consts::TAU * n as f64 / frame as f64;
                (0.5 - 0.5 * phase.cos()) as f32
            })
            .collect();
        Ok(Self {
            channels,
            speed,
            frame,
            hop: half,
            tolerance: ((f64::from(rate) * TOLERANCE_SECONDS).round() as usize).max(1),
            window,
            input: Vec::new(),
            mix: Vec::new(),
            base: 0,
            emitted: 0,
            seen: 0,
            next: 0.0,
            prev: None,
            acc: vec![0.0; frame * channels],
        })
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Stretch `pcm` (interleaved); returns the output that is ready.
    pub fn process(&mut self, pcm: &[f32]) -> Vec<f32> {
        let whole = pcm.len() / self.channels * self.channels;
        let pcm = pcm.get(..whole).unwrap_or_default();
        self.input.extend_from_slice(pcm);
        self.mix
            .extend(pcm.chunks(self.channels).map(|f| f.iter().sum::<f32>()));
        self.seen += whole / self.channels;
        let mut out = Vec::new();
        while self.step(&mut out, self.seen) {}
        out
    }

    /// The input has ended: return the rest, so the output lasts the input's length / speed.
    pub fn finish(&mut self) -> Vec<f32> {
        let end = self.seen;
        let pad = self.frame + self.tolerance + self.hop;
        self.input
            .resize(self.input.len() + pad * self.channels, 0.0);
        self.mix.resize(self.mix.len() + pad, 0.0);
        let available = self.seen + pad;
        let mut out = Vec::new();
        while (self.next as usize) < end && self.step(&mut out, available) {}
        // The second half of the last frame is still in the accumulator.
        out.extend_from_slice(self.acc.get(..self.hop * self.channels).unwrap_or_default());
        self.emitted += self.hop;
        // Hops come in whole frames; cut the tail so the total lasts input / speed.
        let wanted = (end as f64 / self.speed).round() as usize;
        let extra = self
            .emitted
            .saturating_sub(wanted)
            .min(out.len() / self.channels);
        out.truncate(out.len() - extra * self.channels);
        self.emitted -= extra;
        self.acc.iter_mut().for_each(|s| *s = 0.0);
        self.input.clear();
        self.mix.clear();
        self.base = self.seen;
        out
    }

    /// Normalised correlation of `len` frames at `candidate` with those at `target`
    /// (absolute positions), every `step`-th frame.
    fn similarity(&self, candidate: usize, target: usize, len: usize, step: usize) -> f32 {
        let slice = |at: usize| {
            at.checked_sub(self.base)
                .and_then(|from| self.mix.get(from..from + len))
                .unwrap_or_default()
        };
        let (c, t) = (slice(candidate), slice(target));
        let (mut dot, mut energy) = (0.0_f32, 0.0_f32);
        for (a, b) in c.iter().step_by(step).zip(t.iter().step_by(step)) {
            dot += a * b;
            energy += a * a;
        }
        if energy <= f32::EPSILON {
            0.0
        } else {
            dot / energy.sqrt()
        }
    }

    /// Emit one output hop if enough input is buffered (`available` = absolute end).
    fn step(&mut self, out: &mut Vec<f32>, available: usize) -> bool {
        let ideal = self.next.floor() as usize;
        let low = ideal.saturating_sub(self.tolerance).max(self.base);
        let high = ideal + self.tolerance;
        let target = self.prev.map(|p| p + self.hop);
        let needed = (high + self.frame).max(target.map_or(0, |t| t + self.hop));
        if needed > available {
            return false;
        }
        let start = match target {
            None => ideal.max(self.base),
            Some(target) => {
                let mut best = (f32::MIN, ideal.max(low).min(high));
                let mut c = low;
                while c <= high {
                    let score = self.similarity(c, target, self.hop, COARSE);
                    if score > best.0 {
                        best = (score, c);
                    }
                    c += COARSE;
                }
                let (from, to) = (
                    best.1.saturating_sub(COARSE).max(low),
                    (best.1 + COARSE).min(high),
                );
                let mut fine = (f32::MIN, best.1);
                for c in from..=to {
                    let score = self.similarity(c, target, self.hop, 1);
                    if score > fine.0 {
                        fine = (score, c);
                    }
                }
                fine.1
            }
        };
        let ch = self.channels;
        let offset = start.saturating_sub(self.base) * ch;
        for (n, w) in self.window.iter().enumerate() {
            for c in 0..ch {
                if let (Some(acc), Some(x)) = (
                    self.acc.get_mut(n * ch + c),
                    self.input.get(offset + n * ch + c),
                ) {
                    *acc += w * x;
                }
            }
        }
        let emitted = self.hop * ch;
        out.extend_from_slice(self.acc.get(..emitted).unwrap_or_default());
        self.emitted += self.hop;
        self.acc.copy_within(emitted.., 0);
        let len = self.acc.len();
        self.acc
            .get_mut(len - emitted..)
            .unwrap_or_default()
            .iter_mut()
            .for_each(|s| *s = 0.0);
        self.prev = Some(start);
        self.next += self.hop as f64 * self.speed;
        // Keep what the next frame's search and continuation can still reach.
        let keep_from = (self.next.floor() as usize)
            .saturating_sub(self.tolerance)
            .min(start + self.hop)
            .max(self.base);
        let drop = keep_from.saturating_sub(self.base) * ch;
        if drop > 0 {
            self.input.drain(..drop.min(self.input.len()));
            let frames = keep_from.saturating_sub(self.base);
            self.mix.drain(..frames.min(self.mix.len()));
            self.base = keep_from;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(hz: f64, seconds: f64, channels: usize) -> Vec<f32> {
        let frames = (f64::from(RATE) * seconds) as usize;
        (0..frames)
            .flat_map(|n| {
                let v =
                    (std::f64::consts::TAU * hz * n as f64 / f64::from(RATE)).sin() as f32 * 0.5;
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn stretch(input: &[f32], channels: usize, speed: f64, chunk: usize) -> Vec<f32> {
        let mut s = Stretcher::new(RATE, channels, speed).expect("stretcher");
        let mut out = Vec::new();
        for part in input.chunks(chunk * channels) {
            out.extend(s.process(part));
        }
        out.extend(s.finish());
        out
    }

    /// Frequency from upward zero crossings in the steady middle of a mono signal.
    fn frequency(mono: &[f32]) -> f64 {
        let middle = &mono[mono.len() / 4..mono.len() * 3 / 4];
        let crossings = middle
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f64 * f64::from(RATE) / middle.len() as f64
    }

    #[test]
    fn speed_changes_duration_but_not_pitch() {
        let input = sine(440.0, 2.0, 1);
        for speed in [0.5, 0.75, 1.5, 2.0, 3.0] {
            let out = stretch(&input, 1, speed, 4_096);
            let expected = input.len() as f64 / speed;
            let ratio = out.len() as f64 / expected;
            assert!(
                (0.97..=1.03).contains(&ratio),
                "{speed}×: {} frames, expected about {expected}",
                out.len()
            );
            let hz = frequency(&out);
            assert!((hz - 440.0).abs() < 8.0, "{speed}× pitch moved to {hz} Hz");
        }
    }

    #[test]
    fn a_steady_tone_keeps_its_level() {
        let input = sine(300.0, 1.0, 1);
        let out = stretch(&input, 1, 1.5, 1_000);
        let middle = &out[out.len() / 4..out.len() * 3 / 4];
        let peak = middle.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((0.45..=0.55).contains(&peak), "peak {peak}");
    }

    #[test]
    fn chunk_size_does_not_change_the_result() {
        let input = sine(523.0, 0.5, 2);
        let whole = stretch(&input, 2, 1.25, input.len());
        for chunk in [1, 97, 1_024] {
            assert_eq!(stretch(&input, 2, 1.25, chunk), whole, "chunk {chunk}");
        }
    }

    #[test]
    fn channels_stay_separate_and_silence_stays_silent() {
        // Left carries a tone, right is silent.
        let left = sine(440.0, 0.5, 1);
        let input: Vec<f32> = left.iter().flat_map(|l| [*l, 0.0]).collect();
        let out = stretch(&input, 2, 2.0, 2_048);
        assert!(out.chunks(2).all(|f| f[1] == 0.0), "right stays silent");
        assert!(
            out.chunks(2).any(|f| f[0].abs() > 0.1),
            "left carries the tone"
        );
        let silent = stretch(&vec![0.0; 9_600], 1, 0.5, 512);
        assert!(silent.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn buffered_input_stays_bounded() {
        let mut s = Stretcher::new(RATE, 2, 0.5).expect("stretcher");
        let block = sine(200.0, 0.1, 2);
        for _ in 0..200 {
            let _ = s.process(&block);
            assert!(s.input.len() < 3 * (s.frame + 2 * s.tolerance) * 2 + block.len());
        }
    }

    #[test]
    fn invalid_speeds_and_formats_are_refused() {
        for speed in [0.0, 0.49, 3.01, 5.0, f64::NAN, f64::INFINITY] {
            assert!(Stretcher::new(RATE, 2, speed).is_err(), "{speed}");
        }
        assert!(Stretcher::new(RATE, 0, 1.5).is_err());
        assert!(Stretcher::new(0, 2, 1.5).is_err());
        // A ragged buffer is cut to whole frames instead of misaligning channels.
        let mut s = Stretcher::new(RATE, 2, 1.5).expect("stretcher");
        let _ = s.process(&[0.1, 0.2, 0.3]);
        assert_eq!(s.seen, 1);
    }
}
