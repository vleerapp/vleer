use std::path::{Path, PathBuf};

use async_trait::async_trait;
use futures::StreamExt;
use gpui::{App, BorrowAppContext, Global, Window};
use itertools::Itertools as _;
#[cfg(target_os = "linux")]
use raw_window_handle::HasWindowHandle;
use raw_window_handle::RawWindowHandle;
use rustc_hash::FxHashMap;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{Instrument as _, debug, debug_span, error, trace_span, warn};

use crate::data::metadata::AudioMetadata;
use crate::data::models::Song;
use crate::{
    data::db::repo::Database,
    media::{
        playback::Playback,
        queue::{Queue, RepeatMode},
    },
};

#[cfg(target_os = "macos")]
use super::macos;

#[derive(Debug, Clone)]
pub enum PlaybackCommand {
    Play,
    Pause,
    TogglePlayPause,
    Stop,
    Next,
    Previous,
    Jump(usize),
    Seek(f64),
    SetVolume(f64),
    ToggleShuffle,
    SetRepeat(RepeatMode),
}

#[async_trait(?Send)]
pub trait InitMediaController: MediaController {
    async fn init(
        bridge: ControllerBridge,
        handle: Option<RawWindowHandle>,
        db: Database,
    ) -> anyhow::Result<Box<Self>>;
}

#[async_trait]
pub trait MediaController: Send {
    async fn on_position_changed(&mut self, position_seconds: u64) -> anyhow::Result<()>;
    async fn on_duration_changed(&mut self, duration_seconds: u64) -> anyhow::Result<()>;
    async fn on_volume_changed(&mut self, volume: f64) -> anyhow::Result<()>;
    async fn on_song_changed(&mut self, song: &Song) -> anyhow::Result<()>;
    async fn on_album_art_changed(&mut self, album_art: &[u8]) -> anyhow::Result<()>;
    async fn on_repeat_changed(&mut self, repeat_mode: RepeatMode) -> anyhow::Result<()>;
    async fn on_playback_state_changed(&mut self, is_playing: bool) -> anyhow::Result<()>;
    async fn on_shuffle_changed(&mut self, is_shuffled: bool) -> anyhow::Result<()>;
    async fn on_file_loaded(&mut self, path: &Path) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct ControllerBridge {
    playback_command_tx: UnboundedSender<PlaybackCommand>,
}

impl ControllerBridge {
    pub fn new(playback_command_tx: UnboundedSender<PlaybackCommand>) -> Self {
        Self {
            playback_command_tx,
        }
    }

    pub fn play(&self) {
        let _ = self.playback_command_tx.send(PlaybackCommand::Play);
    }

    pub fn pause(&self) {
        let _ = self.playback_command_tx.send(PlaybackCommand::Pause);
    }

    pub fn toggle_play_pause(&self) {
        let _ = self
            .playback_command_tx
            .send(PlaybackCommand::TogglePlayPause);
    }

    pub fn stop(&self) {
        let _ = self.playback_command_tx.send(PlaybackCommand::Stop);
    }

    pub fn next(&self) {
        let _ = self.playback_command_tx.send(PlaybackCommand::Next);
    }

    pub fn previous(&self) {
        let _ = self.playback_command_tx.send(PlaybackCommand::Previous);
    }

    pub fn jump(&self, index: usize) {
        let _ = self.playback_command_tx.send(PlaybackCommand::Jump(index));
    }

    pub fn seek(&self, position: f64) {
        let _ = self
            .playback_command_tx
            .send(PlaybackCommand::Seek(position));
    }

    pub fn set_volume(&self, volume: f64) {
        let _ = self
            .playback_command_tx
            .send(PlaybackCommand::SetVolume(volume));
    }

    pub fn toggle_shuffle(&self) {
        let _ = self
            .playback_command_tx
            .send(PlaybackCommand::ToggleShuffle);
    }

    pub fn set_repeat(&self, repeat: RepeatMode) {
        let _ = self
            .playback_command_tx
            .send(PlaybackCommand::SetRepeat(repeat));
    }
}

enum ControllerType {
    #[cfg(target_os = "macos")]
    Mac(macos::MacMediaPlayerController),
}

impl ControllerType {
    async fn handle_event(&mut self, event: &MediaControllerEvent) -> anyhow::Result<()> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Mac(controller) => event.dispatch(controller).await,
        }
    }
}

type ControllerList = FxHashMap<String, ControllerType>;

#[allow(dead_code)]
pub struct MediaControllerHandle {
    event_tx: UnboundedSender<MediaControllerEvent>,
    cmd_tx: UnboundedSender<PlaybackCommand>,
    task: tokio::task::JoinHandle<()>,
}

