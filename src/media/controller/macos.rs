use super::{PlaybackState, ResolvedMetadata};
use crate::media::playback::PlaybackCommand;
use anyhow::{Result, anyhow};
use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2::{AnyThread, class, msg_send};
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSNumber, NSString};
use std::sync::Mutex;
use std::thread;
use tokio::sync::mpsc;
use tracing::error;

const MP_NOW_PLAYING_STATE_PLAYING: isize = 1;
const MP_NOW_PLAYING_STATE_PAUSED: isize = 2;
const MP_NOW_PLAYING_STATE_STOPPED: isize = 3;

const MP_REMOTE_COMMAND_SUCCESS: isize = 0;

#[link(name = "MediaPlayer", kind = "framework")]
unsafe extern "C" {}

#[allow(non_upper_case_globals)]
unsafe extern "C" {
    static MPMediaItemPropertyTitle: *mut AnyObject;
    static MPMediaItemPropertyArtist: *mut AnyObject;
    static MPMediaItemPropertyAlbumTitle: *mut AnyObject;
    static MPMediaItemPropertyArtwork: *mut AnyObject;
    static MPMediaItemPropertyPlaybackDuration: *mut AnyObject;
    static MPNowPlayingInfoPropertyElapsedPlaybackTime: *mut AnyObject;
}

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
    command_center: *mut AnyObject,
    now_playing_center: *mut AnyObject,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    handlers: Vec<RcBlock<dyn Fn(*mut AnyObject) -> isize>>,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    position_handler: RcBlock<dyn Fn(*mut AnyObject) -> isize>,
    artwork: Mutex<ArtworkState>,
}

struct ArtworkState {
    image: Option<Retained<NSImage>>,
    handler: Option<RcBlock<dyn Fn(CGSize) -> *mut AnyObject>>,
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

