use super::{PlaybackState, ResolvedMetadata};
use crate::media::playback::PlaybackCommand;
use anyhow::{Result, anyhow};
use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSDictionary, NSMutableDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommand, MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork,
    MPMediaItemPropertyAlbumTitle, MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork,
    MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle, MPNowPlayingInfoCenter,
    MPNowPlayingInfoPropertyElapsedPlaybackTime, MPNowPlayingPlaybackState, MPRemoteCommand,
    MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::thread;
use tokio::sync::mpsc;
use tracing::error;

type CommandHandler =
    RcBlock<dyn Fn(NonNull<MPRemoteCommandEvent>) -> MPRemoteCommandHandlerStatus>;

pub struct MacosController {
    tx: mpsc::UnboundedSender<Command>,
}

enum Command {
    UpdateMetadata(ResolvedMetadata),
    SetState(PlaybackState),
    SetPosition(u64),
    SetCanGoNext(bool),
    SetCanGoPrevious(bool),
}

struct MacosState {
    command_center: Retained<MPRemoteCommandCenter>,
    now_playing_center: Retained<MPNowPlayingInfoCenter>,
    #[allow(dead_code)]
    handlers: Vec<CommandHandler>,
    #[allow(dead_code)]
    position_handler: CommandHandler,
    artwork: Mutex<ArtworkState>,
}

struct ArtworkState {
    image: Option<Retained<NSImage>>,
    handler: Option<RcBlock<dyn Fn(CGSize) -> NonNull<NSImage>>>,
}

impl MacosController {
    pub fn new(playback_tx: mpsc::UnboundedSender<PlaybackCommand>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        thread::spawn(move || {
            if let Err(err) = run_macos(rx, playback_tx) {
                error!(?err, "macos media controller stopped");
            }
        });

        Self { tx }
    }

    pub fn update_metadata(&self, metadata: ResolvedMetadata) -> Result<()> {
        self.tx
            .send(Command::UpdateMetadata(metadata))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub fn set_state(&self, state: PlaybackState) -> Result<()> {
        self.tx
            .send(Command::SetState(state))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub fn set_position(&self, position_ms: u64) -> Result<()> {
        self.tx
            .send(Command::SetPosition(position_ms))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub fn set_can_go_next(&self, can_go_next: bool) -> Result<()> {
        self.tx
            .send(Command::SetCanGoNext(can_go_next))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub fn set_can_go_previous(&self, can_go_previous: bool) -> Result<()> {
        self.tx
            .send(Command::SetCanGoPrevious(can_go_previous))
            .map_err(|_| anyhow!("macos command channel closed"))
    }
}

fn run_macos(
    mut rx: mpsc::UnboundedReceiver<Command>,
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
) -> Result<()> {
    let mut state = MacosState::new(playback_tx);

    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            Command::UpdateMetadata(metadata) => state.update_metadata(metadata)?,
            Command::SetState(state_value) => state.set_state(state_value)?,
            Command::SetPosition(position_ms) => state.set_position(position_ms)?,
            Command::SetCanGoNext(can_go_next) => state.set_can_go_next(can_go_next)?,
            Command::SetCanGoPrevious(can_go_previous) => {
                state.set_can_go_previous(can_go_previous)?
            }
        }
    }

    Ok(())
}

impl MacosState {
    fn new(playback_tx: mpsc::UnboundedSender<PlaybackCommand>) -> Self {
        let command_center = shared_command_center();
        let now_playing_center = default_now_playing_center();

        let mut handlers = Vec::new();

        let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::PlayPause);
        command_center.toggle_play_pause_cmd().register(&handler);
        handlers.push(handler);

        let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::Play);
        command_center.play_cmd().register(&handler);
        handlers.push(handler);

        let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::Pause);
        command_center.pause_cmd().register(&handler);
        handlers.push(handler);

