use anyhow::Result;
use gpui::{App, BorrowAppContext, Global};
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
use mpris_server::{
    LoopStatus, Metadata, PlaybackStatus, PlayerInterface, RootInterface, Server, Time, TrackId,
};

#[cfg(any(target_os = "windows", target_os = "macos"))]
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager,
    hotkey::{Code, HotKey},
};

pub struct MediaKeyHandler {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    _hotkey_manager: GlobalHotKeyManager,
}

impl Global for MediaKeyHandler {}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
enum PlayerAction {
    PlayPause,
    Next,
    Previous,
    Play,
    Pause,
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
struct VleerPlayer {
    action_tx: mpsc::UnboundedSender<PlayerAction>,
}

#[cfg(target_os = "linux")]
impl RootInterface for VleerPlayer {
    async fn identity(&self) -> Result<String, mpris_server::zbus::fdo::Error> {
        Ok("Vleer".to_string())
    }

    async fn desktop_entry(&self) -> Result<String, mpris_server::zbus::fdo::Error> {
        Ok("vleer".to_string())
    }

    async fn supported_uri_schemes(&self) -> Result<Vec<String>, mpris_server::zbus::fdo::Error> {
        Ok(vec!["file".to_string()])
    }

    async fn supported_mime_types(&self) -> Result<Vec<String>, mpris_server::zbus::fdo::Error> {
        Ok(vec!["audio/mpeg".to_string(), "audio/flac".to_string()])
    }

    async fn raise(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        Ok(())
    }

    async fn quit(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        Ok(())
    }

    async fn can_quit(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }

    async fn can_raise(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }

    async fn fullscreen(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> Result<(), mpris_server::zbus::Error> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }

    async fn has_track_list(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }
}

#[cfg(target_os = "linux")]
impl PlayerInterface for VleerPlayer {
    async fn next(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        let _ = self.action_tx.send(PlayerAction::Next);
        Ok(())
    }

    async fn previous(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        let _ = self.action_tx.send(PlayerAction::Previous);
        Ok(())
    }

    async fn pause(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        let _ = self.action_tx.send(PlayerAction::Pause);
        Ok(())
    }

    async fn play_pause(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        let _ = self.action_tx.send(PlayerAction::PlayPause);
        Ok(())
    }

    async fn stop(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        Ok(())
    }

    async fn play(&self) -> Result<(), mpris_server::zbus::fdo::Error> {
        let _ = self.action_tx.send(PlayerAction::Play);
        Ok(())
    }

    async fn seek(&self, _offset: Time) -> Result<(), mpris_server::zbus::fdo::Error> {
        Ok(())
    }

    async fn set_position(
        &self,
        _track_id: TrackId,
        _position: Time,
    ) -> Result<(), mpris_server::zbus::fdo::Error> {
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> Result<(), mpris_server::zbus::fdo::Error> {
        Ok(())
    }

    async fn playback_status(&self) -> Result<PlaybackStatus, mpris_server::zbus::fdo::Error> {
        Ok(PlaybackStatus::Playing)
    }

    async fn loop_status(&self) -> Result<LoopStatus, mpris_server::zbus::fdo::Error> {
        Ok(LoopStatus::None)
    }

    async fn set_loop_status(
        &self,
        _loop_status: LoopStatus,
    ) -> Result<(), mpris_server::zbus::Error> {
        Ok(())
    }

    async fn rate(&self) -> Result<f64, mpris_server::zbus::fdo::Error> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: f64) -> Result<(), mpris_server::zbus::Error> {
        Ok(())
    }

    async fn shuffle(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }

    async fn set_shuffle(&self, _shuffle: bool) -> Result<(), mpris_server::zbus::Error> {
        Ok(())
    }

    async fn metadata(&self) -> Result<Metadata, mpris_server::zbus::fdo::Error> {
        Ok(Metadata::new())
    }

    async fn volume(&self) -> Result<f64, mpris_server::zbus::fdo::Error> {
        Ok(1.0)
    }

    async fn set_volume(&self, _volume: f64) -> Result<(), mpris_server::zbus::Error> {
        Ok(())
    }

    async fn position(&self) -> Result<Time, mpris_server::zbus::fdo::Error> {
        Ok(Time::from_micros(0))
    }

    async fn minimum_rate(&self) -> Result<f64, mpris_server::zbus::fdo::Error> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> Result<f64, mpris_server::zbus::fdo::Error> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(true)
    }

    async fn can_go_previous(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(true)
    }

    async fn can_play(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(true)
    }

    async fn can_pause(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(true)
    }

    async fn can_seek(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(false)
    }

    async fn can_control(&self) -> Result<bool, mpris_server::zbus::fdo::Error> {
        Ok(true)
    }
}

impl MediaKeyHandler {
    pub fn new(cx: &mut App) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Self::setup_mpris(cx)?;
            Ok(Self {})
        }

        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            Self::setup_global_hotkeys(cx)
        }
    }

    #[cfg(target_os = "linux")]
    fn setup_mpris(cx: &mut App) -> Result<()> {
        let (action_tx, mut action_rx) = mpsc::unbounded_channel();

        cx.spawn(|cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                while let Some(action) = action_rx.recv().await {
                    use tracing::debug;

                    debug!("MPRIS action: {:?}", action);
                    cx.update(|cx| match action {
                        PlayerAction::PlayPause => {
                            cx.update_global::<crate::media::playback::Playback, _>(|p, _| {
                                p.play_pause()
                            });
                        }
                        PlayerAction::Play => {
                            cx.update_global::<crate::media::playback::Playback, _>(|p, _| {
                                p.play()
                            });
                        }
                        PlayerAction::Pause => {
                            cx.update_global::<crate::media::playback::Playback, _>(|p, _| {
                                p.pause()
                            });
                        }
                        PlayerAction::Next => {
                            let _ = crate::media::queue::Queue::next(cx);
                        }
                        PlayerAction::Previous => {
                            let _ = crate::media::queue::Queue::previous(cx);
                        }
                    })
                    .ok();
                }
            }
        })
        .detach();

        tokio::spawn(async move {
            let player = VleerPlayer { action_tx };

            match Server::new("org.mpris.MediaPlayer2.vleer", player).await {
                Ok(_server) => loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
                },
                Err(e) => {
                    use tracing::error;

                    error!("Failed to start MPRIS server: {}", e);
                }
            }
        });

        Ok(())
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn setup_global_hotkeys(cx: &mut App) -> Result<Self> {
        use tracing::info;

        let manager = GlobalHotKeyManager::new()?;

        let play_pause = HotKey::new(None, Code::MediaPlayPause);
        let next = HotKey::new(None, Code::MediaTrackNext);
        let previous = HotKey::new(None, Code::MediaTrackPrevious);

        manager.register(play_pause)?;
        manager.register(next)?;
        manager.register(previous)?;

        let receiver = GlobalHotKeyEvent::receiver();
        cx.spawn(|cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                loop {
                    if let Ok(event) = receiver.try_recv() {
                        cx.update(|cx| {
                            if event.id == play_pause.id() {
                                cx.update_global::<crate::media::playback::Playback, _>(|p, _| {
                                    p.play_pause()
                                });
                            } else if event.id == next.id() {
                                let _ = crate::media::queue::Queue::next(cx);
                            } else if event.id == previous.id() {
                                let _ = crate::media::queue::Queue::previous(cx);
                            }
                        })
                        .ok();
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
            }
        })
        .detach();

        Ok(Self {
            _hotkey_manager: manager,
        })
    }
}