    pub async fn update_metadata(&self, metadata: ResolvedMetadata) -> Result<()> {
        self.tx
            .send(Command::UpdateMetadata(metadata))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub async fn set_state(&self, state: PlaybackState) -> Result<()> {
        self.tx
            .send(Command::SetState(state))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub async fn set_position(&self, position_ms: u64) -> Result<()> {
        self.tx
            .send(Command::SetPosition(position_ms))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub async fn set_can_go_next(&self, can_go_next: bool) -> Result<()> {
        self.tx
            .send(Command::SetCanGoNext(can_go_next))
            .map_err(|_| anyhow!("macos command channel closed"))
    }

    pub async fn set_can_go_previous(&self, can_go_previous: bool) -> Result<()> {
        self.tx
            .send(Command::SetCanGoPrevious(can_go_previous))
            .map_err(|_| anyhow!("macos command channel closed"))
    }
}

fn run_macos(
    mut rx: mpsc::UnboundedReceiver<Command>,
    playback_tx: mpsc::UnboundedSender<PlaybackCommand>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        let mut state = MacosState::new(playback_tx);

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::UpdateMetadata(metadata) => {
                    state.update_metadata(metadata)?;
                }
                Command::SetState(state_value) => {
                    state.set_state(state_value)?;
                }
                Command::SetPosition(position_ms) => {
                    state.set_position(position_ms)?;
                }
                Command::SetCanGoNext(can_go_next) => {
                    state.set_can_go_next(can_go_next)?;
                }
                Command::SetCanGoPrevious(can_go_previous) => {
                    state.set_can_go_previous(can_go_previous)?;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

impl MacosState {
    fn new(playback_tx: mpsc::UnboundedSender<PlaybackCommand>) -> Self {
        let command_center: *mut AnyObject =
            unsafe { msg_send![class!(MPRemoteCommandCenter), sharedCommandCenter] };
        let now_playing_center: *mut AnyObject =
            unsafe { msg_send![class!(MPNowPlayingInfoCenter), defaultCenter] };

        let mut handlers: Vec<RcBlock<dyn Fn(*mut AnyObject) -> isize>> = Vec::new();

        unsafe {
            let cmd: *mut AnyObject = msg_send![command_center, togglePlayPauseCommand];
            let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::PlayPause);
            let _: () = msg_send![cmd, setEnabled: true];
            let _: () = msg_send![cmd, addTargetWithHandler: &*handler];
            handlers.push(handler);

            let cmd: *mut AnyObject = msg_send![command_center, playCommand];
            let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::PlayPause);
            let _: () = msg_send![cmd, setEnabled: true];
            let _: () = msg_send![cmd, addTargetWithHandler: &*handler];
            handlers.push(handler);

            let cmd: *mut AnyObject = msg_send![command_center, pauseCommand];
            let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::PlayPause);
            let _: () = msg_send![cmd, setEnabled: true];
            let _: () = msg_send![cmd, addTargetWithHandler: &*handler];
            handlers.push(handler);

            let cmd: *mut AnyObject = msg_send![command_center, nextTrackCommand];
            let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::Next);
            let _: () = msg_send![cmd, setEnabled: true];
            let _: () = msg_send![cmd, addTargetWithHandler: &*handler];
            handlers.push(handler);

            let cmd: *mut AnyObject = msg_send![command_center, previousTrackCommand];
            let handler = make_command_handler(playback_tx.clone(), PlaybackCommand::Previous);
            let _: () = msg_send![cmd, setEnabled: true];
            let _: () = msg_send![cmd, addTargetWithHandler: &*handler];
            handlers.push(handler);
        }

        let position_handler = {
            let tx = playback_tx.clone();
            RcBlock::new(move |event: *mut AnyObject| -> isize {
                let position: f64 = unsafe { msg_send![event, positionTime] };
                let _ = tx.send(PlaybackCommand::Seek(position as f32));
                MP_REMOTE_COMMAND_SUCCESS
            })
        };

        unsafe {
            let cmd: *mut AnyObject = msg_send![command_center, changePlaybackPositionCommand];
            let _: () = msg_send![cmd, setEnabled: true];
            let _: () = msg_send![cmd, addTargetWithHandler: &*position_handler];
        }

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
            let now_playing: *mut AnyObject =
                unsafe { msg_send![class!(NSMutableDictionary), dictionary] };

            if let Some(title) = metadata.title {
                let ns_title = NSString::from_str(&title);
                unsafe {
                    let _: () = msg_send![
                        now_playing,
                        setObject: Retained::as_ptr(&ns_title).cast_mut(),
                        forKey: MPMediaItemPropertyTitle
                    ];
                }
            }

            if let Some(artist) = metadata.artist {
                let ns_artist = NSString::from_str(&artist);
                unsafe {
                    let _: () = msg_send![
                        now_playing,
                        setObject: Retained::as_ptr(&ns_artist).cast_mut(),
                        forKey: MPMediaItemPropertyArtist
                    ];
                }
            }

            if let Some(album) = metadata.album {
                let ns_album = NSString::from_str(&album);
                unsafe {
                    let _: () = msg_send![
                        now_playing,
                        setObject: Retained::as_ptr(&ns_album).cast_mut(),
                        forKey: MPMediaItemPropertyAlbumTitle
                    ];
                }
            }

            if let Some(duration_ms) = metadata.duration_ms {
                let duration_secs = duration_ms as f64 / 1000.0;
                let num = NSNumber::new_f64(duration_secs);
                unsafe {
                    let _: () = msg_send![
                        now_playing,
                        setObject: Retained::as_ptr(&num).cast_mut(),
                        forKey: MPMediaItemPropertyPlaybackDuration
                    ];
                }
            }

            if let Some(position_ms) = metadata.position_ms {
                let position_secs = position_ms as f64 / 1000.0;
                let num = NSNumber::new_f64(position_secs);
                unsafe {
                    let _: () = msg_send![
                        now_playing,
                        setObject: Retained::as_ptr(&num).cast_mut(),
                        forKey: MPNowPlayingInfoPropertyElapsedPlaybackTime
                    ];
                }
            }

            if let Some(artwork) = self.build_artwork(metadata.artwork_data.as_deref())? {
                unsafe {
                    let _: () = msg_send![
                        now_playing,
                        setObject: artwork,
                        forKey: MPMediaItemPropertyArtwork
                    ];
                }
            }

            unsafe {
                let _: () = msg_send![self.now_playing_center, setNowPlayingInfo: now_playing];
            }

            Ok(())
        })
    }

    fn set_state(&mut self, state: PlaybackState) -> Result<()> {
        autoreleasepool(|_| {
            let playback_state = match state {
                PlaybackState::Playing => MP_NOW_PLAYING_STATE_PLAYING,
                PlaybackState::Paused => MP_NOW_PLAYING_STATE_PAUSED,
                PlaybackState::Stopped => MP_NOW_PLAYING_STATE_STOPPED,
            };
            unsafe {
                let _: () = msg_send![self.now_playing_center, setPlaybackState: playback_state];
            }
            Ok(())
        })
    }

    fn set_position(&mut self, position_ms: u64) -> Result<()> {
        autoreleasepool(|_| {
            let now_playing: *mut AnyObject =
                unsafe { msg_send![class!(NSMutableDictionary), dictionary] };
            let previous: *mut AnyObject =
                unsafe { msg_send![self.now_playing_center, nowPlayingInfo] };
            if !previous.is_null() {
                unsafe {
                    let _: () = msg_send![now_playing, addEntriesFromDictionary: previous];
                }
            }

            let position_secs = position_ms as f64 / 1000.0;
            let num = NSNumber::new_f64(position_secs);
            unsafe {
                let _: () = msg_send![
                    now_playing,
                    setObject: Retained::as_ptr(&num).cast_mut(),
                    forKey: MPNowPlayingInfoPropertyElapsedPlaybackTime
                ];
                let _: () = msg_send![self.now_playing_center, setNowPlayingInfo: now_playing];
            }

            Ok(())
        })
    }

    fn set_can_go_next(&mut self, can_go_next: bool) -> Result<()> {
        autoreleasepool(|_| {
            unsafe {
                let cmd: *mut AnyObject = msg_send![self.command_center, nextTrackCommand];
                let _: () = msg_send![cmd, setEnabled: can_go_next];
            }
            Ok(())
        })
    }

    fn set_can_go_previous(&mut self, can_go_previous: bool) -> Result<()> {
        autoreleasepool(|_| {
            unsafe {
                let cmd: *mut AnyObject = msg_send![self.command_center, previousTrackCommand];
                let _: () = msg_send![cmd, setEnabled: can_go_previous];
            }
            Ok(())
        })
    }

    fn build_artwork(&self, data: Option<&[u8]>) -> Result<Option<*mut AnyObject>> {
        let data = match data {
            Some(data) if !data.is_empty() => data,
            _ => return Ok(None),
        };

        let ns_data = unsafe { NSData::dataWithBytes_length(data.as_ptr().cast(), data.len()) };
        let image = match NSImage::initWithData(NSImage::alloc(), &ns_data) {
            Some(image) => image,
            None => return Ok(None),
        };
        let size = image.size();
        let image_ptr = Retained::as_ptr(&image).cast_mut();

        let handler = RcBlock::new(move |_size: CGSize| -> *mut AnyObject { image_ptr.cast() });

        let artwork: *mut AnyObject = unsafe { msg_send![class!(MPMediaItemArtwork), alloc] };
        let artwork: *mut AnyObject =
            unsafe { msg_send![artwork, initWithBoundsSize: size, requestHandler: &*handler] };

        if artwork.is_null() {
            error!("failed to create MPMediaItemArtwork");
            return Ok(None);
        }

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
) -> RcBlock<dyn Fn(*mut AnyObject) -> isize> {
    RcBlock::new(move |_event: *mut AnyObject| -> isize {
        let _ = tx.send(command.clone());
        MP_REMOTE_COMMAND_SUCCESS
    })
}
