use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender as EventSender, unbounded};
use gpui::{App, AsyncApp, BorrowAppContext, Global, QuitMode};
use tracing::{info, warn};

use crate::data::config::Config;
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use crate::ui::app::open_main_window;
use crate::ui::global_actions::{Quit, quit};

pub type Snapshot = (bool, Option<String>);

pub enum TrayEvent {
    Show,
    Quit,
}

#[cfg(target_os = "linux")]
mod platform {
    use std::sync::LazyLock;

    use anyhow::Result;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::StandardItem;
    use ksni::{Icon, MenuItem};
    use tokio::sync::mpsc::UnboundedSender;

    use super::{EventSender, Snapshot, TrayEvent};
    use crate::media::playback::PlaybackCommand;

    static ICON: LazyLock<Option<Icon>> = LazyLock::new(|| {
        let image = image::load_from_memory(include_bytes!("../assets/images/icon-128.png"))
            .ok()?
            .into_rgba8();
        let (width, height) = image.dimensions();
        let mut data = image.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        Some(Icon {
            width: width as i32,
            height: height as i32,
            data,
        })
    });

    struct VleerTray {
        playing: bool,
        now_playing: Option<String>,
        playback: UnboundedSender<PlaybackCommand>,
        events: EventSender<TrayEvent>,
    }

    impl VleerTray {
        fn command(command: fn() -> PlaybackCommand) -> Box<dyn Fn(&mut Self) + Send> {
            Box::new(move |tray| {
                let _ = tray.playback.send(command());
            })
        }

        fn event(event: fn() -> TrayEvent) -> Box<dyn Fn(&mut Self) + Send> {
            Box::new(move |tray| {
                let _ = tray.events.unbounded_send(event());
            })
        }
    }

    impl ksni::Tray for VleerTray {
        fn id(&self) -> String {
            "vleer".into()
        }

        fn title(&self) -> String {
            "Vleer".into()
        }

        fn category(&self) -> ksni::Category {
            ksni::Category::ApplicationStatus
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            ICON.clone().into_iter().collect()
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.events.unbounded_send(TrayEvent::Show);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let mut items = vec![
                StandardItem {
                    label: "Show Vleer".into(),
                    icon_name: "window-new".into(),
                    activate: Self::event(|| TrayEvent::Show),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
            ];

            if let Some(label) = &self.now_playing {
                items.push(
                    StandardItem {
                        label: label.clone(),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
            }

            items.extend([
                StandardItem {
                    label: "Previous".into(),
                    icon_name: "media-skip-backward".into(),
                    activate: Self::command(|| PlaybackCommand::Previous),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: if self.playing { "Pause" } else { "Play" }.into(),
                    icon_name: if self.playing {
                        "media-playback-pause"
                    } else {
                        "media-playback-start"
                    }
                    .into(),
                    activate: Self::command(|| PlaybackCommand::PlayPause),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Next".into(),
                    icon_name: "media-skip-forward".into(),
                    activate: Self::command(|| PlaybackCommand::Next),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: "Quit".into(),
                    icon_name: "application-exit".into(),
                    activate: Self::event(|| TrayEvent::Quit),
                    ..Default::default()
                }
                .into(),
            ]);

            items
        }
    }

    pub struct Platform {
        handle: Handle<VleerTray>,
    }

    impl Platform {
        pub fn start(
            (playing, now_playing): Snapshot,
            playback: UnboundedSender<PlaybackCommand>,
            events: EventSender<TrayEvent>,
        ) -> Result<Self> {
            let handle = VleerTray {
                playing,
                now_playing,
                playback,
                events,
            }
            .spawn()?;
            Ok(Self { handle })
        }

        pub fn update(&self, (playing, now_playing): Snapshot) {
            self.handle.update(|tray| {
                tray.playing = playing;
                tray.now_playing = now_playing;
            });
        }

        pub fn stop(self) {
            self.handle.shutdown().wait();
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod platform {
    use anyhow::Result;
    use tokio::sync::mpsc::UnboundedSender;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::{EventSender, Snapshot, TrayEvent};
    use crate::media::playback::PlaybackCommand;

    const SHOW: &str = "show";
    const PREVIOUS: &str = "previous";
    const PLAY_PAUSE: &str = "play-pause";
    const NEXT: &str = "next";
    const QUIT: &str = "quit";

    pub struct Platform {
        _tray: TrayIcon,
        now_playing: MenuItem,
        play_pause: MenuItem,
    }

    impl Platform {
        pub fn start(
            snapshot: Snapshot,
            playback: UnboundedSender<PlaybackCommand>,
            events: EventSender<TrayEvent>,
        ) -> Result<Self> {
            let now_playing = MenuItem::new("Nothing playing", false, None);
            let play_pause = MenuItem::with_id(PLAY_PAUSE, "Play", true, None);

            let menu = Menu::new();
            menu.append_items(&[
                &MenuItem::with_id(SHOW, "Show Vleer", true, None),
                &PredefinedMenuItem::separator(),
                &now_playing,
                &MenuItem::with_id(PREVIOUS, "Previous", true, None),
                &play_pause,
                &MenuItem::with_id(NEXT, "Next", true, None),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id(QUIT, "Quit", true, None),
            ])?;

            {
                let events = events.clone();
                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    match event.id.as_ref() {
                        SHOW => {
                            let _ = events.unbounded_send(TrayEvent::Show);
                        }
                        QUIT => {
                            let _ = events.unbounded_send(TrayEvent::Quit);
                        }
                        PREVIOUS => {
                            let _ = playback.send(PlaybackCommand::Previous);
                        }
                        PLAY_PAUSE => {
                            let _ = playback.send(PlaybackCommand::PlayPause);
                        }
                        NEXT => {
                            let _ = playback.send(PlaybackCommand::Next);
                        }
                        _ => {}
                    }
                }));
            }

            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = events.unbounded_send(TrayEvent::Show);
                }
            }));

