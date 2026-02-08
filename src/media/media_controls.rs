use base64::Engine;
use gpui::{App, AsyncApp, BorrowAppContext, Global};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, error, warn};

use crate::data::db::repo::Database;
use crate::media::playback::Playback;
use crate::media::queue::Queue;

#[derive(Debug, Clone)]
pub enum MediaKeyEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    SeekForward,
    SeekBackward,
    SetPosition(Duration),
}

#[derive(Debug)]
pub enum MediaControlsError {
    InitFailed(String),
    AttachFailed(String),
    UpdateFailed(String),
    WindowHandleFailed(String),
}

impl std::fmt::Display for MediaControlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaControlsError::InitFailed(e) => {
                write!(f, "Failed to initialize media controls: {}", e)
            }
            MediaControlsError::AttachFailed(e) => {
                write!(f, "Failed to attach media controls: {}", e)
            }
            MediaControlsError::UpdateFailed(e) => {
                write!(f, "Failed to update media controls: {}", e)
            }
            MediaControlsError::WindowHandleFailed(e) => {
                write!(f, "Failed to get window handle: {}", e)
            }
        }
    }
}

impl std::error::Error for MediaControlsError {}

pub struct MediaControlsHandler {
    controls: MediaControls,
    receiver: Receiver<MediaKeyEvent>,
}

impl MediaControlsHandler {
    pub fn new() -> Result<Self, MediaControlsError> {
        let config = PlatformConfig {
            dbus_name: "vleer",
            display_name: "Vleer",
            hwnd: None,
        };

        let mut controls = MediaControls::new(config)
            .map_err(|e| MediaControlsError::InitFailed(e.to_string()))?;

        let (sender, receiver) = mpsc::channel::<MediaKeyEvent>();

        Self::attach_handler(&mut controls, sender)?;

        Ok(Self { controls, receiver })
    }

    fn attach_handler(
        controls: &mut MediaControls,
        sender: Sender<MediaKeyEvent>,
    ) -> Result<(), MediaControlsError> {
        controls
            .attach(move |event: MediaControlEvent| {
                let media_event = match event {
                    MediaControlEvent::Play => Some(MediaKeyEvent::Play),
                    MediaControlEvent::Pause => Some(MediaKeyEvent::Pause),
                    MediaControlEvent::Toggle => Some(MediaKeyEvent::Toggle),
                    MediaControlEvent::Next => Some(MediaKeyEvent::Next),
                    MediaControlEvent::Previous => Some(MediaKeyEvent::Previous),
                    MediaControlEvent::Stop => Some(MediaKeyEvent::Stop),
                    MediaControlEvent::Seek(SeekDirection::Forward) => {
                        Some(MediaKeyEvent::SeekForward)
                    }
                    MediaControlEvent::Seek(SeekDirection::Backward) => {
                        Some(MediaKeyEvent::SeekBackward)
                    }
                    MediaControlEvent::SetPosition(pos) => Some(MediaKeyEvent::SetPosition(pos.0)),
                    _ => None,
                };

                if let Some(event) = media_event {
                    let _ = sender.send(event);
                }
            })
            .map_err(|e| MediaControlsError::AttachFailed(e.to_string()))
    }

    pub fn try_recv(&self) -> Option<MediaKeyEvent> {
        self.receiver.try_recv().ok()
    }

    pub fn set_metadata(
        &mut self,
        title: Option<&str>,
        artist: Option<&str>,
        album: Option<&str>,
        duration: Option<Duration>,
        cover_url: Option<&str>,
    ) -> Result<(), MediaControlsError> {
        self.controls
            .set_metadata(MediaMetadata {
                title,
                artist,
                album,
                duration,
                cover_url,
            })
            .map_err(|e| MediaControlsError::UpdateFailed(e.to_string()))
    }

    pub fn set_playback_playing(
        &mut self,
        position: Option<Duration>,
    ) -> Result<(), MediaControlsError> {
        let progress = position.map(MediaPosition);
        self.controls
            .set_playback(MediaPlayback::Playing { progress })
            .map_err(|e| MediaControlsError::UpdateFailed(e.to_string()))
    }

    pub fn set_playback_paused(
        &mut self,
        position: Option<Duration>,
    ) -> Result<(), MediaControlsError> {
        let progress = position.map(MediaPosition);
        self.controls
            .set_playback(MediaPlayback::Paused { progress })
            .map_err(|e| MediaControlsError::UpdateFailed(e.to_string()))
    }

    pub fn set_playback_stopped(&mut self) -> Result<(), MediaControlsError> {
        self.controls
            .set_playback(MediaPlayback::Stopped)
            .map_err(|e| MediaControlsError::UpdateFailed(e.to_string()))
    }
}

pub struct MediaKeyHandler {
    inner: Arc<Mutex<MediaControlsHandler>>,
    last_song_id: Arc<Mutex<Option<String>>>,
    last_is_playing: Arc<Mutex<Option<bool>>>,
}

impl Global for MediaKeyHandler {}

