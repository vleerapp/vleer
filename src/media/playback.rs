use super::equalizer::{Equalizer, EqualizerSource};
use super::queue::Queue;
use crate::data::config::{Config, EqualizerSettings};
use crate::data::db::repo::Database;
use crate::data::models::Cuid;
use crate::media::controller::{MediaController, PlaybackState};
use crate::media::visualizer::{F32Converter, VisualizerSource, VisualizerState};
use anyhow::{Context, Result};
use gpui::{App, AsyncWindowContext, BorrowAppContext, Global, Window};
use rodio::decoder::{Decoder, DecoderBuilder};
use rodio::source::Source;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player as Sink};
use std::fs::File;
use std::io::BufReader;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use symphonia::core::codecs::CodecRegistry;
use symphonia::default::register_enabled_codecs;
use symphonia_adapter_libopus::OpusDecoder;
use tokio::sync::mpsc;
use tracing::{debug, error};

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

struct PreparedPlayback {
    _device: MixerDeviceSink,
    sink: Sink,
    current_file: String,
}

pub struct Playback {
    _device: Option<MixerDeviceSink>,
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
    load_token: u64,
}

impl Global for Playback {}

static PLAYBACK_CMD_TX: OnceLock<mpsc::UnboundedSender<PlaybackCommand>> = OnceLock::new();

fn get_codec_registry() -> Arc<CodecRegistry> {
    static REGISTRY: OnceLock<Arc<CodecRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| {
            let mut registry = CodecRegistry::new();
            registry.register_all::<OpusDecoder>();
            register_enabled_codecs(&mut registry);
            Arc::new(registry)
        })
        .clone()
}

impl Playback {
    fn compute_normalization_gain_for(
        normalization_enabled: bool,
        current_lufs: Option<f32>,
    ) -> f32 {
        if normalization_enabled {
            if let Some(lufs) = current_lufs {
                let gain_db = (DEFAULT_TARGET_LUFS - lufs).clamp(-12.0, 12.0);
                let linear_gain = 10.0f32.powf(gain_db / 20.0);
                debug!("Normalization: LUFS {:.2}, gain {:.2} dB", lufs, gain_db);
                return linear_gain;
            }
        }
        1.0
    }

    fn prepare_playback(
        path: String,
        lufs: Option<f32>,
        volume: f32,
        normalization_enabled: bool,
        eq_settings: EqualizerSettings,
        equalizer: Arc<Mutex<Equalizer>>,
        visualizer_state: VisualizerState,
    ) -> Result<PreparedPlayback> {
        let file =
            File::open(&path).with_context(|| format!("Failed to open audio file: {:?}", path))?;

        let decoder = DecoderBuilder::new()
            .with_codec_registry(get_codec_registry())
            .with_data(BufReader::new(file))
            .build()
            .context("Failed to decode audio file")?;

        let source = F32Converter { input: decoder };
        let sample_rate = source.sample_rate();
        let channels = source.channels();

        debug!("Audio file: {:?}Hz, {:?} channels", sample_rate, channels);

        let device = DeviceSinkBuilder::open_default_sink()
            .context("Failed to open default audio device")?;

        let sink = Sink::connect_new(device.mixer());

        let sample_rate_u32: u32 = sample_rate.get();
        *equalizer.lock().unwrap() = Equalizer::from_settings(sample_rate_u32, &eq_settings);

        let eq_source = EqualizerSource::new(source, equalizer.clone());
        let vis_source = VisualizerSource::new(eq_source, visualizer_state);
        let gain = Self::compute_normalization_gain_for(normalization_enabled, lufs);
        let normalized = vis_source.amplify(gain);

        sink.append(normalized);
        sink.set_volume(Self::compute_log_volume(volume));
        sink.pause();

        Ok(PreparedPlayback {
            _device: device,
            sink,
            current_file: path,
        })
    }

    fn load_song_by_id(&mut self, cx: &mut App, song_id: Cuid) {
        let db = cx.global::<Database>().clone();
        let config = cx.global::<Config>().clone();
        let eq_settings = config.get().equalizer.clone();
        let normalization_enabled = config.get().audio.normalization;
        let equalizer = self.equalizer.clone();
        let visualizer_state = self.visualizer_state.clone();
        let volume = self.volume;

        self.load_token = self.load_token.wrapping_add(1);
        let token = self.load_token;

        cx.spawn(async move |cx| {
            let song = db.get_song(song_id.clone()).await.ok().flatten();
            let Some(song) = song else {
                return;
            };

            let path = song.file_path.clone();
            let lufs = song.lufs;

            let prepared = tokio::task::spawn_blocking(move || {
                Playback::prepare_playback(
                    path,
                    lufs,
                    volume,
                    normalization_enabled,
                    eq_settings,
                    equalizer,
                    visualizer_state,
                )
            })
            .await;

            let prepared = match prepared {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(e)) => {
                    error!("Failed to open track: {}", e);
                    return;
                }
                Err(e) => {
                    error!("Failed to open track: {}", e);
                    return;
                }
            };

            cx.update(|cx| {
                let mut applied = false;
                cx.update_global::<Playback, _>(|playback, cx| {
                    if playback.load_token != token {
                        return;
                    }

                    playback._device = Some(prepared._device);
                    playback.sink = Some(prepared.sink);
                    playback.position = 0.0;
                    playback.current_file = Some(prepared.current_file);
                    playback.current_lufs = lufs;
                    playback.normalization_enabled = normalization_enabled;
                    playback.paused = true;
                    playback.play(cx);
                    applied = true;
                });

                if !applied {
                    return;
                }

                cx.update_global::<Queue, _>(|queue, _cx| {
                    queue.set_current_song_cache(song.id.clone(), song.clone());
                });

                if let Some(mc) = cx.try_global::<MediaController>() {
                    let mc = mc.clone();
                    let song = song.clone();
                    tokio::spawn(async move {
                        mc.update_song(song).await.ok();
                    });
                }
            });
        })
        .detach();
    }

    fn new() -> Result<Self> {
        let equalizer = Arc::new(Mutex::new(Equalizer::new(44100, 2)));

        Ok(Self {
            _device: None,
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
            load_token: 0,
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

    pub fn get_command_sender(_cx: &App) -> mpsc::UnboundedSender<PlaybackCommand> {
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
            let mut source: Decoder<BufReader<File>> = DecoderBuilder::new()
                .with_codec_registry(get_codec_registry())
                .with_data(BufReader::new(file))
                .build()?;
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
        let song_id = cx.update_global::<Queue, _>(|queue, _| queue.get_current_song_id());
        if let Some(song_id) = song_id {
            self.load_song_by_id(cx, song_id);
        }
    }

    pub fn next(&mut self, cx: &mut App) {
        let song_id = cx.update_global::<Queue, _>(|queue, _| queue.advance_next_id());
        if let Some(song_id) = song_id {
            self.load_song_by_id(cx, song_id);
        }
    }

    pub fn previous(&mut self, cx: &mut App) {
        let song_id = cx.update_global::<Queue, _>(|queue, _| queue.advance_previous_id());
        if let Some(song_id) = song_id {
            self.load_song_by_id(cx, song_id);
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