impl Global for MediaControllerHandle {}

impl MediaControllerHandle {
    pub fn command_sender(&self) -> UnboundedSender<PlaybackCommand> {
        self.cmd_tx.clone()
    }

    pub fn event_sender(&self) -> UnboundedSender<MediaControllerEvent> {
        self.event_tx.clone()
    }

    pub fn send_song(&self, song: Song) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::SongChanged(Box::new(song)));
    }

    pub fn send_playback_state(&self, is_playing: bool) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::PlaybackStateChanged(is_playing));
    }

    pub fn send_position(&self, position: u64) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::PositionChanged(position));
    }

    pub fn send_duration(&self, duration: u64) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::DurationChanged(duration));
    }

    pub fn send_file_loaded(&self, path: PathBuf) {
        let _ = self.event_tx.send(MediaControllerEvent::FileLoaded(path));
    }

    pub fn send_volume(&self, volume: f64) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::VolumeChanged(volume));
    }

    pub fn send_repeat_state(&self, state: RepeatMode) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::RepeatStateChanged(state));
    }

    pub fn send_shuffle_state(&self, shuffling: bool) {
        let _ = self
            .event_tx
            .send(MediaControllerEvent::ShuffleStateChanged(shuffling));
    }
}

#[derive(Debug)]
pub enum MediaControllerEvent {
    SongChanged(Box<Song>),
    AlbumArtChanged(Box<[u8]>),
    PositionChanged(u64),
    DurationChanged(u64),
    FileLoaded(PathBuf),
    VolumeChanged(f64),
    RepeatStateChanged(RepeatMode),
    PlaybackStateChanged(bool),
    ShuffleStateChanged(bool),
}

impl MediaControllerEvent {
    async fn dispatch<T: MediaController>(&self, controller: &mut T) -> anyhow::Result<()> {
        match self {
            Self::SongChanged(song) => controller.on_song_changed(song).await,
            Self::AlbumArtChanged(art) => controller.on_album_art_changed(art).await,
            Self::PositionChanged(pos) => controller.on_position_changed(*pos).await,
            Self::DurationChanged(dur) => controller.on_duration_changed(*dur).await,
            Self::FileLoaded(path) => controller.on_file_loaded(path).await,
            Self::VolumeChanged(vol) => controller.on_volume_changed(*vol).await,
            Self::RepeatStateChanged(state) => controller.on_repeat_changed(*state).await,
            Self::PlaybackStateChanged(playing) => {
                controller.on_playback_state_changed(*playing).await
            }
            Self::ShuffleStateChanged(shuffle) => controller.on_shuffle_changed(*shuffle).await,
        }
    }
}