        let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::Next);
        command_center.next_track_cmd().register(&handler);
        handlers.push(handler);

        let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::Previous);
        command_center.previous_track_cmd().register(&handler);
        handlers.push(handler);

        let tx = playback_tx;
        let position_handler = RcBlock::new(
            move |event: NonNull<MPRemoteCommandEvent>| -> MPRemoteCommandHandlerStatus {
                let _ = tx.send(PlaybackCommand::Seek(position_time(event) as f32));
                MPRemoteCommandHandlerStatus::Success
            },
        );
        command_center
            .change_position_cmd()
            .register(&position_handler);

        Self {
            command_center,
            now_playing_center,
            handlers,
            position_handler,
            artwork: Mutex::new(ArtworkState {
                image: None,
                handler: None,
            }),
        }
    }

    fn update_metadata(&mut self, metadata: ResolvedMetadata) -> Result<()> {
        autoreleasepool(|_| {
            let dict = NSMutableDictionary::<NSString, AnyObject>::new();

            if let Some(title) = metadata.title {
                dict_insert(&dict, key_title(), &NSString::from_str(&title));
            }
            if let Some(artist) = metadata.artist {
                dict_insert(&dict, key_artist(), &NSString::from_str(&artist));
            }
            if let Some(album) = metadata.album {
                dict_insert(&dict, key_album(), &NSString::from_str(&album));
            }
            if let Some(duration_ms) = metadata.duration_ms {
                dict_insert(
                    &dict,
                    key_duration(),
                    &NSNumber::new_f64(duration_ms as f64 / 1000.0),
                );
            }
            if let Some(position_ms) = metadata.position_ms {
                dict_insert(
                    &dict,
                    key_elapsed_time(),
                    &NSNumber::new_f64(position_ms as f64 / 1000.0),
                );
            }
            if let Some(artwork) = self.build_artwork(metadata.artwork_data.as_deref())? {
                dict_insert(&dict, key_artwork(), &artwork);
            }

            self.now_playing_center.apply_dict(&dict);
            Ok(())
        })
    }

    fn set_state(&mut self, state: PlaybackState) -> Result<()> {
        autoreleasepool(|_| {
            let playback_state = match state {
                PlaybackState::Playing => MPNowPlayingPlaybackState::Playing,
                PlaybackState::Paused => MPNowPlayingPlaybackState::Paused,
                PlaybackState::Stopped => MPNowPlayingPlaybackState::Stopped,
            };
            self.now_playing_center.apply_state(playback_state);
            Ok(())
        })
    }

    fn set_position(&mut self, position_ms: u64) -> Result<()> {
        autoreleasepool(|_| {
            let dict = NSMutableDictionary::<NSString, AnyObject>::new();
            if let Some(previous) = self.now_playing_center.current_dict() {
                dict.addEntriesFromDictionary(&*previous);
            }
            dict_insert(
                &dict,
                key_elapsed_time(),
                &NSNumber::new_f64(position_ms as f64 / 1000.0),
            );
            self.now_playing_center.apply_dict(&dict);
            Ok(())
        })
    }

    fn set_can_go_next(&mut self, can_go_next: bool) -> Result<()> {
        autoreleasepool(|_| {
            self.command_center
                .next_track_cmd()
                .set_enabled(can_go_next);
            Ok(())
        })
    }

    fn set_can_go_previous(&mut self, can_go_previous: bool) -> Result<()> {
        autoreleasepool(|_| {
            self.command_center
                .previous_track_cmd()
                .set_enabled(can_go_previous);
            Ok(())
        })
    }

    fn build_artwork(&self, data: Option<&[u8]>) -> Result<Option<Retained<MPMediaItemArtwork>>> {
        let data = match data {
            Some(data) if !data.is_empty() => data,
            _ => return Ok(None),
        };

        let ns_data = nsdata_from_bytes(data);
        let image = match NSImage::initWithData(NSImage::alloc(), &ns_data) {
            Some(image) => image,
            None => return Ok(None),
        };
        let size = image.size();
        let image_ptr = Retained::as_ptr(&image).cast_mut();
        let (artwork, handler) = create_artwork(image_ptr, size);

        if let Ok(mut state) = self.artwork.lock() {
            state.image = Some(image);
            state.handler = Some(handler);
        }

        Ok(Some(artwork))
    }
}

fn make_command_handler(
    tx: mpsc::UnboundedSender<PlaybackCommand>,
    command: PlaybackCommand,
) -> CommandHandler {
    RcBlock::new(
        move |_event: NonNull<MPRemoteCommandEvent>| -> MPRemoteCommandHandlerStatus {
            let _ = tx.send(command.clone());
            MPRemoteCommandHandlerStatus::Success
        },
    )
}

fn shared_command_center() -> Retained<MPRemoteCommandCenter> {
    unsafe { MPRemoteCommandCenter::sharedCommandCenter() }
}

fn default_now_playing_center() -> Retained<MPNowPlayingInfoCenter> {
    unsafe { MPNowPlayingInfoCenter::defaultCenter() }
}

fn position_time(event: NonNull<MPRemoteCommandEvent>) -> f64 {
    unsafe {
        event
            .cast::<MPChangePlaybackPositionCommandEvent>()
            .as_ref()
            .positionTime()
    }
}

