use super::equalizer::{Equalizer, EqualizerSource};
use super::queue::Queue;
use crate::data::config::Config;
use crate::media::controller::{MediaController, PlaybackState};
use crate::media::visualizer::{F32Converter, VisualizerSource, VisualizerState};
use anyhow::{Context, Result};
use gpui::{App, AsyncWindowContext, BorrowAppContext, Global, Window};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::debug;

const LOG_VOLUME_GROWTH_RATE: f32 = 6.908;
const LOG_VOLUME_SCALE_FACTOR: f32 = 1000.0;
const UNITY_GAIN: f32 = 1.0;
const DEFAULT_TARGET_LUFS: f32 = -14.0;

#[derive(Debug, Clone)]
pub enum PlaybackCommand {
    PlayPause,
    Next,
    Previous,
    Seek(f32),
}

pub struct Playback {
    _stream: Option<OutputStream>,
    sink: Option<Sink>,
    equalizer: Arc<Mutex<Equalizer>>,
    volume: f32,
    paused: bool,
    current_file: Option<String>,
    current_lufs: Option<f32>,
    normalization_enabled: bool,
    position: f32,
    visualizer_state: VisualizerState,
    command_rx: Option<mpsc::UnboundedReceiver<PlaybackCommand>>,
}

impl Global for Playback {}

use std::sync::OnceLock;
static PLAYBACK_CMD_TX: OnceLock<mpsc::UnboundedSender<PlaybackCommand>> = OnceLock::new();

impl Playback {
    fn new() -> Result<Self> {
        let equalizer = Arc::new(Mutex::new(Equalizer::new(44100, 2)));

        Ok(Self {
            _stream: None,
            sink: None,
            equalizer,
            volume: 0.5,
            paused: true,
            current_file: None,
            current_lufs: None,
            normalization_enabled: false,
            position: 0.0,
            visualizer_state: VisualizerState::default(),
            command_rx: None,
        })
    }

    pub fn init(cx: &mut App) -> Result<()> {
        let (tx, rx) = mpsc::unbounded_channel();
        PLAYBACK_CMD_TX.set(tx).ok();

        let mut playback = Self::new()?;
        playback.command_rx = Some(rx);
        let config = cx.global::<Config>();
        playback.apply_config(config);
        cx.set_global(playback);

        Self::start_command_processor(cx);

        Ok(())
    }

    pub fn get_command_sender(cx: &App) -> mpsc::UnboundedSender<PlaybackCommand> {
        PLAYBACK_CMD_TX
            .get()
            .expect("Playback not initialized")
            .clone()
    }

    fn start_command_processor(cx: &mut App) {
        let mut rx =
            cx.update_global::<Playback, _>(|playback, _cx| playback.command_rx.take().unwrap());

        cx.spawn(async move |cx| {
            while let Some(cmd) = rx.recv().await {
                cx.update(|cx| match cmd {
                    PlaybackCommand::PlayPause => {
                        cx.update_global::<Playback, _>(|playback, cx| {
                            playback.play_pause(cx);
                        });
                    }
                    PlaybackCommand::Next => {
                        cx.update_global::<Playback, _>(|playback, cx| {
                            playback.next(cx);
                        });
                    }
                    PlaybackCommand::Previous => {
                        cx.update_global::<Playback, _>(|playback, cx| {
                            playback.previous(cx);
                        });
                    }
                    PlaybackCommand::Seek(position) => {
                        cx.update_global::<Playback, _>(|playback, _cx| {
                            playback.seek(position).ok();
                        });
                    }
                });
            }
        })
        .detach();
    }

    pub fn open(
        &mut self,
        path: impl AsRef<Path>,
        config: &Config,
        lufs: Option<f32>,
    ) -> Result<()> {
        let path = path.as_ref();
        debug!("Opening audio file: {:?}", path);

        let file =
            File::open(path).with_context(|| format!("Failed to open audio file: {:?}", path))?;

        let decoder = Decoder::new(BufReader::new(file)).context("Failed to decode audio file")?;

        let source = F32Converter { input: decoder };
        let sample_rate = source.sample_rate();
        let channels = source.channels();

        debug!("Audio file: {}Hz, {} channels", sample_rate, channels);

        let stream = OutputStreamBuilder::from_default_device()
            .context("Failed to create output stream builder")?
            .with_sample_rate(sample_rate)
            .open_stream()
            .context("Failed to open output stream")?;

        let sink = Sink::connect_new(stream.mixer());

        *self.equalizer.lock().unwrap() =
            Equalizer::from_settings(sample_rate, &config.get().equalizer);

        self.current_lufs = lufs;
        self.normalization_enabled = config.get().audio.normalization;

        let eq_source = EqualizerSource::new(source, self.equalizer.clone());
        let vis_source = VisualizerSource::new(eq_source, self.visualizer_state.clone());
        let gain = self.compute_normalization_gain();
        let normalized = vis_source.amplify(gain);

        sink.append(normalized);
        sink.set_volume(Self::compute_log_volume(self.volume));
        sink.pause();

        self._stream = Some(stream);
        self.sink = Some(sink);
        self.position = 0.0;
        self.current_file = Some(path.to_string_lossy().to_string());
        self.paused = true;

        debug!("Loaded audio file");
        Ok(())
    }

