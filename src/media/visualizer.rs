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

            let normalize = |v: f32| (v * 0.1).clamp(0.05, 1.0);

            let mut bands = self.state.bands.lock().unwrap();
            *bands = [normalize(b1), normalize(b2), normalize(b3), normalize(b4)];
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
