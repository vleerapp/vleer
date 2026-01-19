use rodio::Source;
use spectrum_analyzer::scaling::divide_by_N_sqrt;
use spectrum_analyzer::{FrequencyLimit, samples_fft_to_spectrum};
use std::sync::{Arc, Mutex};

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

pub struct VisualizerSource<I> {
    input: I,
    buffer: Vec<f32>,
    state: VisualizerState,
    _channels: u16,
    sample_rate: u32,
}

impl<I> VisualizerSource<I>
where
    I: Source<Item = f32>,
{
    pub fn new(input: I, state: VisualizerState) -> Self {
        let channels = input.channels();
        let sample_rate = input.sample_rate();
        Self {
            input,
            buffer: Vec::with_capacity(1024),
            state,
            _channels: channels,
            sample_rate,
        }
    }

    fn process_spectrum(&mut self) {
        let spectrum = samples_fft_to_spectrum(
            &self.buffer,
            self.sample_rate,
            FrequencyLimit::All,
            Some(&divide_by_N_sqrt),
        );

        if let Ok(spec) = spectrum {
            let b1 = spec.freq_val_exact(40.0).val() + spec.freq_val_exact(80.0).val();
            let b2 = spec.freq_val_exact(120.0).val() + spec.freq_val_exact(250.0).val();
            let b3 = spec.freq_val_exact(400.0).val() + spec.freq_val_exact(800.0).val();
            let b4 = spec.freq_val_exact(1200.0).val() + spec.freq_val_exact(2400.0).val();

            let gains: [f32; 4] = [0.3, 0.7, 0.9, 1.0];

            let base_mul: f32 = 0.07;

            let alphas: [f32; 4] = [0.08, 0.18, 0.28, 0.35];

            let targets: [f32; 4] = [
                (b1 * gains[0] * base_mul).clamp(0.02, 1.0),
                (b2 * gains[1] * base_mul).clamp(0.02, 1.0),
                (b3 * gains[2] * base_mul).clamp(0.02, 1.0),
                (b4 * gains[3] * base_mul).clamp(0.02, 1.0),
            ];

            let mut bands_guard = self.state.bands.lock().unwrap();
            let prev = *bands_guard;

            let mut new_bands = [0.0_f32; 4];
            for i in 0..4 {
                let alpha = alphas[i];
                new_bands[i] = prev[i] * (1.0 - alpha) + targets[i] * alpha;
            }

            *bands_guard = new_bands;
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

        if self.buffer.len() < 1024 {
            self.buffer.push(sample);
        } else {
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

    fn channels(&self) -> u16 {
        self.input.channels()
    }

    fn sample_rate(&self) -> u32 {
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

    fn channels(&self) -> u16 {
        self.input.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.input.total_duration()
    }
}
