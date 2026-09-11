use std::sync::Arc;

use parking_lot::RwLock;

use crate::data::config::EqualizerSettings;

pub const Q_DEFAULT: f32 = 1.461;
pub const Q_MIN: f32 = 0.2;
pub const Q_MAX: f32 = 12.0;

const NYQUIST_MARGIN: f64 = 0.45;
const QUIET: f64 = 1e-30;
const FLUSH_INTERVAL: u32 = 512;

pub struct Equalizer {
    sample_rate: u32,
    bands: Vec<Band>,
    coeffs: Arc<RwLock<Vec<Coeffs>>>,
}

pub struct Band {
    pub fc: f32,
    pub q: f32,
    pub gain_db: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Equalizer {
    pub fn new(sample_rate: u32, _channels: u16) -> Self {
        let config = EqualizerSettings::default();
        Self::from_settings(sample_rate, &config)
    }

    pub fn from_settings(sample_rate: u32, config: &EqualizerSettings) -> Self {
        let num_bands = config.frequencies.len();

        let bands: Vec<Band> = (0..num_bands)
            .map(|i| {
                let fc = config.frequencies.get(i).copied().unwrap_or(1000) as f32;
                let q = config.q_values.get(i).copied().unwrap_or(Q_DEFAULT);
                let gain = config.gains.get(i).copied().unwrap_or(0.0);
                let gain_db = if config.enabled { gain } else { 0.0 };

                Band { fc, q, gain_db }
            })
            .collect();

        let coeffs_vec: Vec<Coeffs> = bands
            .iter()
            .map(|band| Coeffs::peaking(band.fc, band.q, band.gain_db, sample_rate as f32))
            .collect();

        let coeffs = Arc::new(RwLock::new(coeffs_vec));

        Self {
            sample_rate,
            bands,
            coeffs,
        }
    }

    pub fn apply_settings(&mut self, config: &EqualizerSettings) {
        let num_bands = config.frequencies.len().min(self.bands.len());

        for i in 0..num_bands {
            if let (Some(&freq), Some(&gain), Some(&q)) = (
                config.frequencies.get(i),
                config.gains.get(i),
                config.q_values.get(i),
            ) {
                self.bands[i].fc = freq as f32;
                self.bands[i].q = q;

                let effective_gain = if config.enabled { gain } else { 0.0 };
                self.bands[i].gain_db = effective_gain;

                let mut coeffs_guard = self.coeffs.write();
                coeffs_guard[i] =
                    Coeffs::peaking(freq as f32, q, effective_gain, self.sample_rate as f32);
            }
        }
    }

    pub fn set_gain(&mut self, band: usize, gain_db: f32) {
        if band >= self.bands.len() {
            return;
        }
        self.bands[band].gain_db = gain_db;
        let mut coeffs_guard = self.coeffs.write();
        coeffs_guard[band] = Coeffs::peaking(
            self.bands[band].fc,
            self.bands[band].q,
            gain_db,
            self.sample_rate as f32,
        );
    }

    pub fn set_q(&mut self, band: usize, q: f32) {
        if band >= self.bands.len() {
            return;
        }
        self.bands[band].q = q;
        let mut coeffs_guard = self.coeffs.write();
        coeffs_guard[band] = Coeffs::peaking(
            self.bands[band].fc,
            q,
            self.bands[band].gain_db,
            self.sample_rate as f32,
        );
    }

    pub(crate) fn get_coeffs(&self) -> Arc<RwLock<Vec<Coeffs>>> {
        self.coeffs.clone()
    }
}

fn clamp_q(q: f32) -> f32 {
    if q.is_nan() {
        Q_DEFAULT
    } else {
        q.clamp(Q_MIN, Q_MAX)
    }
}

impl Coeffs {
    const PASSTHROUGH: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    fn peaking(fc: f32, q: f32, gain_db: f32, fs: f32) -> Self {
        if gain_db == 0.0 || gain_db.is_nan() || fs <= 0.0 {
            return Self::PASSTHROUGH;
        }
        let fc = fc as f64;
        let fs = fs as f64;
        if !fc.is_finite() || fc <= 0.0 || fc >= fs * NYQUIST_MARGIN {
            return Self::PASSTHROUGH;
        }

        let a = 10.0f64.powf(gain_db as f64 / 40.0);
        let omega = std::f64::consts::TAU * fc / fs;
        let sn = omega.sin();
        let cs = omega.cos();
        let alpha = sn / (2.0 * clamp_q(q) as f64);
        let a0 = 1.0 + alpha / a;
        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cs) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cs) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

pub struct EqualizerSource<S> {
    inner: S,
    coeffs: Arc<RwLock<Vec<Coeffs>>>,
    states: Vec<Vec<(f64, f64)>>,
    current_channel: usize,
    since_flush: u32,
}

impl<S: rodio::Source<Item = f32>> EqualizerSource<S> {
    pub fn new(inner: S, equalizer: Arc<parking_lot::Mutex<Equalizer>>) -> Self {
        let eq = equalizer.lock();
        let channels = inner.channels().get() as usize;
        let num_bands = eq.get_coeffs().read().len();
        let states: Vec<Vec<(f64, f64)>> = (0..channels)
            .map(|_| (0..num_bands).map(|_| (0.0, 0.0)).collect())
            .collect();
        Self {
            inner,
            coeffs: eq.get_coeffs(),
            states,
            current_channel: 0,
            since_flush: 0,
        }
    }
}

impl<S: rodio::Source<Item = f32>> Iterator for EqualizerSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let input = self.inner.next()?;
        let ch = self.current_channel;
        let coeffs_guard = self.coeffs.read();
        let mut s = input as f64;
        let state = &mut self.states[ch];
        let bands = state.len().min(coeffs_guard.len());
        for (i, (z1, z2)) in state.iter_mut().enumerate().take(bands) {
            let c = coeffs_guard[i];
            let out = c.b0 * s + *z1;
            *z1 = c.b1 * s - c.a1 * out + *z2;
            *z2 = c.b2 * s - c.a2 * out;
            s = out;
        }

