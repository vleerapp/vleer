use super::{PlaybackState, ResolvedMetadata};
use crate::media::playback::PlaybackCommand;
use anyhow::{anyhow, Result};
use image::ImageFormat;
use mpris_server::{Metadata, PlaybackStatus, Player, Time, TrackId};
use std::cell::Cell;
use std::rc::Rc;
use std::{fs, thread};
use tokio::sync::mpsc;
use tracing::error;
use url::Url;

pub struct LinuxController {
    tx: mpsc::UnboundedSender<Command>,
}

enum Command {
    UpdateMetadata(ResolvedMetadata),
    SetState(PlaybackState),
    SetPosition(u64),
    SetCanGoNext(bool),
    SetCanGoPrevious(bool),
}

impl LinuxController {
    pub fn new(playback_tx: mpsc::UnboundedSender<PlaybackCommand>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        thread::spawn(move || {
            if let Err(err) = run_mpris(rx, playback_tx) {
                error!(?err, "mpris controller stopped");
            }
        });

        Self { tx }
    }

    pub async fn update_metadata(&self, metadata: ResolvedMetadata) -> Result<()> {
        self.tx
            .send(Command::UpdateMetadata(metadata))
            .map_err(|_| anyhow!("mpris command channel closed"))
    }

    pub async fn set_state(&self, state: PlaybackState) -> Result<()> {
        self.tx
            .send(Command::SetState(state))
            .map_err(|_| anyhow!("mpris command channel closed"))
    }

    pub async fn set_position(&self, position_ms: u64) -> Result<()> {
        self.tx
            .send(Command::SetPosition(position_ms))
            .map_err(|_| anyhow!("mpris command channel closed"))
    }

    pub async fn set_can_go_next(&self, can_go_next: bool) -> Result<()> {
        self.tx
            .send(Command::SetCanGoNext(can_go_next))
            .map_err(|_| anyhow!("mpris command channel closed"))
    }

    pub async fn set_can_go_previous(&self, can_go_previous: bool) -> Result<()> {
        self.tx
            .send(Command::SetCanGoPrevious(can_go_previous))
            .map_err(|_| anyhow!("mpris command channel closed"))
    }
}