impl MediaKeyHandler {
    pub fn init(cx: &mut App) {
        match Self::new() {
            Ok(handler) => {
                let inner_ref = handler.inner.clone();
                cx.set_global(handler);

                cx.spawn({
                    let inner_ref = inner_ref.clone();
                    async move |cx: &mut AsyncApp| {
                        Self::event_loop(cx, inner_ref).await;
                    }
                })
                .detach();
            }
            Err(e) => error!("{}", e),
        }
    }

    fn new() -> Result<Self, MediaControlsError> {
        let handler = MediaControlsHandler::new()?;
        Ok(Self {
            inner: Arc::new(Mutex::new(handler)),
            last_song_id: Arc::new(Mutex::new(None)),
            last_is_playing: Arc::new(Mutex::new(None)),
        })
    }

    async fn event_loop(cx: &mut AsyncApp, handler: Arc<Mutex<MediaControlsHandler>>) {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;

            let events = {
                let mut out = Vec::new();
                if let Ok(guard) = handler.lock() {
                    while let Some(ev) = guard.try_recv() {
                        out.push(ev);
                    }
                }
                out
            };

            if events.is_empty() {
                continue;
            }

            let _ = cx.update(|cx: &mut App| {
                for event in events {
                    Self::handle_gpui_event(cx, event);
                }
                Self::update_playback(cx);
            });
        }
    }

    fn handle_gpui_event(cx: &mut App, event: MediaKeyEvent) {
        debug!("Media Key Event: {:?}", event);
        match event {
            MediaKeyEvent::Play | MediaKeyEvent::Pause | MediaKeyEvent::Toggle => {
                cx.update_global::<Playback, _>(|p: &mut Playback, cx: &mut App| p.play_pause(cx));
            }
            MediaKeyEvent::Next => {
                cx.update_global::<Queue, _>(|queue, cx| queue.next(cx));
            }
            MediaKeyEvent::Previous => {
                cx.update_global::<Queue, _>(|queue, cx| queue.previous(cx));
            }
            MediaKeyEvent::Stop => {
                cx.update_global::<Playback, _>(|p: &mut Playback, cx: &mut App| p.pause(cx));
            }
            _ => {}
        }
    }

    pub fn update_playback(cx: &mut App) {
        let (song, is_playing, progress_seconds) = cx.update_global::<Queue, _>(|queue, cx| {
            let (playing, pos) = if let Some(p) = cx.try_global::<Playback>() {
                (p.get_playing(), Some(p.get_position()))
            } else {
                (false, None)
            };
            (queue.get_current_song(cx), playing, pos)
        });

        if let Some(global_handler) = cx.try_global::<MediaKeyHandler>() {
            let mut handler = match global_handler.inner.lock() {
                Ok(h) => h,
                Err(e) => {
                    error!("MediaControls lock poisoned: {}", e);
                    return;
                }
            };

            let mut last_playing = global_handler.last_is_playing.lock().unwrap();

            if *last_playing != Some(is_playing) {
                let position = progress_seconds.map(Duration::from_secs_f32);
                let res = if is_playing {
                    handler.set_playback_playing(position)
                } else {
                    handler.set_playback_paused(position)
                };

                if let Err(e) = res {
                    error!("{}", e);
                } else {
                    *last_playing = Some(is_playing);
                }
            }

            let current_song_id = song.as_ref().map(|s| s.id.to_string());
            let mut last_id = global_handler.last_song_id.lock().unwrap();

            if *last_id != current_song_id {
                if let Some(song) = song {
                    let db = cx.global::<Database>();

                    let artist_name = song
                        .artist_id
                        .as_ref()
                        .and_then(|id| {
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current()
                                    .block_on(db.get_artist(id.clone()))
                                    .ok()
                                    .flatten()
                            })
                        })
                        .map(|a| a.name.clone());

                    let album_title = song
                        .album_id
                        .as_ref()
                        .and_then(|id| {
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current()
                                    .block_on(db.get_album(id.clone()))
                                    .ok()
                                    .flatten()
                            })
                        })
                        .map(|a| a.title.clone());

                    let duration = Duration::from_secs_f32(song.duration as f32);
                    let cover_url = song
                        .image_id
                        .as_deref()
                        .and_then(|id| resolve_cover_url_from_db(&db, id));

                    if let Err(e) = handler.set_metadata(
                        Some(&song.title),
                        artist_name.as_deref(),
                        album_title.as_deref(),
                        Some(duration),
                        cover_url.as_deref(),
                    ) {
                        error!("{}", e);
                    } else {
                        *last_id = current_song_id;
                    }
                } else {
                    let _ = handler.set_playback_stopped();
                    *last_id = None;
                }
            }
        } else {
            warn!("MediaKeyHandler global not initialized");
        }
    }
}

fn resolve_cover_url_from_db(db: &Database, image_id: &str) -> Option<String> {
    let image = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(db.get_image(image_id))
            .ok()
            .flatten()
    })?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&image.data);
    Some(format!("data:image/jpeg;base64,{}", encoded))
}