        self.since_flush += 1;
        if self.since_flush >= FLUSH_INTERVAL {
            self.since_flush = 0;
            for (z1, z2) in state.iter_mut() {
                if z1.abs() < QUIET && z2.abs() < QUIET {
                    *z1 = 0.0;
                    *z2 = 0.0;
                }
            }
        }

        self.current_channel = (self.current_channel + 1) % self.states.len();
        Some(s as f32)
    }
}

impl<S: rodio::Source<Item = f32>> rodio::Source for EqualizerSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> std::num::NonZero<u16> {
        self.inner.channels()
    }

    fn sample_rate(&self) -> std::num::NonZero<u32> {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;

    impl Coeffs {
        fn response_db(&self, hz: f64, fs: f64) -> f64 {
            let w = std::f64::consts::TAU * hz / fs;
            let (sin1, cos1) = w.sin_cos();
            let (sin2, cos2) = (2.0 * w).sin_cos();
            let num_re = self.b0 + self.b1 * cos1 + self.b2 * cos2;
            let num_im = -(self.b1 * sin1 + self.b2 * sin2);
            let den_re = 1.0 + self.a1 * cos1 + self.a2 * cos2;
            let den_im = -(self.a1 * sin1 + self.a2 * sin2);
            let num = num_re * num_re + num_im * num_im;
            let den = den_re * den_re + den_im * den_im;
            10.0 * (num / den).log10()
        }
    }

    #[test]
    fn a_band_hits_its_labelled_gain_at_its_center() {
        for db in [-12.0, -6.0, 3.0, 6.0, 12.0] {
            let c = Coeffs::peaking(1000.0, Q_DEFAULT, db, RATE);
            let at_center = c.response_db(1000.0, RATE as f64);
            assert!(
                (at_center - db as f64).abs() < 0.1,
                "{db} dB band read {at_center} dB at its center"
            );
        }
    }

    #[test]
    fn a_flat_band_is_passthrough() {
        let c = Coeffs::peaking(1000.0, Q_DEFAULT, 0.0, RATE);
        assert_eq!(c.b0, 1.0);
        assert_eq!(c.b1, 0.0);
        assert_eq!(c.b2, 0.0);
        assert_eq!(c.a1, 0.0);
        assert_eq!(c.a2, 0.0);
    }

    #[test]
    fn a_band_past_nyquist_passes_through() {
        let c = Coeffs::peaking(16000.0, Q_DEFAULT, 12.0, 32000.0);
        assert_eq!(c.b0, 1.0);
        assert_eq!(c.b2, 0.0);

        let c = Coeffs::peaking(16000.0, Q_DEFAULT, 12.0, 44100.0);
        assert!((c.response_db(16000.0, 44100.0) - 12.0).abs() < 0.1);
    }

    #[test]
    fn a_higher_q_narrows_the_bell() {
        let wide = Coeffs::peaking(1000.0, 0.5, 12.0, RATE);
        let narrow = Coeffs::peaking(1000.0, 8.0, 12.0, RATE);
        assert!(wide.response_db(1400.0, RATE as f64) > narrow.response_db(1400.0, RATE as f64));
    }

    #[test]
    fn a_degenerate_q_cannot_poison_the_coefficients() {
        for q in [0.0, -4.0, f32::NAN, 1e9] {
            let c = Coeffs::peaking(1000.0, q, 12.0, RATE);
            assert!(c.b0.is_finite() && c.a1.is_finite() && c.a2.is_finite());
        }
    }
}