fn nsdata_from_bytes(data: &[u8]) -> Retained<NSData> {
    unsafe { NSData::dataWithBytes_length(data.as_ptr().cast(), data.len()) }
}

type ArtworkHandler = RcBlock<dyn Fn(CGSize) -> NonNull<NSImage>>;

fn create_artwork(
    image_ptr: *mut NSImage,
    size: CGSize,
) -> (Retained<MPMediaItemArtwork>, ArtworkHandler) {
    let handler: ArtworkHandler = RcBlock::new(move |_: CGSize| {
        NonNull::new(image_ptr).expect("image_ptr from Retained is always non-null")
    });
    let artwork = unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            size,
            &handler,
        )
    };
    (artwork, handler)
}

fn dict_insert(dict: &NSMutableDictionary<NSString, AnyObject>, key: &NSString, value: &AnyObject) {
    unsafe { dict.setObject_forKey(value, ProtocolObject::from_ref(key)) }
}

fn key_title() -> &'static NSString {
    unsafe { MPMediaItemPropertyTitle }
}
fn key_artist() -> &'static NSString {
    unsafe { MPMediaItemPropertyArtist }
}
fn key_album() -> &'static NSString {
    unsafe { MPMediaItemPropertyAlbumTitle }
}
fn key_duration() -> &'static NSString {
    unsafe { MPMediaItemPropertyPlaybackDuration }
}
fn key_artwork() -> &'static NSString {
    unsafe { MPMediaItemPropertyArtwork }
}
fn key_elapsed_time() -> &'static NSString {
    unsafe { MPNowPlayingInfoPropertyElapsedPlaybackTime }
}

trait CommandCenterExt {
    fn toggle_play_pause_cmd(&self) -> Retained<MPRemoteCommand>;
    fn play_cmd(&self) -> Retained<MPRemoteCommand>;
    fn pause_cmd(&self) -> Retained<MPRemoteCommand>;
    fn next_track_cmd(&self) -> Retained<MPRemoteCommand>;
    fn previous_track_cmd(&self) -> Retained<MPRemoteCommand>;
    fn change_position_cmd(&self) -> Retained<MPChangePlaybackPositionCommand>;
}

impl CommandCenterExt for MPRemoteCommandCenter {
    fn toggle_play_pause_cmd(&self) -> Retained<MPRemoteCommand> {
        unsafe { self.togglePlayPauseCommand() }
    }
    fn play_cmd(&self) -> Retained<MPRemoteCommand> {
        unsafe { self.playCommand() }
    }
    fn pause_cmd(&self) -> Retained<MPRemoteCommand> {
        unsafe { self.pauseCommand() }
    }
    fn next_track_cmd(&self) -> Retained<MPRemoteCommand> {
        unsafe { self.nextTrackCommand() }
    }
    fn previous_track_cmd(&self) -> Retained<MPRemoteCommand> {
        unsafe { self.previousTrackCommand() }
    }
    fn change_position_cmd(&self) -> Retained<MPChangePlaybackPositionCommand> {
        unsafe { self.changePlaybackPositionCommand() }
    }
}

trait RemoteCommandExt {
    fn register(&self, handler: &CommandHandler);
    fn set_enabled(&self, enabled: bool);
}

impl RemoteCommandExt for MPRemoteCommand {
    fn register(&self, handler: &CommandHandler) {
        unsafe {
            self.setEnabled(true);
            let _ = self.addTargetWithHandler(handler);
        }
    }
    fn set_enabled(&self, enabled: bool) {
        unsafe { self.setEnabled(enabled) }
    }
}

trait NowPlayingCenterExt {
    fn apply_dict(&self, dict: &NSMutableDictionary<NSString, AnyObject>);
    fn current_dict(&self) -> Option<Retained<NSDictionary<NSString, AnyObject>>>;
    fn apply_state(&self, state: MPNowPlayingPlaybackState);
}

impl NowPlayingCenterExt for MPNowPlayingInfoCenter {
    fn apply_dict(&self, dict: &NSMutableDictionary<NSString, AnyObject>) {
        unsafe { self.setNowPlayingInfo(Some(&**dict)) }
    }
    fn current_dict(&self) -> Option<Retained<NSDictionary<NSString, AnyObject>>> {
        unsafe { self.nowPlayingInfo() }
    }
    fn apply_state(&self, state: MPNowPlayingPlaybackState) {
        unsafe { self.setPlaybackState(state) }
    }
}
