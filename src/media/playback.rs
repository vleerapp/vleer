use anyhow::{Context, Result};
use gpui::{App, AsyncWindowContext, BorrowAppContext, Global, Window};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::debug;

use super::equalizer::{Equalizer, EqualizerSource};
use super::queue::Queue;
use crate::data::config::Config;
use crate::media::media_keys::MediaKeyHandler;
use crate::media::visualizer::{F32Converter, VisualizerSource, VisualizerState};

const LOG_VOLUME_GROWTH_RATE: f32 = 6.908;
const LOG_VOLUME_SCALE_FACTOR: f32 = 1000.0;
const UNITY_GAIN: f32 = 1.0;
const DEFAULT_TARGET_LUFS: f32 = -14.0;

pub struct Playback {
    _stream: Option<OutputStream>,
    sink: Option<Sink>,
    equalizer: Arc<Mutex<Equalizer>>,
    volume: f32,
    is_paused: bool,
    current_file: Option<String>,
    current_track_lufs: Option<f32>,
    normalization_enabled: bool,
    position: f32,
    visualizer_state: VisualizerState,
}

impl Global for Playback {}

impl Playback {
    pub fn new() -> Result<Self> {
        let equalizer = Arc::new(Mutex::new(Equalizer::new(44100, 2)));

        Ok(Self {
            _stream: None,
            sink: None,
            equalizer,
            volume: 0.5,
            is_paused: true,
            current_file: None,
            current_track_lufs: None,
            normalization_enabled: false,
            position: 0.0,
            visualizer_state: VisualizerState::default(),
        })
    }

    pub fn init(cx: &mut App) -> Result<()> {
        let mut context = Self::new()?;
        let settings = cx.global::<Config>();
        context.apply_settings(settings);
        cx.set_global(context);
        Ok(())
    }

    pub fn open(
        &mut self,
        path: impl AsRef<Path>,
        settings: &Config,
        track_lufs: Option<f32>,
    ) -> Result<()> {
        let path = path.as_ref();
        debug!("Loading audio file: {:?}", path);

        let file =
            File::open(path).with_context(|| format!("Failed to open audio file: {:?}", path))?;

        let decoder = Decoder::new(BufReader::new(file)).context("Failed to decode audio file")?;

        let source = F32Converter { input: decoder };

        let sample_rate = source.sample_rate();
        let channels = source.channels();

        debug!("Audio file info: {}Hz, {} channels", sample_rate, channels);

        let builder = OutputStreamBuilder::from_default_device()
            .context("Failed to create output stream builder")?;

        let stream = builder
            .with_sample_rate(sample_rate)
            .open_stream()
            .context("Failed to open output stream with custom sample rate")?;

        let sink = Sink::connect_new(stream.mixer());

        *self.equalizer.lock().unwrap() =
            Equalizer::from_settings(sample_rate, &settings.get().equalizer);

        self.current_track_lufs = track_lufs;
        self.normalization_enabled = settings.get().audio.normalization;

        let eq_source = EqualizerSource::new(source, self.equalizer.clone());
        let vis_source = VisualizerSource::new(eq_source, self.visualizer_state.clone());
        let gain = self.calculate_normalization_gain();
        let normalized = vis_source.amplify(gain);

        sink.append(normalized);
        sink.set_volume(Self::log_volume(self.volume));
        sink.pause();

        self._stream = Some(stream);
        self.sink = Some(sink);

        self.position = 0.0;
        self.current_file = Some(path.to_string_lossy().to_string());
        self.is_paused = true;

        debug!("Successfully loaded audio file with rate: {}", sample_rate);
        Ok(())
    }

