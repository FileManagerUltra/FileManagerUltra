//! Ten-band graphic equalizer. Settings are shared with the output callback through
//! atomics; the callback owns the filter state, recomputes coefficients without allocating
//! when the generation changes, and keeps the state so live changes do not click.

use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use fm_vfs::FmError;

pub const BANDS: usize = 10;
pub const BAND_HZ: [f32; BANDS] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];
pub const MAX_BAND_DB: f32 = 12.0;
const MAX_CHANNELS: usize = 8;
/// One octave between neighbours.
const Q: f32 = 1.414;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqSettings {
    pub enabled: bool,
    pub bands: [f32; BANDS],
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bands: [0.0; BANDS],
        }
    }
}

impl EqSettings {
    pub fn validate(&self) -> Result<(), FmError> {
        if self
            .bands
            .iter()
            .any(|g| !g.is_finite() || g.abs() > MAX_BAND_DB)
        {
            return Err(FmError::InvalidParams(
                "equalizer bands must be within ±12 dB".into(),
            ));
        }
        Ok(())
    }

    /// Preamp that cancels the largest boost so raised bands do not clip.
    pub fn safe_preamp(&self) -> f32 {
        -self.bands.iter().copied().fold(0.0_f32, f32::max)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EqHandle(Arc<EqShared>);

#[derive(Debug)]
struct EqShared {
    generation: AtomicU32,
    enabled: AtomicBool,
    bands: [AtomicU32; BANDS],
}

impl Default for EqHandle {
    fn default() -> Self {
        Self(Arc::new(EqShared {
            generation: AtomicU32::new(0),
            enabled: AtomicBool::new(false),
            bands: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
        }))
    }
}

impl EqHandle {
    pub(crate) fn set(&self, settings: &EqSettings) -> Result<(), FmError> {
        settings.validate()?;
        for (slot, gain) in self.0.bands.iter().zip(settings.bands) {
            slot.store(gain.to_bits(), Ordering::Relaxed);
        }
        self.0.enabled.store(settings.enabled, Ordering::Relaxed);
        self.0.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub(crate) fn get(&self) -> EqSettings {
        EqSettings {
            enabled: self.0.enabled.load(Ordering::Relaxed),
            bands: std::array::from_fn(|i| {
                self.0
                    .bands
                    .get(i)
                    .map_or(0.0, |b| f32::from_bits(b.load(Ordering::Relaxed)))
            }),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// Real-time side: no locks, no allocation.
pub(crate) struct EqProcessor {
    handle: EqHandle,
    rate: f32,
    generation: u32,
    active: bool,
    coefs: [Option<Coef>; BANDS],
    state: [[[f32; 2]; BANDS]; MAX_CHANNELS],
}

impl EqProcessor {
    pub(crate) fn new(handle: EqHandle, rate: u32) -> Self {
        let mut p = Self {
            handle,
            rate: rate as f32,
            generation: u32::MAX,
            active: false,
            coefs: [None; BANDS],
            state: [[[0.0; 2]; BANDS]; MAX_CHANNELS],
        };
        p.refresh();
        p
    }

    /// Call once per callback; recomputes coefficients only when settings changed.
    pub(crate) fn refresh(&mut self) {
        let generation = self.handle.0.generation.load(Ordering::Acquire);
        if generation == self.generation {
            return;
        }
        self.generation = generation;
        let settings = self.handle.get();
        self.active = settings.enabled && settings.bands.iter().any(|g| *g != 0.0);
        for ((coef, gain), hz) in self.coefs.iter_mut().zip(settings.bands).zip(BAND_HZ) {
            *coef = (gain != 0.0 && hz < self.rate * 0.45).then(|| peaking(self.rate, hz, gain));
        }
        if !self.active {
            self.state = [[[0.0; 2]; BANDS]; MAX_CHANNELS];
        }
    }

    #[inline]
    pub(crate) fn process(&mut self, channel: usize, mut x: f32) -> f32 {
        if !self.active {
            return x;
        }
        let Some(state) = self.state.get_mut(channel) else {
            return x;
        };
        for (coef, z) in self.coefs.iter().zip(state.iter_mut()) {
            if let Some(c) = coef {
                let y = c.b0 * x + z[0];
                z[0] = c.b1 * x - c.a1 * y + z[1];
                z[1] = c.b2 * x - c.a2 * y;
                x = y;
            }
        }
        x
    }
}

// RBJ peaking filter, normalised by a0.
fn peaking(rate: f32, hz: f32, db: f32) -> Coef {
    let a = 10.0_f32.powf(db / 40.0);
    let w = 2.0 * PI * hz / rate;
    let alpha = w.sin() / (2.0 * Q);
    let a0 = 1.0 + alpha / a;
    Coef {
        b0: (1.0 + alpha * a) / a0,
        b1: -2.0 * w.cos() / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: -2.0 * w.cos() / a0,
        a2: (1.0 - alpha / a) / a0,
    }
}

/// Built-in presets, in dB per band.
pub const PRESETS: &[(&str, [f32; BANDS])] = &[
    ("Flat", [0.0; BANDS]),
    (
        "Bass boost",
        [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    (
        "Treble boost",
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 6.0],
    ),
    (
        "Vocal",
        [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0],
    ),
    (
        "Loudness",
        [6.0, 4.0, 1.0, 0.0, -1.0, 0.0, 1.0, 3.0, 5.0, 4.0],
    ),
    (
        "Rock",
        [4.0, 3.0, 2.0, -1.0, -2.0, -1.0, 2.0, 3.0, 4.0, 4.0],
    ),
    (
        "Classical",
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -3.0, -3.0, -3.0, -4.0],
    ),
    (
        "Electronic",
        [5.0, 4.0, 1.0, 0.0, -2.0, 2.0, 1.0, 3.0, 5.0, 5.0],
    ),
];

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    fn run(handle: &EqHandle, hz: f32) -> f32 {
        let rate = 48_000;
        let mut eq = EqProcessor::new(handle.clone(), rate);
        let n = rate as usize;
        let mut energy_in = 0.0_f64;
        let mut energy_out = 0.0_f64;
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f32 / rate as f32).sin() * 0.25;
            let y = eq.process(0, x);
            if i > n / 2 {
                energy_in += f64::from(x * x);
                energy_out += f64::from(y * y);
            }
        }
        (10.0 * (energy_out / energy_in).log10()) as f32
    }

    fn handle(band: usize, db: f32, enabled: bool) -> EqHandle {
        let h = EqHandle::default();
        let mut s = EqSettings {
            enabled,
            ..EqSettings::default()
        };
        s.bands[band] = db;
        h.set(&s).expect("set");
        h
    }

    #[test]
    fn a_boosted_band_raises_its_centre_and_leaves_distant_frequencies() {
        let h = handle(1, 12.0, true);
        assert!((run(&h, 62.0) - 12.0).abs() < 0.5);
        assert!(run(&h, 8_000.0).abs() < 0.5);
    }

    #[test]
    fn a_cut_lowers_the_band() {
        let h = handle(5, -12.0, true);
        assert!((run(&h, 1_000.0) + 12.0).abs() < 0.5);
    }

    #[test]
    fn disabled_or_flat_is_transparent() {
        assert!(run(&handle(1, 12.0, false), 62.0).abs() < 0.001);
        assert!(run(&handle(1, 0.0, true), 62.0).abs() < 0.001);
    }

    #[test]
    fn out_of_range_or_non_finite_settings_are_rejected_and_keep_the_old_ones() {
        let h = handle(2, 3.0, true);
        for bad in [13.0, -12.5, f32::NAN, f32::INFINITY] {
            let mut s = EqSettings::default();
            s.bands[0] = bad;
            assert!(h.set(&s).is_err());
        }
        assert_eq!(h.get().bands[2], 3.0);
    }

    #[test]
    fn live_changes_stay_finite_and_continuous() {
        let h = EqHandle::default();
        let mut eq = EqProcessor::new(h.clone(), 44_100);
        let mut last = 0.0_f32;
        for i in 0..44_100 {
            if i % 4_410 == 0 {
                let mut s = EqSettings {
                    enabled: true,
                    ..EqSettings::default()
                };
                s.bands[1] = (i / 4_410) as f32 - 5.0;
                h.set(&s).expect("set");
                eq.refresh();
            }
            let x = (2.0 * PI * 62.0 * i as f32 / 44_100.0).sin() * 0.25;
            let y = eq.process(0, x);
            assert!(y.is_finite() && y.abs() < 2.0);
            assert!((y - last).abs() < 0.2, "click at {i}");
            last = y;
        }
    }

    #[test]
    fn presets_are_valid_and_a_boost_gets_a_safe_preamp() {
        for (name, bands) in PRESETS {
            let s = EqSettings {
                enabled: true,
                bands: *bands,
            };
            assert!(s.validate().is_ok(), "{name}");
            assert!(s.safe_preamp() <= 0.0);
        }
        assert_eq!(PRESETS[0].0, "Flat");
    }

    #[test]
    fn bands_above_nyquist_are_skipped() {
        let h = handle(9, 12.0, true);
        let mut eq = EqProcessor::new(h, 8_000);
        assert!(eq.process(0, 0.5).is_finite());
        assert!(eq.coefs[9].is_none());
    }
}
