use super::{PlaybackState, ResolvedMetadata};
use crate::media::playback::PlaybackCommand;
use anyhow::{anyhow, Result};
use image::ImageFormat;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use url::Url;
use windows::core::HSTRING;
use windows::Foundation::{EventRegistrationToken, TimeSpan, TypedEventHandler, Uri};
use windows::Media::*;
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

pub struct WindowsController {
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
    state: Arc<Mutex<ControllerState>>,
}

struct ControllerState {
    smtc: Option<SmtcState>,
    pending: PendingState,
}

struct PendingState {
    metadata: Option<ResolvedMetadata>,
    playback: Option<PlaybackState>,
    position_ms: Option<u64>,
    can_next: Option<bool>,
    can_prev: Option<bool>,
}

struct SmtcState {
    controls: SystemMediaTransportControls,
    display_updater: SystemMediaTransportControlsDisplayUpdater,
    timeline_properties: SystemMediaTransportControlsTimelineProperties,
    button_handler_token: Option<EventRegistrationToken>,
    position_handler_token: Option<EventRegistrationToken>,
    artwork_cache: ArtworkCache,
}

#[derive(Default)]
struct ArtworkCache {
    id: Option<String>,
    uri: Option<String>,
}

impl WindowsController {
    pub fn new(playback_tx: mpsc::UnboundedSender<PlaybackCommand>) -> Self {
        Self {
            playback_tx,
            state: Arc::new(Mutex::new(ControllerState {
                smtc: None,
                pending: PendingState {
                    metadata: None,
                    playback: None,
                    position_ms: None,
                    can_next: None,
                    can_prev: None,
                },
            })),
        }
    }

    pub fn set_window_handle(&self, hwnd: isize) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("windows controller lock poisoned"))?;

        if state.smtc.is_some() {
            return Ok(());
        }

        let mut smtc = init_smtc(hwnd, self.playback_tx.clone())?;

        if let Some(playback) = state.pending.playback.take() {
            apply_playback(&mut smtc, playback, state.pending.position_ms)?;
        }
        if let Some(metadata) = state.pending.metadata.take() {
            apply_metadata(&mut smtc, metadata)?;
        }
        if let Some(position_ms) = state.pending.position_ms.take() {
            apply_position(&mut smtc, position_ms)?;
        }
        if let Some(can_next) = state.pending.can_next.take() {
            smtc.controls.SetIsNextEnabled(can_next)?;
        }
        if let Some(can_prev) = state.pending.can_prev.take() {
            smtc.controls.SetIsPreviousEnabled(can_prev)?;
        }

        state.smtc = Some(smtc);
        Ok(())
    }

    pub async fn update_metadata(&self, metadata: ResolvedMetadata) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("windows controller lock poisoned"))?;
        if let Some(smtc) = state.smtc.as_mut() {
            apply_metadata(smtc, metadata)
        } else {
            state.pending.metadata = Some(metadata);
            Ok(())
        }
    }

    pub async fn set_state(&self, state_value: PlaybackState) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("windows controller lock poisoned"))?;
        if let Some(smtc) = state.smtc.as_mut() {
            apply_playback(smtc, state_value, None)
        } else {
            state.pending.playback = Some(state_value);
            Ok(())
        }
    }

    pub async fn set_position(&self, position_ms: u64) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("windows controller lock poisoned"))?;
        if let Some(smtc) = state.smtc.as_mut() {
            apply_position(smtc, position_ms)
        } else {
            state.pending.position_ms = Some(position_ms);
            Ok(())
        }
    }

    pub async fn set_can_go_next(&self, can_go_next: bool) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("windows controller lock poisoned"))?;
        if let Some(smtc) = state.smtc.as_mut() {
            smtc.controls.SetIsNextEnabled(can_go_next)?;
            Ok(())
        } else {
            state.pending.can_next = Some(can_go_next);
            Ok(())
        }
    }

    pub async fn set_can_go_previous(&self, can_go_previous: bool) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("windows controller lock poisoned"))?;
        if let Some(smtc) = state.smtc.as_mut() {
            smtc.controls.SetIsPreviousEnabled(can_go_previous)?;
            Ok(())
        } else {
            state.pending.can_prev = Some(can_go_previous);
            Ok(())
        }
    }
}

fn init_smtc(
    hwnd: isize,
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
) -> Result<SmtcState> {
    let interop: ISystemMediaTransportControlsInterop =
        windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd)) }?;

    controls.SetIsEnabled(true)?;
    controls.SetIsPlayEnabled(true)?;
    controls.SetIsPauseEnabled(true)?;
    controls.SetIsStopEnabled(true)?;
    controls.SetIsNextEnabled(true)?;
    controls.SetIsPreviousEnabled(true)?;
    controls.SetIsFastForwardEnabled(true)?;
    controls.SetIsRewindEnabled(true)?;

    let display_updater = controls.DisplayUpdater()?;
    display_updater.SetType(MediaPlaybackType::Music)?;

    let timeline_properties = SystemMediaTransportControlsTimelineProperties::new()?;

    let button_handler_token = attach_button_handler(&controls, playback_tx.clone())?;
    let position_handler_token = attach_position_handler(&controls, playback_tx)?;

    Ok(SmtcState {
        controls,
        display_updater,
        timeline_properties,
        button_handler_token: Some(button_handler_token),
        position_handler_token: Some(position_handler_token),
        artwork_cache: ArtworkCache::default(),
    })
}