    pub fn play(&mut self, cx: &mut App) {
        if self.paused {
            if let Some(sink) = &self.sink {
                sink.play();
                self.paused = false;
                debug!("Started playback");

                if let Some(mc) = cx.try_global::<MediaController>() {
                    let mc = mc.clone();
                    tokio::spawn(async move {
                        mc.set_state(PlaybackState::Playing).await.ok();
                    });
                }
            }
        }
    }

    pub fn pause(&mut self, cx: &mut App) {
        if !self.paused {
            if let Some(sink) = &self.sink {
                sink.pause();
                self.paused = true;
                debug!("Paused playback");

                if let Some(mc) = cx.try_global::<MediaController>() {
                    let mc = mc.clone();
                    tokio::spawn(async move {
                        mc.set_state(PlaybackState::Paused).await.ok();
                    });
                }
            }
        }
    }

    pub fn play_pause(&mut self, cx: &mut App) {
        if self.paused {
            self.play(cx);
        } else {
            self.pause(cx);
        }
    }

    pub fn seek(&mut self, position: f32) -> Result<()> {
        if let Some(file_path) = &self.current_file {
            let was_playing = !self.paused;

            let file = File::open(file_path)?;
            let mut source: Decoder<BufReader<File>> = Decoder::try_from(file)?;
            source.try_seek(Duration::from_secs_f32(position)).ok();

            let eq_source = EqualizerSource::new(source, self.equalizer.clone());
            let vis_source = VisualizerSource::new(eq_source, self.visualizer_state.clone());
            let gain = self.compute_normalization_gain();
            let normalized = vis_source.amplify(gain);

            if let Some(sink) = &self.sink {
                sink.stop();
                sink.append(normalized);
                sink.set_volume(Self::compute_log_volume(self.volume));

                if was_playing {
                    sink.play();
                    self.paused = false;
                } else {
                    sink.pause();
                    self.paused = true;
                }
            }
            self.position = position;
        }
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f32, _cx: &mut App) {
        self.volume = volume.clamp(0.0, 1.0);
        let log_volume = Self::compute_log_volume(self.volume);

        if let Some(sink) = &self.sink {
            sink.set_volume(log_volume);
        }

        debug!("Volume: {:.2} (log: {:.2})", self.volume, log_volume);
    }

    pub fn get_volume(&self) -> f32 {
        self.volume
    }

    pub fn get_paused(&self) -> bool {
        self.paused
    }

    pub fn get_playing(&self) -> bool {
        !self.paused
    }

    pub fn get_position(&self) -> f32 {
        if let Some(sink) = &self.sink {
            self.position + sink.get_pos().as_secs_f32()
        } else {
            0.0
        }
    }

    pub fn empty(&self) -> bool {
        self.sink.as_ref().map_or(true, |s| s.empty())
    }

    pub fn get_spectrum(&self) -> [f32; 4] {
        *self.visualizer_state.bands.lock().unwrap()
    }

    pub fn set_eq_gain(&mut self, band: usize, gain_db: f32) {
        if band < 10 {
            self.equalizer.lock().unwrap().set_gain(band, gain_db);
        }
    }

    pub fn set_eq_q(&mut self, band: usize, q: f32) {
        if band < 10 {
            self.equalizer.lock().unwrap().set_q(band, q);
        }
    }

    pub fn set_eq_gains(&self) -> Vec<f32> {
        self.equalizer
            .lock()
            .unwrap()
            .get_bands()
            .iter()
            .map(|b| b.gain_db)
            .collect()
    }

    pub fn set_eq_qs(&self) -> Vec<f32> {
        self.equalizer
            .lock()
            .unwrap()
            .get_bands()
            .iter()
            .map(|b| b.q)
            .collect()
    }

