use anyhow::{Context, Result};
use gpui::{App, AsyncWindowContext, BorrowAppContext, Global, Window};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::debug;

use super::queue::Queue;

use super::equalizer::{Equalizer, EqualizerSource};
use super::normalization::{NormalizationSource, NormalizationState};
use crate::data::config::Config;

pub struct Playback {
    _stream: Arc<OutputStream>,
    sink: Arc<Sink>,
    equalizer: Arc<Mutex<Equalizer>>,
    normalization: Arc<std::sync::RwLock<NormalizationState>>,
    volume: f32,
    is_paused: bool,
    current_file: Option<String>,
    position: f32,
}

impl Global for Playback {}

impl Playback {
    const LOG_VOLUME_GROWTH_RATE: f32 = 6.908;
    const LOG_VOLUME_SCALE_FACTOR: f32 = 1000.0;
    const UNITY_GAIN: f32 = 1.0;

    pub fn new() -> Result<Self> {
        let stream = OutputStreamBuilder::open_default_stream()
            .context("Failed to open default audio output stream")?;

        let sink = Sink::connect_new(&stream.mixer());

        sink.pause();

        let equalizer = Arc::new(Mutex::new(Equalizer::new(44100, 2)));
        let normalization = Arc::new(std::sync::RwLock::new(NormalizationState::default()));

        Ok(Self {
            _stream: Arc::new(stream),
            sink: Arc::new(sink),
            equalizer,
            normalization,
            volume: 0.5,
            is_paused: true,
            current_file: None,
            position: 0.0,
        })
    }

    pub fn init(cx: &mut App) -> Result<()> {
        let mut context = Self::new()?;

        if let Some(settings) = cx.try_global::<Config>() {
            context.apply_settings(settings);
        }

        cx.set_global(context);
        Ok(())
    }

    pub fn load_file_with_replaygain(
        &mut self,
        path: impl AsRef<Path>,
        settings: &Config,
        replaygain_track_gain: Option<f32>,
        replaygain_track_peak: Option<f32>,
    ) -> Result<()> {
        let path = path.as_ref();
        debug!("Loading audio file: {:?}", path);

        let file =
            File::open(path).with_context(|| format!("Failed to open audio file: {:?}", path))?;

        let source = Decoder::new(BufReader::new(file)).context("Failed to decode audio file")?;

        let sample_rate = source.sample_rate();
        let channels = source.channels();

        debug!("Audio file info: {}Hz, {} channels", sample_rate, channels);

        *self.equalizer.lock().unwrap() =
            Equalizer::from_settings(sample_rate, &settings.get().equalizer);

        {
            let mut norm_state = self.normalization.write().unwrap();
            norm_state.enabled = settings.get().audio.normalization;
            norm_state.reset();

            if norm_state.enabled {
                if let Some(gain_db) = replaygain_track_gain {
                    norm_state.set_replaygain(gain_db, replaygain_track_peak);
                    debug!(
                        "Applied ReplayGain: {:.2} dB, peak: {:?}",
                        gain_db, replaygain_track_peak
                    );
                }
            }
        }

        self.sink.stop();

        let eq_source = EqualizerSource::new(source, self.equalizer.clone());
        let norm_source = NormalizationSource::new(eq_source, self.normalization.clone());
        self.sink.append(norm_source);
        self.sink.set_volume(Self::log_volume(self.volume));

        self.position = 0.0;

        self.current_file = Some(path.to_string_lossy().to_string());
        self.is_paused = true;

        debug!("Successfully loaded audio file");
        Ok(())
    }

    pub fn play_queue(cx: &mut App) -> Result<()> {
        let first_song = cx.update_global::<Queue, _>(|queue, _cx| queue.current().cloned());

        if let Some(song) = first_song {
            let config = cx.global::<Config>().clone();
            cx.update_global::<Playback, _>(|playback, _cx| {
                playback.load_file_with_replaygain(
                    &song.file_path,
                    &config,
                    song.replaygain_track_gain,
                    song.replaygain_track_peak,
                )?;
                playback.play();
                debug!("Started playback from queue");
                Ok(())
            })
        } else {
            debug!("Queue is empty, nothing to play");
            Ok(())
        }
    }