fn run_mpris(
    mut rx: mpsc::UnboundedReceiver<Command>,
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async move {
        let player = Player::builder("vleer")
            .identity("Vleer")
            .desktop_entry("vleer")
            .can_play(true)
            .can_pause(true)
            .can_go_next(true)
            .can_go_previous(true)
            .can_seek(true)
            .build()
            .await?;

        let position_ms = Rc::new(Cell::new(0i64));

        {
            let tx = playback_tx.clone();
            player.connect_play_pause(move |_player| {
                let _ = tx.send(PlaybackCommand::PlayPause);
            });
        }

        {
            let tx = playback_tx.clone();
            player.connect_play(move |_player| {
                let _ = tx.send(PlaybackCommand::PlayPause);
            });
        }

        {
            let tx = playback_tx.clone();
            player.connect_pause(move |_player| {
                let _ = tx.send(PlaybackCommand::PlayPause);
            });
        }

        {
            let tx = playback_tx.clone();
            player.connect_stop(move |_player| {
                let _ = tx.send(PlaybackCommand::PlayPause);
            });
        }

        {
            let tx = playback_tx.clone();
            player.connect_next(move |_player| {
                let _ = tx.send(PlaybackCommand::Next);
            });
        }

        {
            let tx = playback_tx.clone();
            player.connect_previous(move |_player| {
                let _ = tx.send(PlaybackCommand::Previous);
            });
        }

        {
            let tx = playback_tx.clone();
            let position_ms = position_ms.clone();
            player.connect_seek(move |_player, offset| {
                let next_ms = (position_ms.get() + offset.as_millis()).max(0);
                position_ms.set(next_ms);
                let _ = tx.send(PlaybackCommand::Seek(next_ms as f32 / 1000.0));
            });
        }

        {
            let tx = playback_tx.clone();
            let position_ms = position_ms.clone();
            player.connect_set_position(move |_player, _track_id, position| {
                let next_ms = position.as_millis().max(0);
                position_ms.set(next_ms);
                let _ = tx.send(PlaybackCommand::Seek(next_ms as f32 / 1000.0));
            });
        }

        tokio::task::spawn_local(player.run());

        let mut artwork_cache = ArtworkCache::default();

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::UpdateMetadata(metadata) => {
                    if let Some(position_ms_value) = metadata.position_ms {
                        let pos = position_ms_value as i64;
                        position_ms.set(pos);
                        player.set_position(Time::from_millis(pos));
                    }

                    let track_id = metadata
                        .track_id
                        .as_deref()
                        .and_then(|id| TrackId::try_from(id).ok())
                        .unwrap_or(TrackId::NO_TRACK);

                    let art_url = artwork_cache
                        .resolve(metadata.artwork_id.as_deref(), metadata.artwork_data.as_deref())?;

                    let mut builder = Metadata::builder().trackid(track_id);

                    if let Some(title) = metadata.title {
                        builder = builder.title(title);
                    }

                    if let Some(artist) = metadata.artist {
                        builder = builder.artist([artist]);
                    }

                    if let Some(album) = metadata.album {
                        builder = builder.album(album);
                    }

                    if let Some(duration_ms) = metadata.duration_ms {
                        builder = builder.length(Time::from_millis(duration_ms as i64));
                    }

                    if let Some(art_url) = art_url {
                        builder = builder.art_url(art_url);
                    }

                    player.set_metadata(builder.build()).await?;
                }
                Command::SetState(state) => {
                    let status = match state {
                        PlaybackState::Playing => PlaybackStatus::Playing,
                        PlaybackState::Paused => PlaybackStatus::Paused,
                        PlaybackState::Stopped => PlaybackStatus::Stopped,
                    };
                    player.set_playback_status(status).await?;
                }
                Command::SetPosition(position_ms_value) => {
                    let pos = position_ms_value as i64;
                    position_ms.set(pos);
                    player.set_position(Time::from_millis(pos));
                }
                Command::SetCanGoNext(can_go_next) => {
                    player.set_can_go_next(can_go_next).await?;
                }
                Command::SetCanGoPrevious(can_go_previous) => {
                    player.set_can_go_previous(can_go_previous).await?;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[derive(Default)]
struct ArtworkCache {
    id: Option<String>,
    uri: Option<String>,
}

impl ArtworkCache {
    fn resolve(&mut self, id: Option<&str>, data: Option<&[u8]>) -> Result<Option<String>> {
        match (id, data) {
            (Some(id), Some(data)) => {
                if self.id.as_deref() == Some(id) {
                    if let Some(uri) = self.uri.clone() {
                        return Ok(Some(uri));
                    }
                }

                let uri = write_artwork_to_cache(id, data)?;
                self.id = Some(id.to_string());
                self.uri = Some(uri.clone());
                Ok(Some(uri))
            }
            _ => {
                self.id = None;
                self.uri = None;
                Ok(None)
            }
        }
    }
}

fn write_artwork_to_cache(id: &str, data: &[u8]) -> Result<String> {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("vleer")
        .join("mpris");

    fs::create_dir_all(&cache_dir)?;

    let ext = match image::guess_format(data) {
        Ok(ImageFormat::Png) => "png",
        Ok(ImageFormat::Jpeg) => "jpg",
        Ok(ImageFormat::Gif) => "gif",
        Ok(ImageFormat::Bmp) => "bmp",
        Ok(ImageFormat::Tiff) => "tiff",
        Ok(ImageFormat::WebP) => "webp",
        _ => "bin",
    };

    let safe_id: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();

    let file_path = cache_dir.join(format!("artwork-{}.{}", safe_id, ext));
    fs::write(&file_path, data)?;

    let uri = Url::from_file_path(&file_path)
        .map_err(|_| anyhow!("failed to convert artwork path to uri"))?;

    Ok(uri.to_string())
}