    pub fn apply_eq_settings(&mut self, gains: &[f32], q_values: &[f32]) {
        let mut eq = self.equalizer.lock().unwrap();
        for i in 0..10.min(gains.len()).min(q_values.len()) {
            eq.set_gain(i, gains[i]);
            eq.set_q(i, q_values[i]);
        }
        debug!("Applied EQ settings");
    }

    pub fn set_eq_enabled(&mut self, enabled: bool) {
        if !enabled {
            let mut eq = self.equalizer.lock().unwrap();
            for i in 0..10 {
                eq.set_gain(i, 0.0);
            }
        }
        debug!("EQ {}", if enabled { "enabled" } else { "disabled" });
    }

    pub fn set_normalization(&mut self, enabled: bool) {
        self.normalization_enabled = enabled;
        debug!(
            "Normalization {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    pub fn apply_config(&mut self, config: &Config) {
        let settings = config.get();
        self.volume = settings.audio.volume;
        self.set_normalization(settings.audio.normalization);

        let mut eq = self.equalizer.lock().unwrap();
        eq.apply_settings(&settings.equalizer);

        debug!("Applied config to playback");
    }

    pub fn play_queue(&mut self, cx: &mut App) {
        let song = cx.update_global::<Queue, _>(|queue, cx| queue.get_current_song(cx));
        let config = cx.global::<Config>().clone();
        if let Some(song) = song {
            if let Err(e) = self.open(&song.file_path, &config, song.lufs) {
                tracing::error!("Failed to open track: {}", e);
                return;
            }
            self.play(cx);

            if let Some(mc) = cx.try_global::<MediaController>() {
                let mc = mc.clone();
                let song = song.clone();
                tokio::spawn(async move {
                    mc.update_song(song).await.ok();
                });
            }
        }
    }

    pub fn next(&mut self, cx: &mut App) {
        let song = cx.update_global::<Queue, _>(|queue, cx| queue.next(cx));
        let config = cx.global::<Config>().clone();
        if let Some(song) = song {
            if let Err(e) = self.open(&song.file_path, &config, song.lufs) {
                tracing::error!("Failed to open next track: {}", e);
                return;
            }
            self.play(cx);

            if let Some(mc) = cx.try_global::<MediaController>() {
                let mc = mc.clone();
                let song = song.clone();
                tokio::spawn(async move {
                    mc.update_song(song).await.ok();
                });
            }
        }
    }

    pub fn previous(&mut self, cx: &mut App) {
        let song = cx.update_global::<Queue, _>(|queue, cx| queue.previous(cx));
        let config = cx.global::<Config>().clone();
        if let Some(song) = song {
            if let Err(e) = self.open(&song.file_path, &config, song.lufs) {
                tracing::error!("Failed to open previous track: {}", e);
                return;
            }
            self.play(cx);

            if let Some(mc) = cx.try_global::<MediaController>() {
                let mc = mc.clone();
                let song = song.clone();
                tokio::spawn(async move {
                    mc.update_song(song).await.ok();
                });
            }
        }
    }

    pub fn start_monitor<T: 'static>(window: &Window, cx: &mut gpui::Context<T>) {
        cx.spawn_in(window, |_entity, cx: &mut AsyncWindowContext| {
            let mut cx = cx.clone();
            async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                    let should_advance = cx
                        .update(|_window, cx| {
                            cx.try_global::<Playback>()
                                .map(|p| p.empty() && p.get_playing())
                                .unwrap_or(false)
                        })
                        .unwrap_or(false);

                    if should_advance {
                        cx.update(|_window, cx| {
                            cx.update_global::<Playback, _>(|playback, cx| {
                                playback.next(cx);
                            });
                        })
                        .ok();
                    }

                    cx.update(|window, _cx| window.refresh()).ok();
                }
            }
        })
        .detach();
    }

    fn compute_log_volume(volume: f32) -> f32 {
        let mut amplitude = volume;
        if amplitude > 0.0 && amplitude < UNITY_GAIN {
            amplitude = f32::exp(LOG_VOLUME_GROWTH_RATE * volume) / LOG_VOLUME_SCALE_FACTOR;
            if volume < 0.1 {
                amplitude *= volume * 10.0;
            }
        }
        amplitude
    }

    fn compute_normalization_gain(&self) -> f32 {
        if self.normalization_enabled {
            if let Some(lufs) = self.current_lufs {
                let gain_db = (DEFAULT_TARGET_LUFS - lufs).clamp(-12.0, 12.0);
                let linear_gain = 10.0f32.powf(gain_db / 20.0);
                debug!("Normalization: LUFS {:.2}, gain {:.2} dB", lufs, gain_db);
                return linear_gain;
            }
        }
        1.0
    }
}