fn attach_button_handler(
    controls: &SystemMediaTransportControls,
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
) -> Result<EventRegistrationToken> {
    let handler = TypedEventHandler::new(move |_, args: &Option<_>| {
        let args: &SystemMediaTransportControlsButtonPressedEventArgs = args.as_ref().unwrap();
        let button = args.Button()?;

        let cmd = match button {
            SystemMediaTransportControlsButton::Play
            | SystemMediaTransportControlsButton::Pause
            | SystemMediaTransportControlsButton::Stop => PlaybackCommand::PlayPause,
            SystemMediaTransportControlsButton::Next => PlaybackCommand::Next,
            SystemMediaTransportControlsButton::Previous => PlaybackCommand::Previous,
            _ => return Ok(()),
        };

        let _ = playback_tx.send(cmd);
        Ok(())
    });

    Ok(controls.ButtonPressed(&handler)?)
}

fn attach_position_handler(
    controls: &SystemMediaTransportControls,
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
) -> Result<EventRegistrationToken> {
    let handler = TypedEventHandler::new(move |_, args: &Option<_>| {
        let args: &PlaybackPositionChangeRequestedEventArgs = args.as_ref().unwrap();
        let position = args.RequestedPlaybackPosition()?;
        let duration = std::time::Duration::from(position);
        let _ = playback_tx.send(PlaybackCommand::Seek(duration.as_secs_f32()));
        Ok(())
    });

    Ok(controls.PlaybackPositionChangeRequested(&handler)?)
}

fn apply_playback(
    smtc: &mut SmtcState,
    playback: PlaybackState,
    position_ms: Option<u64>,
) -> Result<()> {
    let status = match playback {
        PlaybackState::Playing => MediaPlaybackStatus::Playing,
        PlaybackState::Paused => MediaPlaybackStatus::Paused,
        PlaybackState::Stopped => MediaPlaybackStatus::Stopped,
    };
    smtc.controls.SetPlaybackStatus(status)?;

    if let Some(position_ms) = position_ms {
        smtc.timeline_properties
            .SetPosition(TimeSpan::from(std::time::Duration::from_millis(position_ms)))?;
        smtc.controls
            .UpdateTimelineProperties(&smtc.timeline_properties)?;
    }

    Ok(())
}

fn apply_position(smtc: &mut SmtcState, position_ms: u64) -> Result<()> {
    smtc.timeline_properties
        .SetPosition(TimeSpan::from(std::time::Duration::from_millis(position_ms)))?;
    smtc.controls
        .UpdateTimelineProperties(&smtc.timeline_properties)?;
    Ok(())
}

fn apply_metadata(smtc: &mut SmtcState, metadata: ResolvedMetadata) -> Result<()> {
    let properties = smtc.display_updater.MusicProperties()?;

    if let Some(title) = metadata.title {
        properties.SetTitle(&HSTRING::from(title))?;
    }
    if let Some(artist) = metadata.artist {
        properties.SetArtist(&HSTRING::from(artist))?;
    }
    if let Some(album) = metadata.album {
        properties.SetAlbumTitle(&HSTRING::from(album))?;
    }

    if let Some(duration_ms) = metadata.duration_ms {
        let duration = std::time::Duration::from_millis(duration_ms);
        smtc.timeline_properties.SetStartTime(TimeSpan::default())?;
        smtc.timeline_properties.SetMinSeekTime(TimeSpan::default())?;
        smtc.timeline_properties
            .SetEndTime(TimeSpan::from(duration))?;
        smtc.timeline_properties
            .SetMaxSeekTime(TimeSpan::from(duration))?;
    }

    if let Some(artwork) = metadata
        .artwork_id
        .as_deref()
        .zip(metadata.artwork_data.as_deref())
    {
        if let Some(uri) = smtc.artwork_cache.resolve(artwork.0, artwork.1)? {
            let stream = RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&HSTRING::from(uri))?)?;
            smtc.display_updater.SetThumbnail(&stream)?;
        }
    }

    smtc.controls
        .UpdateTimelineProperties(&smtc.timeline_properties)?;
    smtc.display_updater.Update()?;
    Ok(())
}

impl ArtworkCache {
    fn resolve(&mut self, id: &str, data: &[u8]) -> Result<Option<String>> {
        if self.id.as_deref() == Some(id) {
            return Ok(self.uri.clone());
        }

        let uri = write_artwork_to_cache(id, data)?;
        self.id = Some(id.to_string());
        self.uri = Some(uri.clone());
        Ok(Some(uri))
    }
}

fn write_artwork_to_cache(id: &str, data: &[u8]) -> Result<String> {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("vleer")
        .join("smtc");

    std::fs::create_dir_all(&cache_dir)?;

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
    std::fs::write(&file_path, data)?;

    let uri = Url::from_file_path(&file_path)
        .map_err(|_| anyhow!("failed to convert artwork path to uri"))?;

    Ok(uri.to_string())
}
