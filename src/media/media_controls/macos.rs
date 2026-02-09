use super::controllers::{ControllerBridge, InitMediaController, MediaController};
use crate::data::models::Cuid;
use crate::{data::db::repo::Database, data::models::Song, media::queue::RepeatMode};
use async_trait::async_trait;
use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSCopying;
use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSObject, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
    MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingPlaybackState, MPRemoteCommandCenter, MPRemoteCommandEvent,
    MPRemoteCommandHandlerStatus,
};
use raw_window_handle::RawWindowHandle;
use std::collections::HashMap;
use std::{path::Path, ptr::NonNull};
use tracing::debug;

pub struct MacMediaPlayerController {
    bridge: ControllerBridge,
    db: Database,
    last_song: Option<Song>,
    last_artist_name: Option<String>,
    last_album_title: Option<String>,
    last_duration: Option<u64>,
    last_position: Option<u64>,
    last_album_art: Option<Vec<u8>>,
    artist_name_cache: HashMap<Cuid, String>,
    album_title_cache: HashMap<Cuid, String>,
}

impl MacMediaPlayerController {
    fn update_file(&mut self, _path: &Path) {
        self.last_song = None;
        self.last_artist_name = None;
        self.last_album_title = None;
        self.last_album_art = None;
        self.update_now_playing();
    }

    async fn update_song(&mut self, song: &Song) {
        let is_new_song = self
            .last_song
            .as_ref()
            .map(|s| s.id != song.id)
            .unwrap_or(true);

        self.last_song = Some(song.clone());

        if is_new_song {
            if let Some(artist_id) = &song.artist_id {
                if let Some(cached_name) = self.artist_name_cache.get(artist_id) {
                    self.last_artist_name = Some(cached_name.clone());
                } else {
                    if let Ok(Some(artist)) = self.db.get_artist(artist_id.clone()).await {
                        self.last_artist_name = Some(artist.name.clone());
                        self.artist_name_cache
                            .insert(artist_id.clone(), artist.name);
                    }
                }
            } else {
                self.last_artist_name = None;
            }

            if let Some(album_id) = &song.album_id {
                if let Some(cached_title) = self.album_title_cache.get(album_id) {
                    self.last_album_title = Some(cached_title.clone());
                } else {
                    if let Ok(Some(album)) = self.db.get_album(album_id.clone()).await {
                        self.last_album_title = Some(album.title.clone());
                        self.album_title_cache.insert(album_id.clone(), album.title);
                    }
                }
            } else {
                self.last_album_title = None;
            }
        }

        self.update_now_playing();
    }

    fn update_duration(&mut self, duration: u64) {
        self.last_duration = Some(duration);
        self.update_now_playing();
    }

    fn update_position(&mut self, position: u64) {
        self.last_position = Some(position);
        self.update_now_playing();
    }

    fn update_album_art(&mut self, art: &[u8]) {
        debug!("Received album art, {} bytes", art.len());
        self.last_album_art = Some(art.to_vec());
        self.update_now_playing();
    }

