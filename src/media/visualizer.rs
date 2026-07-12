use parking_lot::Mutex;
use rodio::Source;
use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};
use std::sync::Arc;

pub trait ToF32 {
    fn to_f32_sample(&self) -> f32;
}

impl ToF32 for f32 {
    fn to_f32_sample(&self) -> f32 {
        *self
    }
}

impl ToF32 for i16 {
    fn to_f32_sample(&self) -> f32 {
        *self as f32 / 32768.0
    }
}

impl ToF32 for u16 {
    fn to_f32_sample(&self) -> f32 {
        (*self as f32 - 32768.0) / 32768.0
    }
}

#[derive(Clone, Default)]
pub struct VisualizerState {
    pub bands: Arc<Mutex<[f32; 4]>>,
}

struct BandDetector {
    value: f32,
    attack: f32,
    release: f32,
}

impl BandDetector {
    fn new(attack: f32, release: f32) -> Self {
        Self {
            value: 0.0,
            attack,
            release,
        }
    }

    fn update(&mut self, energy: f32) -> f32 {
        let target = energy.clamp(0.0, 1.0);
        let alpha = if target > self.value {
            self.attack
        } else {
            self.release
        };
        self.value = self.value * (1.0 - alpha) + target * alpha;
        self.value.clamp(0.0, 1.0)
    }
}

pub struct VisualizerSource<I> {
    input: I,
    buffer: Vec<f32>,
    state: VisualizerState,
    _channels: u16,
    sample_rate: u32,
    bands: [BandDetector; 4],
    peak_ref: [f32; 4],
}

impl<I> VisualizerSource<I>
where
    I: Source<Item = f32>,
{
    pub fn new(input: I, state: VisualizerState) -> Self {
        let channels = input.channels().get();
        let sample_rate = input.sample_rate().get();
        Self {
            input,
            buffer: Vec::with_capacity(1024),
            state,
            _channels: channels,
            sample_rate,
            bands: [
                BandDetector::new(0.45, 0.12),
                BandDetector::new(0.35, 0.10),
                BandDetector::new(0.25, 0.08),
                BandDetector::new(0.40, 0.11),
            ],
            peak_ref: [10.0; 4],
        }
    }

    fn process_spectrum(&mut self) {
        let effective_sample_rate = self.sample_rate * self._channels as u32;
        let spectrum = samples_fft_to_spectrum(
            &self.buffer,
            effective_sample_rate,
            FrequencyLimit::All,
            Some(&divide_by_N_sqrt),
        );

        if let Ok(spec) = spectrum {
            let band_peak = |low: f32, high: f32| -> f32 {
                let mut peak = 0.0f32;
                for (freq, val) in spec.data().iter() {
                    let f = freq.val();
                    if f >= low && f <= high {
                        peak = peak.max(val.val());
                    }
                }
                peak
            };

            let raw = [
                band_peak(20.0, 150.0),
                band_peak(150.0, 2000.0),
                band_peak(2000.0, 8000.0),
                band_peak(8000.0, 20000.0),
            ];

            let mut energies = [0.0f32; 4];
            for i in 0..4 {
                if raw[i] > self.peak_ref[i] {
                    self.peak_ref[i] = raw[i];
                } else {
                    self.peak_ref[i] *= 0.9985;
                }
                energies[i] = if self.peak_ref[i] > 1e-4 {
                    (raw[i] / self.peak_ref[i]).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }

            let mut bands_guard = self.state.bands.lock();
            for i in 0..4 {
                bands_guard[i] = self.bands[i].update(energies[i]);
            }
        }
    }
}

impl<I> Iterator for VisualizerSource<I>
where
    I: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.input.next()?;

        self.buffer.push(sample);

        if self.buffer.len() >= 1024 {
            self.process_spectrum();
            self.buffer.clear();
        }

        Some(sample)
    }
}

impl<I> Source for VisualizerSource<I>
where
    I: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> std::num::NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> std::num::NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.input.total_duration()
    }
}

pub struct F32Converter<I> {
    pub input: I,
}

impl<I> Iterator for F32Converter<I>
where
    I: Iterator,
    I::Item: ToF32,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.input.next().map(|s| s.to_f32_sample())
    }
}

impl<I> Source for F32Converter<I>
where
    I: Source,
    I::Item: ToF32,
{
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> std::num::NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> std::num::NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.input.total_duration()
    }
}