            let image = image::load_from_memory(include_bytes!("../assets/images/icon-128.png"))?
                .into_rgba8();
            let (width, height) = image.dimensions();
            let icon = Icon::from_rgba(image.into_raw(), width, height)?;

            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(cfg!(target_os = "macos"))
                .with_tooltip("Vleer")
                .with_icon(icon)
                .build()?;

            let platform = Self {
                _tray: tray,
                now_playing,
                play_pause,
            };
            platform.update(snapshot);
            Ok(platform)
        }

        pub fn update(&self, (playing, now_playing): Snapshot) {
            self.play_pause
                .set_text(if playing { "Pause" } else { "Play" });
            self.now_playing
                .set_text(now_playing.as_deref().unwrap_or("Nothing playing"));
        }

        pub fn stop(self) {
            MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
            TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
        }
    }
}

use platform::Platform;

struct TrayState {
    platform: Option<Platform>,
    events: EventSender<TrayEvent>,
    requested: bool,
    shown: Snapshot,
}

impl Global for TrayState {}

pub fn init(cx: &mut App) {
    let (events, mut receiver) = unbounded();
    cx.set_global(TrayState {
        platform: None,
        events,
        requested: false,
        shown: (false, None),
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(event) = receiver.next().await {
            cx.update(|cx| match event {
                TrayEvent::Show => show_window(cx),
                TrayEvent::Quit => quit(&Quit, cx),
            });
        }
    })
    .detach();

    cx.observe_global::<Config>(sync).detach();
    cx.observe_global::<Playback>(refresh).detach();
    cx.observe_global::<Queue>(refresh).detach();

    sync(cx);
}

pub fn prepare_close(cx: &mut App) {
    if close_to_tray_active(cx) {
        cx.set_quit_mode(QuitMode::Explicit);
    }
}

fn close_to_tray_active(cx: &App) -> bool {
    cx.global::<Config>().get().general.close_to_tray
        && cx
            .try_global::<TrayState>()
            .is_some_and(|state| state.platform.is_some())
}

fn show_window(cx: &mut App) {
    match cx.windows().first().copied() {
        Some(window) => {
            let _ = window.update(cx, |_, window, _| window.activate());
            cx.activate(true);
        }
        None => open_main_window(cx),
    }
}

fn snapshot(cx: &App) -> Snapshot {
    let playing = cx.global::<Playback>().get_playing();
    let now_playing = cx.global::<Queue>().get_current_song(cx).map(|song| {
        if song.artists.is_empty() {
            song.title
        } else {
            format!("{} - {}", song.title, song.artists.join(", "))
        }
    });
    (playing, now_playing)
}

fn sync(cx: &mut App) {
    let wanted = cx.global::<Config>().get().general.tray_icon;
    if cx.global::<TrayState>().requested == wanted {
        return;
    }
    cx.update_global::<TrayState, _>(|state, _| state.requested = wanted);

    if wanted {
        start(cx);
    } else {
        stop(cx);
    }
}

fn start(cx: &mut App) {
    let current = snapshot(cx);
    let playback = Playback::get_command_sender(cx);
    let events = cx.global::<TrayState>().events.clone();

    match Platform::start(current.clone(), playback, events) {
        Ok(platform) => {
            info!("System tray started");
            cx.update_global::<TrayState, _>(|state, _| {
                state.platform = Some(platform);
                state.shown = current;
            });
        }
        Err(err) => warn!(?err, "System tray unavailable"),
    }
}

fn stop(cx: &mut App) {
    let platform = cx.update_global::<TrayState, _>(|state, _| state.platform.take());
    if let Some(platform) = platform {
        platform.stop();
        info!("System tray stopped");
    }
}

fn refresh(cx: &mut App) {
    if cx.global::<TrayState>().platform.is_none() {
        return;
    }
    let current = snapshot(cx);
    if cx.global::<TrayState>().shown == current {
        return;
    }

    cx.update_global::<TrayState, _>(|state, _| {
        if let Some(platform) = &state.platform {
            platform.update(current.clone());
        }
        state.shown = current;
    });
}