fn process_playback_command(cmd: PlaybackCommand, cx: &mut App) {
    match cmd {
        PlaybackCommand::Play => {
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.play(cx);
            });
        }
        PlaybackCommand::Pause => {
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.pause(cx);
            });
        }
        PlaybackCommand::TogglePlayPause => {
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.play_pause(cx);
            });
        }
        PlaybackCommand::Stop => {
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.pause(cx);
            });
        }
        PlaybackCommand::Next => {
            if let Err(e) = Playback::next(cx) {
                error!("Failed to skip to next track: {}", e);
            }
        }
        PlaybackCommand::Previous => {
            if let Err(e) = Playback::previous(cx) {
                error!("Failed to skip to previous track: {}", e);
            }
        }
        PlaybackCommand::Jump(index) => {
            let song = cx.update_global::<Queue, _>(|queue, cx| queue.set_current_index(index, cx));

            if let Some(song) = song {
                let config = cx.global::<crate::data::config::Config>().clone();
                cx.update_global::<Playback, _>(|playback, cx| {
                    if let Ok(()) = playback.open(&song.file_path, &config, song.lufs) {
                        playback.play(cx);
                    }
                });

                if let Some(mc) = cx.try_global::<MediaControllerHandle>() {
                    let sender = mc.event_sender();
                    let song_path = song.file_path.clone();
                    let song_clone = song.clone();
                    let tag_path = song_path.clone();
                    let db = cx.global::<Database>().clone();

                    tokio::spawn(async move {
                        let audio_meta = tokio::task::spawn_blocking(move || {
                            AudioMetadata::from_path_with_options(
                                std::path::Path::new(&tag_path),
                                false,
                            )
                            .ok()
                        })
                        .await
                        .ok()
                        .flatten();

                        let mut final_song = song_clone.clone();

                        if let Some(meta) = audio_meta.as_ref() {
                            if let Some(title) = &meta.title {
                                final_song.title = title.clone();
                            }
                            if let Some(genre) = &meta.genre {
                                final_song.genre = Some(genre.clone());
                            }
                            if let Some(year) = meta.year {
                                final_song.date = Some(year.to_string());
                            }
                            if let Some(track) = meta.track_number {
                                final_song.track_number = Some(track as i32);
                            }
                        }

                        let _ = sender.send(MediaControllerEvent::SongChanged(Box::new(
                            final_song.clone(),
                        )));

                        if let Some(dur) = audio_meta.as_ref().map(|m| m.duration.as_secs()) {
                            let _ = sender.send(MediaControllerEvent::DurationChanged(dur));
                        }

                        if let Some(image_id) = &final_song.image_id {
                            if let Ok(Some(image_row)) = db.get_image(image_id).await {
                                let _ = sender.send(MediaControllerEvent::AlbumArtChanged(
                                    image_row.data.into_boxed_slice(),
                                ));
                            }
                        } else if let Some(album_id) = &final_song.album_id {
                            if let Ok(Some(album)) = db.get_album(album_id.clone()).await {
                                if let Some(album_image_id) = album.image_id {
                                    if let Ok(Some(image_row)) = db.get_image(&album_image_id).await
                                    {
                                        let _ = sender.send(MediaControllerEvent::AlbumArtChanged(
                                            image_row.data.into_boxed_slice(),
                                        ));
                                    }
                                }
                            }
                        }

                        let _ = sender.send(MediaControllerEvent::FileLoaded(
                            std::path::PathBuf::from(song_path),
                        ));
                    });
                }
            }
        }
        PlaybackCommand::Seek(position) => {
            cx.update_global::<Playback, _>(|playback, _cx| {
                let _ = playback.seek(position as f32);
            });
        }
        PlaybackCommand::SetVolume(volume) => {
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.set_volume(volume as f32, cx);
            });
        }
        PlaybackCommand::ToggleShuffle => {
            cx.update_global::<Queue, _>(|queue, _cx| {
                queue.set_shuffle(queue.get_shuffle() ^ true);
            });
        }
        PlaybackCommand::SetRepeat(repeat) => {
            cx.update_global::<Queue, _>(|queue, _cx| {
                queue.set_repeat_mode(repeat);
            });
        }
    }
}

pub fn init_media_controllers(cx: &mut App, _window: &Window) {
    let mut list = ControllerList::default();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<PlaybackCommand>();
    let bridge = ControllerBridge::new(cmd_tx.clone());

    #[cfg(target_os = "linux")]
    let _rwh = HasWindowHandle::window_handle(_window)
        .ok()
        .map(|v| v.as_raw());

    #[cfg(target_os = "macos")]
    {
        let bridge_clone = bridge.clone();
        let db_clone = cx.global::<Database>().clone();
        match std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(macos::MacMediaPlayerController::init(
                bridge_clone,
                None,
                db_clone,
            ))
        })
        .join()
        {
            Ok(Ok(macos_controller)) => {
                list.insert("macos".to_string(), ControllerType::Mac(*macos_controller));
                debug!("MacMediaPlayerController initialized successfully");
            }
            Ok(Err(e)) => {
                error!("Failed to initialize MacMediaPlayerController: {e}");
                warn!("Desktop integration will be unavailable.");
            }
            Err(_) => {
                error!("MacMediaPlayerController initialization panicked");
                warn!("Desktop integration will be unavailable.");
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        warn!("Linux MPRIS controller not yet implemented");
    }

    #[cfg(target_os = "windows")]
    {
        warn!("Windows controller not yet implemented");
    }

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<MediaControllerEvent>();

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Some(cmd) = cmd_rx.recv().await {
            let _ = cx.update(|cx| {
                process_playback_command(cmd, cx);
            });
        }
    })
    .detach();

    let task = tokio::spawn(async move {
        let span = debug_span!("media_controllers", controllers = %list.keys().format(","));
        while let Some(event) = event_rx.recv().await {
            let span = trace_span!(parent: &span, "dispatch_event", ?event);
            futures::stream::iter(&mut list)
                .for_each_concurrent(None, async |(name, controller)| {
                    if let Err(err) = controller
                        .handle_event(&event)
                        .instrument(trace_span!(parent: &span, "handle", controller = %name))
                        .await
                    {
                        error!(?err, "controller '{name}': {err}");
                    }
                })
                .await;
        }
        tracing::info!("channel closed, ending task");
    });

    cx.set_global(MediaControllerHandle {
        event_tx,
        cmd_tx,
        task,
    });
}