    pub fn play_queue(cx: &mut App) -> Result<()> {
        let first_song = cx.update_global::<Queue, _>(|queue, _cx| queue.current().cloned());

        if let Some(song) = first_song {
            let config = cx.global::<Config>().clone();
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.open(&song.file_path, &config, song.track_lufs)?;
                playback.play(cx);
                debug!("Started playback from queue");
                Ok(())
            })
        } else {
            debug!("Queue is empty, nothing to play");
            Ok(())
        }
    }

    pub fn play(&mut self, cx: &mut App) {
        if self.is_paused {
            if let Some(sink) = &self.sink {
                sink.play();
                self.is_paused = false;
                debug!("Playback started");

                MediaKeyHandler::update_playback(cx);
            }
        }
    }

    pub fn pause(&mut self, cx: &mut App) {
        if !self.is_paused {
            if let Some(sink) = &self.sink {
                sink.pause();
                self.is_paused = true;
                debug!("Playback paused");

                MediaKeyHandler::update_playback(cx);
            }
        }
    }

    pub fn play_pause(&mut self, cx: &mut App) {
        if self.is_paused {
            self.play(cx);
        } else {
            self.pause(cx);
        }
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        let actual_vol = Self::log_volume(self.volume);

        if let Some(sink) = &self.sink {
            sink.set_volume(actual_vol);
        }
        debug!(
            "Volume set to {:.2} (actual: {:.2})",
            self.volume, actual_vol
        );
    }

    fn log_volume(volume: f32) -> f32 {
        let mut amplitude = volume;
        if amplitude > 0.0 && amplitude < UNITY_GAIN {
            amplitude = f32::exp(LOG_VOLUME_GROWTH_RATE * volume) / LOG_VOLUME_SCALE_FACTOR;
            if volume < 0.1 {
                amplitude *= volume * 10.0;
            }
        }
        amplitude
    }

    pub fn get_volume(&self) -> f32 {
        self.volume
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    pub fn is_playing(&self) -> bool {
        !self.is_paused
    }

    pub fn apply_equalizer_settings(&mut self, gains: &[f32], q_values: &[f32]) {
        let mut eq = self.equalizer.lock().unwrap();
        for i in 0..10.min(gains.len()).min(q_values.len()) {
            eq.set_gain(i, gains[i]);
            eq.set_q(i, q_values[i]);
        }
        debug!("Applied equalizer settings");
    }

    pub fn set_equalizer_gain(&mut self, band: usize, gain_db: f32) {
        if band < 10 {
            self.equalizer.lock().unwrap().set_gain(band, gain_db);
        }
    }

    pub fn set_equalizer_q(&mut self, band: usize, q: f32) {
        if band < 10 {
            self.equalizer.lock().unwrap().set_q(band, q);
        }
    }

    pub fn set_equalizer_enabled(&mut self, enabled: bool) {
        if !enabled {
            let mut eq = self.equalizer.lock().unwrap();
            for i in 0..10 {
                eq.set_gain(i, 0.0);
            }
            debug!("Equalizer disabled");
        } else {
            debug!("Equalizer enabled");
        }
    }

    pub fn get_equalizer_gains(&self) -> Vec<f32> {
        let eq = self.equalizer.lock().unwrap();
        eq.get_bands().iter().map(|b| b.gain_db).collect()
    }

    pub fn get_equalizer_qs(&self) -> Vec<f32> {
        let eq = self.equalizer.lock().unwrap();
        eq.get_bands().iter().map(|b| b.q).collect()
    }

    pub fn load_eq_settings(&mut self, settings: &Config) {
        let config = settings.get();
        let eq_settings = &config.equalizer;

        let mut eq = self.equalizer.lock().unwrap();
        eq.apply_settings(eq_settings);

        debug!(
            "Loaded equalizer settings from config (enabled: {})",
            eq_settings.enabled
        );
    }

    pub fn apply_settings(&mut self, settings: &Config) {
        let volume = settings.get().audio.volume;
        let normalization = settings.get().audio.normalization;

        self.set_volume(volume);
        self.load_eq_settings(settings);
        self.set_normalization(normalization);

        debug!("Applied all settings to playback context");
    }

    pub fn set_normalization(&mut self, enabled: bool) {
        self.normalization_enabled = enabled;
        debug!(
            "Normalization {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    fn calculate_normalization_gain(&self) -> f32 {
        if self.normalization_enabled {
            if let Some(lufs) = self.current_track_lufs {
                let gain_db = (DEFAULT_TARGET_LUFS - lufs).clamp(-12.0, 12.0);
                let linear_gain = 10.0f32.powf(gain_db / 20.0);

                debug!(
                    "Normalization - Track LUFS: {:.2}, Gain dB: {:.2}, Linear: {:.4}",
                    lufs, gain_db, linear_gain
                );

                return linear_gain;
            } else {
                debug!("No LUFS data available, normalization disabled for this track");
            }
        }
        1.0
    }

    pub fn seek(&mut self, position: f32) -> anyhow::Result<()> {
        if let Some(file_path) = &self.current_file {
            let was_playing = !self.is_paused;

            let file = File::open(file_path)?;
            let mut source: Decoder<BufReader<File>> = Decoder::try_from(file)?;
            source.try_seek(Duration::from_secs_f32(position)).ok();

            let eq_source = EqualizerSource::new(source, self.equalizer.clone());
            let vis_source = VisualizerSource::new(eq_source, self.visualizer_state.clone());
            let gain = self.calculate_normalization_gain();
            let normalized = vis_source.amplify(gain);

            if let Some(sink) = &self.sink {
                sink.stop();
                sink.append(normalized);
                sink.set_volume(Self::log_volume(self.volume));

                if was_playing {
                    sink.play();
                    self.is_paused = false;
                } else {
                    sink.pause();
                    self.is_paused = true;
                }
            }
            self.position = position;
        }
        Ok(())
    }

    pub fn get_position(&self) -> f32 {
        if let Some(sink) = &self.sink {
            self.position + sink.get_pos().as_secs_f32()
        } else {
            0.0
        }
    }

    pub fn is_empty(&self) -> bool {
        if let Some(sink) = &self.sink {
            sink.empty()
        } else {
            true
        }
    }

    pub fn get_spectrum(&self) -> [f32; 4] {
        *self.visualizer_state.bands.lock().unwrap()
    }

    pub fn start_playback_monitor<T: 'static>(window: &Window, cx: &mut gpui::Context<T>) {
        cx.spawn_in(window, |_entity, cx: &mut AsyncWindowContext| {
            let mut cx = cx.clone();
            async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    let should_advance = cx
                        .update(|_window, cx| {
                            if let Some(playback) = cx.try_global::<Playback>() {
                                playback.is_empty() && playback.is_playing()
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);

                    if should_advance {
                        cx.update(|_window, cx| {
                            if let Err(e) = Queue::next(cx) {
                                tracing::error!("Failed to auto-advance: {}", e);
                            }
                        })
                        .ok();
                    }

                    cx.update(|window, _cx| {
                        window.refresh();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }
}
