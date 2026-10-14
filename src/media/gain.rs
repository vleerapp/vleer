use rodio::Source;
use rodio::source::SeekError;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

const RAMP_SECS: f32 = 0.012;
const SEEK_BLEND_SECS: f32 = 0.006;

pub struct GainControl {
    volume: AtomicU32,
    muted: AtomicBool,
    silent: AtomicBool,
}

impl GainControl {
    pub fn new(volume: f32, muted: bool) -> Arc<Self> {
        Arc::new(Self {
            volume: AtomicU32::new(volume.to_bits()),
            muted: AtomicBool::new(muted),
            silent: AtomicBool::new(muted),
        })
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn set_muted(&self, muted: bool) {
        if !muted {
            self.silent.store(false, Ordering::Relaxed);
        }
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn is_silent(&self) -> bool {
        self.silent.load(Ordering::Relaxed)
    }

    fn target(&self) -> f32 {
        if self.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.volume.load(Ordering::Relaxed))
        }
    }
}

pub struct GainSource<I> {
    input: I,
    control: Arc<GainControl>,
    current: f32,
    step: f32,
    channels: usize,
    channel: usize,
    held: Vec<f32>,
    blend_total: u32,
    blend_left: u32,
}

impl<I: Source<Item = f32>> GainSource<I> {
    pub fn new(input: I, control: Arc<GainControl>) -> Self {
        let rate = input.sample_rate().get() as f32;
        let channels = input.channels().get() as usize;
        Self {
            input,
            control,
            current: 0.0,
            step: 1.0 / (rate * RAMP_SECS).max(1.0),
            channels,
            channel: 0,
            held: vec![0.0; channels],
            blend_total: (rate * SEEK_BLEND_SECS).max(1.0) as u32,
            blend_left: 0,
        }
    }
}

impl<I: Source<Item = f32>> Iterator for GainSource<I> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.channel == 0 {
            let target = self.control.target();
            let delta = target - self.current;
            if delta.abs() <= self.step {
                self.current = target;
            } else {
                self.current += self.step * delta.signum();
            }
            self.control
                .silent
                .store(self.current == 0.0 && target == 0.0, Ordering::Relaxed);
        }

        let sample = self.input.next()?;
        let mut out = sample * self.current;

        if self.blend_left > 0 {
            let t = 1.0 - self.blend_left as f32 / self.blend_total as f32;
            out = self.held[self.channel] * (1.0 - t) + out * t;
        }
        self.held[self.channel] = out;

        self.channel += 1;
        if self.channel == self.channels {
            self.channel = 0;
            self.blend_left = self.blend_left.saturating_sub(1);
        }
        Some(out)
    }
}

impl<I: Source<Item = f32>> Source for GainSource<I> {
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    fn channels(&self) -> NonZero<u16> {
        self.input.channels()
    }

    fn sample_rate(&self) -> NonZero<u32> {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.input.try_seek(pos)?;
        self.blend_left = self.blend_total;
        Ok(())
    }
}
