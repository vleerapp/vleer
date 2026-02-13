use std::sync::{Arc, RwLock};

use crate::data::config::EqualizerSettings;

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
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
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
                let q = config.q_values.get(i).copied().unwrap_or(1.461);
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

                let mut coeffs_guard = self.coeffs.write().unwrap();
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
        let mut coeffs_guard = self.coeffs.write().unwrap();
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
        let mut coeffs_guard = self.coeffs.write().unwrap();
        coeffs_guard[band] = Coeffs::peaking(
            self.bands[band].fc,
            q,
            self.bands[band].gain_db,
            self.sample_rate as f32,
        );
    }

    pub fn get_bands(&self) -> &[Band] {
        &self.bands
    }

    pub fn get_coeffs(&self) -> Arc<RwLock<Vec<Coeffs>>> {
        self.coeffs.clone()
    }
}

impl Coeffs {
    fn peaking(fc: f32, q: f32, gain_db: f32, fs: f32) -> Self {
        if gain_db == 0.0 {
            Self {
                b0: 1.0,
                b1: 0.0,
                b2: 0.0,
                a1: 0.0,
                a2: 0.0,
            }
        } else {
            let a = 10.0f32.powf(gain_db / 20.0);
            let omega = 2.0 * std::f32::consts::PI * fc / fs;
            let sn = omega.sin();
            let cs = omega.cos();
            let alpha = sn / (2.0 * q);
            let b0 = 1.0 + alpha * a;
            let b1 = -2.0 * cs;
            let b2 = 1.0 - alpha * a;
            let a0 = 1.0 + alpha / a;
            let a1 = -2.0 * cs;
            let a2 = 1.0 - alpha / a;
            let inv_a0 = 1.0 / a0;
            Self {
                b0: b0 * inv_a0,
                b1: b1 * inv_a0,
                b2: b2 * inv_a0,
                a1: a1 * inv_a0,
                a2: a2 * inv_a0,
            }
        }
    }
}

pub struct EqualizerSource<S> {
    inner: S,
    coeffs: Arc<RwLock<Vec<Coeffs>>>,
    states: Vec<Vec<(f32, f32)>>,
    current_channel: usize,
}

impl<S: rodio::Source<Item = f32>> EqualizerSource<S> {
    pub fn new(inner: S, equalizer: Arc<std::sync::Mutex<Equalizer>>) -> Self {
        let eq = equalizer.lock().unwrap();
        let channels = inner.channels().get() as usize;
        let states: Vec<Vec<(f32, f32)>> = (0..channels)
            .map(|_| (0..10).map(|_| (0.0, 0.0)).collect())
            .collect();
        Self {
            inner,
            coeffs: eq.get_coeffs(),
            states,
            current_channel: 0,
        }
    }
}

impl<S: rodio::Source<Item = f32>> Iterator for EqualizerSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let input = self.inner.next()?;
        let ch = self.current_channel;
        let coeffs_guard = self.coeffs.read().unwrap();
        let mut s = input;
        let state = &mut self.states[ch];
        for (i, (z1, z2)) in state.iter_mut().enumerate() {
            let c = coeffs_guard[i];
            let out = c.b0 * s + *z1;
            *z1 = c.b1 * s - c.a1 * out + *z2;
            *z2 = c.b2 * s - c.a2 * out;
            s = out;
        }
        self.current_channel = (self.current_channel + 1) % self.states.len();
        Some(s)
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