    fn update_now_playing(&mut self) {
        unsafe {
            let media_center = MPNowPlayingInfoCenter::defaultCenter();
            let now_playing: Retained<NSMutableDictionary<NSString, NSObject>> =
                NSMutableDictionary::new();

            if let Some(song) = &self.last_song {
                let ns = NSString::from_str(&song.title);
                let key: &ProtocolObject<dyn NSCopying> =
                    ProtocolObject::from_ref(MPMediaItemPropertyTitle);
                now_playing.setObject_forKey(&*ns as &NSObject, key);
            }

            if let Some(artist) = &self.last_artist_name {
                let ns = NSString::from_str(artist);
                let key: &ProtocolObject<dyn NSCopying> =
                    ProtocolObject::from_ref(MPMediaItemPropertyArtist);
                now_playing.setObject_forKey(&*ns as &NSObject, key);
            }

            if let Some(album) = &self.last_album_title {
                let ns = NSString::from_str(album);
                let key: &ProtocolObject<dyn NSCopying> =
                    ProtocolObject::from_ref(MPMediaItemPropertyAlbumTitle);
                now_playing.setObject_forKey(&*ns as &NSObject, key);
            }

            if let Some(dur) = self.last_duration {
                let ns = NSNumber::numberWithUnsignedLong(dur);
                let key: &ProtocolObject<dyn NSCopying> =
                    ProtocolObject::from_ref(MPMediaItemPropertyPlaybackDuration);
                now_playing.setObject_forKey(&*ns as &NSObject, key);
            }

            if let Some(pos) = self.last_position {
                let ns = NSNumber::numberWithUnsignedLong(pos);
                let key: &ProtocolObject<dyn NSCopying> =
                    ProtocolObject::from_ref(MPNowPlayingInfoPropertyElapsedPlaybackTime);
                now_playing.setObject_forKey(&*ns as &NSObject, key);
            }

            if let Some(art) = &self.last_album_art {
                debug!("Processing album art for Now Playing");
                if let Ok(size) = imagesize::blob_size(art) {
                    debug!("Album art size: {}x{}", size.width, size.height);
                    let data = NSData::with_bytes(art);
                    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                        debug!("NSImage created successfully");

                        let image_retained = image.clone();

                        let request_handler =
                            RcBlock::new(move |size: CGSize| -> NonNull<NSImage> {
                                debug!(
                                    "Artwork request handler called with size: {}x{}",
                                    size.width, size.height
                                );
                                NonNull::from(&*image_retained)
                            });

                        let bounds_size = CGSize::new(size.width as f64, size.height as f64);
                        let artwork = MPMediaItemArtwork::initWithBoundsSize_requestHandler(
                            MPMediaItemArtwork::alloc(),
                            bounds_size,
                            &request_handler,
                        );

                        let key: &ProtocolObject<dyn NSCopying> =
                            ProtocolObject::from_ref(MPMediaItemPropertyArtwork);
                        now_playing.setObject_forKey(&*artwork as &NSObject, key);
                        debug!("Album artwork set in Now Playing");
                    } else {
                        debug!("Failed to create NSImage from data");
                    }
                } else {
                    debug!("Failed to determine image size");
                }
            }

            let dict = Retained::cast_unchecked(now_playing);
            media_center.setNowPlayingInfo(Some(&*dict));
        }
    }

    fn update_playback_state(&mut self, is_playing: bool) {
        unsafe {
            debug!(
                "Setting playback state: {}",
                if is_playing {
                    "playing"
                } else {
                    "paused/stopped"
                }
            );
            let media_center = MPNowPlayingInfoCenter::defaultCenter();
            media_center.setPlaybackState(if is_playing {
                MPNowPlayingPlaybackState::Playing
            } else {
                MPNowPlayingPlaybackState::Paused
            });
        }
    }

    fn setup_command_handlers(&self) {
        unsafe {
            let command_center = MPRemoteCommandCenter::sharedCommandCenter();

            let play_bridge = self.bridge.clone();
            let play_handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                play_bridge.play();
                MPRemoteCommandHandlerStatus::Success
            });
            let cmd = command_center.playCommand();
            cmd.setEnabled(true);
            cmd.addTargetWithHandler(&play_handler);

            let pause_bridge = self.bridge.clone();
            let pause_handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                pause_bridge.pause();
                MPRemoteCommandHandlerStatus::Success
            });
            let cmd = command_center.pauseCommand();
            cmd.setEnabled(true);
            cmd.addTargetWithHandler(&pause_handler);

            let toggle_bridge = self.bridge.clone();
            let toggle_handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                toggle_bridge.toggle_play_pause();
                MPRemoteCommandHandlerStatus::Success
            });
            let cmd = command_center.togglePlayPauseCommand();
            cmd.setEnabled(true);
            cmd.addTargetWithHandler(&toggle_handler);

            let prev_bridge = self.bridge.clone();
            let prev_handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                prev_bridge.previous();
                MPRemoteCommandHandlerStatus::Success
            });
            let cmd = command_center.previousTrackCommand();
            cmd.setEnabled(true);
            cmd.addTargetWithHandler(&prev_handler);

            let next_bridge = self.bridge.clone();
            let next_handler = RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| {
                next_bridge.next();
                MPRemoteCommandHandlerStatus::Success
            });
            let cmd = command_center.nextTrackCommand();
            cmd.setEnabled(true);
            cmd.addTargetWithHandler(&next_handler);

            let seek_bridge = self.bridge.clone();
            let seek_handler = RcBlock::new(move |mut event: NonNull<MPRemoteCommandEvent>| {
                if let Some(ev) = Retained::retain(event.as_mut()) {
                    let ev: Retained<MPChangePlaybackPositionCommandEvent> =
                        Retained::cast_unchecked(ev);
                    seek_bridge.seek(ev.positionTime());
                }
                MPRemoteCommandHandlerStatus::Success
            });
            let cmd = command_center.changePlaybackPositionCommand();
            cmd.setEnabled(true);
            cmd.addTargetWithHandler(&seek_handler);
        }
    }
}

#[async_trait]
impl MediaController for MacMediaPlayerController {
    async fn on_position_changed(&mut self, position_seconds: u64) -> anyhow::Result<()> {
        self.update_position(position_seconds);
        Ok(())
    }

    async fn on_duration_changed(&mut self, duration_seconds: u64) -> anyhow::Result<()> {
        self.update_duration(duration_seconds);
        Ok(())
    }

    async fn on_volume_changed(&mut self, _volume: f64) -> anyhow::Result<()> {
        Ok(())
    }

    async fn on_song_changed(&mut self, song: &Song) -> anyhow::Result<()> {
        self.update_song(song).await;
        Ok(())
    }

    async fn on_album_art_changed(&mut self, album_art: &[u8]) -> anyhow::Result<()> {
        self.update_album_art(album_art);
        Ok(())
    }

    async fn on_repeat_changed(&mut self, _repeat_mode: RepeatMode) -> anyhow::Result<()> {
        Ok(())
    }

    async fn on_playback_state_changed(&mut self, is_playing: bool) -> anyhow::Result<()> {
        self.update_playback_state(is_playing);
        Ok(())
    }

    async fn on_shuffle_changed(&mut self, _is_shuffled: bool) -> anyhow::Result<()> {
        Ok(())
    }

    async fn on_file_loaded(&mut self, path: &Path) -> anyhow::Result<()> {
        self.update_file(path);
        Ok(())
    }
}

#[async_trait(?Send)]
impl InitMediaController for MacMediaPlayerController {
    async fn init(
        bridge: ControllerBridge,
        _handle: Option<RawWindowHandle>,
        db: Database,
    ) -> anyhow::Result<Box<Self>> {
        let controller = MacMediaPlayerController {
            bridge,
            db,
            last_song: None,
            last_artist_name: None,
            last_album_title: None,
            last_duration: None,
            last_position: None,
            last_album_art: None,
            artist_name_cache: HashMap::new(),
            album_title_cache: HashMap::new(),
        };
        controller.setup_command_handlers();
        Ok(Box::new(controller))
    }
}
