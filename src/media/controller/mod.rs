use crate::data::db::repo::Database;
use crate::data::models::Song;
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use anyhow::Result;
use gpui::{App, Global};
#[cfg(target_os = "windows")]
use gpui::Window;
#[cfg(target_os = "windows")]
use raw_window_handle::HasWindowHandle;
use std::sync::Arc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux::LinuxController as PlatformController;
#[cfg(target_os = "macos")]
use macos::MacosController as PlatformController;
#[cfg(target_os = "windows")]
use windows::WindowsController as PlatformController;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResolvedMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub position_ms: Option<u64>,
    pub artwork_id: Option<String>,
    pub artwork_data: Option<Vec<u8>>,
    pub track_id: Option<String>,
}

#[derive(Clone)]
pub struct MediaController {
    inner: Arc<MediaControllerInner>,
}

struct MediaControllerInner {
    db: Database,
    platform: PlatformController,
}

impl Global for MediaController {}

impl MediaController {
    pub fn init(cx: &mut App) {
        let playback_tx = Playback::get_command_sender(cx);
        let platform = PlatformController::new(playback_tx);
        let db = cx.global::<Database>().clone();
        let controller = MediaController {
            inner: Arc::new(MediaControllerInner { db, platform }),
        };

        cx.set_global(controller.clone());
        Self::start_monitor(cx, controller);
    }

    pub async fn set_state(&self, state: PlaybackState) -> Result<()> {
        self.inner.platform.set_state(state).await
    }

    pub async fn update_song(&self, song: Song) -> Result<()> {
        let db = self.inner.db.clone();

        let artist = match song.artist_id.clone() {
            Some(id) => db.get_artist(id).await.ok().flatten().map(|a| a.name),
            None => None,
        };

        let album = match song.album_id.clone() {
            Some(id) => db.get_album(id).await.ok().flatten().map(|a| a.title),
            None => None,
        };

        let artwork_data = match song.image_id.as_deref() {
            Some(id) => db.get_image(id).await.ok().flatten().map(|i| i.data),
            None => None,
        };

        let artwork_id = if artwork_data.is_some() {
            song.image_id.clone().or_else(|| Some(song.id.to_string()))
        } else {
            None
        };

        let metadata = ResolvedMetadata {
            title: Some(song.title),
            artist,
            album,
            duration_ms: Some(song.duration.max(0) as u64 * 1000),
            position_ms: Some(0),
            artwork_id,
            artwork_data,
            track_id: Some(format!("/app/vleer/track/{}", song.id)),
        };

        self.inner.platform.update_metadata(metadata).await
    }

    pub async fn set_position_ms(&self, position_ms: u64) -> Result<()> {
        self.inner.platform.set_position(position_ms).await
    }

    pub async fn set_can_go_next(&self, can_go_next: bool) -> Result<()> {
        self.inner.platform.set_can_go_next(can_go_next).await
    }

    pub async fn set_can_go_previous(&self, can_go_previous: bool) -> Result<()> {
        self.inner.platform.set_can_go_previous(can_go_previous).await
    }

    #[cfg(target_os = "windows")]
    pub fn set_window_handle(&self, window: &Window) {
        if let Ok(handle) = window.window_handle() {
            if let raw_window_handle::WindowHandle::Win32(handle) = handle {
                let hwnd = handle.hwnd.get();
                self.inner.platform.set_window_handle(hwnd).ok();
            }
        }
    }

    fn start_monitor(cx: &mut App, controller: MediaController) {
        cx.spawn(async move |cx| {
            let mut last_position_ms: Option<u64> = None;
            let mut last_can_next: Option<bool> = None;
            let mut last_can_prev: Option<bool> = None;

            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                let (position_ms, can_next, can_prev) = cx.update(|app| {
                    let position_ms = app
                        .try_global::<Playback>()
                        .map(|p| (p.get_position().max(0.0) * 1000.0) as u64)
                        .unwrap_or(0);
                    let can_next = app
                        .try_global::<Queue>()
                        .map(|q| q.has_next())
                        .unwrap_or(false);
                    let can_prev = app
                        .try_global::<Queue>()
                        .map(|q| q.has_previous())
                        .unwrap_or(false);
                    (position_ms, can_next, can_prev)
                });

                if last_position_ms.map_or(true, |prev| prev != position_ms) {
                    controller.set_position_ms(position_ms).await.ok();
                    last_position_ms = Some(position_ms);
                }

                if last_can_next.map_or(true, |prev| prev != can_next) {
                    controller.set_can_go_next(can_next).await.ok();
                    last_can_next = Some(can_next);
                }

                if last_can_prev.map_or(true, |prev| prev != can_prev) {
                    controller.set_can_go_previous(can_prev).await.ok();
                    last_can_prev = Some(can_prev);
                }
            }
        })
        .detach();
    }
}
