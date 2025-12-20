use anyhow::{Context, Result};
use gpui::{App, BorrowAppContext, Global, Window};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, error, warn};

use crate::media::playback::Playback;
use crate::media::queue::Queue;

pub struct MediaKeyHandler {
    controls: Arc<Mutex<MediaControls>>,
    last_song_id: Arc<Mutex<Option<String>>>,
    last_is_playing: Arc<Mutex<Option<bool>>>,
}

impl Global for MediaKeyHandler {}

impl MediaKeyHandler {
    pub fn init(cx: &mut App, window: &Window) {
        match Self::new(cx, window) {
            Ok(handler) => cx.set_global(handler),
            Err(e) => error!("Failed to initialize MediaKeyHandler: {}", e),
        }
    }

    pub fn new(cx: &mut App, window: &Window) -> Result<Self> {
        #[cfg(target_os = "linux")]
        let hwnd = None;

        #[cfg(target_os = "windows")]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle_wrapper = HasWindowHandle::window_handle(window)
                .map_err(|e| anyhow::anyhow!("Failed to get window handle: {}", e))?;

            match handle_wrapper.as_raw() {
                RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            }
        };

        #[cfg(target_os = "macos")]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle_wrapper = HasWindowHandle::window_handle(window)
                .map_err(|e| anyhow::anyhow!("Failed to get window handle: {}", e))?;

            match handle_wrapper.as_raw() {
                RawWindowHandle::AppKit(handle) => {
                    Some(handle.ns_view.as_ptr() as *mut std::ffi::c_void)
                }
                _ => None,
            }
        };

        let config = PlatformConfig {
            dbus_name: "vleer",
            display_name: "Vleer",
            hwnd,
        };

        let mut controls =
            MediaControls::new(config).context("Failed to initialize media controls")?;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        controls
            .attach(move |event| {
                let _ = tx.send(event);
            })
            .context("Failed to attach media control events")?;

        cx.spawn(|cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                while let Some(event) = rx.recv().await {
                    cx.update(|cx| Self::handle_event(cx, event)).ok();
                }
            }
        })
        .detach();

        Ok(Self {
            controls: Arc::new(Mutex::new(controls)),
            last_song_id: Arc::new(Mutex::new(None)),
            last_is_playing: Arc::new(Mutex::new(None)),
        })
    }

    fn handle_event(cx: &mut App, event: MediaControlEvent) {
        debug!("Media Control Event: {:?}", event);
        match event {
            MediaControlEvent::Play | MediaControlEvent::Pause | MediaControlEvent::Toggle => {
                cx.update_global::<Playback, _>(|p: &mut Playback, cx| p.play_pause(cx));
            }
            MediaControlEvent::Next => {
                let _ = Queue::next(cx);
            }
            MediaControlEvent::Previous => {
                let _ = Queue::previous(cx);
            }
            MediaControlEvent::Stop => {
                cx.update_global::<Playback, _>(|p: &mut Playback, cx| p.pause(cx));
            }
            _ => {}
        }
        Self::update_playback(cx);
    }

    pub fn update_playback(cx: &mut App) {
        let (song, is_playing, progress_seconds) =
            cx.update_global::<Queue, _>(|queue: &mut Queue, cx| {
                let (playing, pos) = if let Some(p) = cx.try_global::<Playback>() {
                    (p.is_playing(), Some(p.get_position()))
                } else {
                    (false, None)
                };
                (queue.current().cloned(), playing, pos)
            });

        if let Some(handler) = cx.try_global::<MediaKeyHandler>() {
            let mut controls = match handler.controls.lock() {
                Ok(c) => c,
                Err(e) => {
                    error!("MediaControls lock poisoned: {}", e);
                    return;
                }
            };

            let mut last_playing = handler.last_is_playing.lock().unwrap();

            if *last_playing != Some(is_playing) {
                let position =
                    progress_seconds.map(|secs| MediaPosition(Duration::from_secs_f32(secs)));

                let playback_status = if is_playing {
                    MediaPlayback::Playing { progress: position }
                } else {
                    MediaPlayback::Paused { progress: position }
                };

                match controls.set_playback(playback_status) {
                    Ok(_) => {
                        *last_playing = Some(is_playing);
                    }
                    Err(e) => error!("Failed to set MPRIS playback: {}", e),
                }
            }

            let current_song_id = song.as_ref().map(|s| s.id.to_string());
            let mut last_id = handler.last_song_id.lock().unwrap();

            if *last_id != current_song_id {
                if let Some(song) = song {
                    let duration = Duration::from_secs_f32(song.duration as f32);

                    let cover_url = song.cover.as_deref().map(|path| {
                        if path.starts_with("http") || path.starts_with("file://") {
                            path.to_string()
                        } else {
                            if !path.starts_with("/") {
                                if let Some(base_dir) = dirs::data_dir() {
                                    let full_path =
                                        base_dir.join("vleer").join("covers").join(path);
                                    format!("file://{}", full_path.to_string_lossy())
                                } else {
                                    format!("file://{}", path)
                                }
                            } else {
                                format!("file://{}", path)
                            }
                        }
                    });

                    let metadata = MediaMetadata {
                        title: Some(&song.title),
                        artist: song.artist.as_ref().map(|a| a.name.as_str()),
                        album: song.album.as_ref().map(|a| a.title.as_str()),
                        duration: Some(duration),
                        cover_url: cover_url.as_deref(),
                    };

                    if let Err(e) = controls.set_metadata(metadata) {
                        error!("Failed to set MPRIS metadata: {}", e);
                    } else {
                        *last_id = current_song_id;
                    }
                } else {
                    *last_id = None;
                }
            }
        } else {
            warn!("MediaKeyHandler global not initialized");
        }
    }
}