    pub fn play(&mut self) {
        if self.is_paused {
            self.sink.play();
            self.is_paused = false;
            debug!("Playback started");
        }
    }

    pub fn pause(&mut self) {
        if !self.is_paused {
            self.sink.pause();
            self.is_paused = true;
            debug!("Playback paused");
        }
    }

    pub fn toggle_play_pause(&mut self) {
        if self.is_paused {
            self.play();
        } else {
            self.pause();
        }
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.is_paused = true;
        self.current_file = None;
        debug!("Playback stopped");
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        let actual_vol = Self::log_volume(self.volume);
        self.sink.set_volume(actual_vol);
        debug!(
            "Volume set to {:.2} (actual: {:.2})",
            self.volume, actual_vol
        );
    }

    pub fn set_volume_and_save(&mut self, vol: f32, config: &mut Config) {
        self.set_volume(vol);

        config.get_mut().audio.volume = self.volume;
        if let Err(e) = config.save() {
            tracing::error!("Failed to save volume to config: {}", e);
        }
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

    pub fn current_file(&self) -> Option<&str> {
        self.current_file.as_deref()
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

    pub fn load_equalizer_from_settings(&mut self, settings: &Config) {
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
        let config = settings.get();

        self.set_volume(config.audio.volume);
        self.load_equalizer_from_settings(settings);
        self.set_normalization_enabled(config.audio.normalization);

        debug!("Applied all settings to playback context");
    }

    pub fn set_normalization_enabled(&mut self, enabled: bool) {
        let mut state = self.normalization.write().unwrap();
        state.enabled = enabled;
        debug!(
            "Normalization {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    pub fn is_normalization_enabled(&self) -> bool {
        self.normalization.read().unwrap().enabled
    }

    pub fn set_track_replaygain(&mut self, gain_db: f32, peak: Option<f32>) {
        let mut state = self.normalization.write().unwrap();
        state.set_replaygain(gain_db, peak);
        debug!("Set ReplayGain: {:.2} dB, peak: {:?}", gain_db, peak);
    }

    pub fn set_track_loudness(&mut self, lufs: f32) {
        let mut state = self.normalization.write().unwrap();
        state.set_track_loudness(lufs);
        debug!(
            "Set track loudness: {:.2} LUFS, gain: {:.3}",
            lufs, state.gain
        );
    }

    fn log_volume(volume: f32) -> f32 {
        let mut amplitude = volume;
        if amplitude > 0.0 && amplitude < Self::UNITY_GAIN {
            amplitude =
                f32::exp(Self::LOG_VOLUME_GROWTH_RATE * volume) / Self::LOG_VOLUME_SCALE_FACTOR;
            if volume < 0.1 {
                amplitude *= volume * 10.0;
            }
        }
        amplitude
    }

    pub fn seek(&mut self, position: f32) -> anyhow::Result<()> {
        if let Some(file_path) = &self.current_file {
            let was_playing = !self.is_paused;

            let file = File::open(file_path)?;
            let mut source: Decoder<BufReader<File>> = Decoder::try_from(file)?;
            source.try_seek(Duration::from_secs_f32(position)).ok();
            let eq_source = EqualizerSource::new(source, self.equalizer.clone());
            let norm_source = NormalizationSource::new(eq_source, self.normalization.clone());

            self.sink.stop();
            self.sink.append(norm_source);
            self.sink.set_volume(Self::log_volume(self.volume));

            self.position = position;

            if was_playing {
                self.sink.play();
                self.is_paused = false;
            } else {
                self.sink.pause();
                self.is_paused = true;
            }
        }
        Ok(())
    }

    pub fn get_position(&self) -> f32 {
        self.position + self.sink.get_pos().as_secs_f32()
    }

    pub fn is_empty(&self) -> bool {
        self.sink.empty()
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
