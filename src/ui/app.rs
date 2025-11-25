use anyhow::Ok;
use gpui::{prelude::FluentBuilder, *};
use std::fs;
use tracing::debug;

use crate::{
    data::{
        config::{Config, ConfigWatcher},
        db::{Database, create_pool},
        scan::{MusicScanner, MusicWatcher, expand_scan_paths},
    },
    media::{playback::PlaybackContext, queue::Queue},
    ui::{
        assets::VleerAssetSource,
        components::div::{flex_col, flex_row},
        layout::{library::Library, navbar::Navbar, player::Player},
        state::State,
        variables::Variables,
        views::{AppView, HomeView, SongsView},
    },
};

struct MainWindow {
    library: Entity<Library>,
    navbar: Entity<Navbar>,
    player: Entity<Player>,
    home_view: Entity<HomeView>,
    songs_view: Entity<SongsView>,
}

#[derive(Clone, Copy, PartialEq)]
enum HoverTarget {
    Library,
    Navbar,
    Content,
    Player,
}

impl MainWindow {
    fn set_hover(&mut self, target: HoverTarget, cx: &mut Context<Self>) {
        self.library.update(cx, |library, cx| {
            library.hovered = target == HoverTarget::Library;
            cx.notify();
        });
        self.navbar.update(cx, |navbar, cx| {
            navbar.hovered = target == HoverTarget::Navbar;
            cx.notify();
        });
        self.player.update(cx, |player, cx| {
            player.hovered = target == HoverTarget::Player;
            cx.notify();
        });

        let state = cx.global::<State>();
        let current_view = state.get_current_view_sync();
        match current_view {
            AppView::Home => {
                self.home_view.update(cx, |home, cx| {
                    home.hovered = target == HoverTarget::Content;
                    cx.notify();
                });
                self.songs_view.update(cx, |songs, cx| {
                    songs.hovered = false;
                    cx.notify();
                });
            }
            AppView::Songs => {
                self.home_view.update(cx, |home, cx| {
                    home.hovered = false;
                    cx.notify();
                });
                self.songs_view.update(cx, |songs, cx| {
                    songs.hovered = target == HoverTarget::Content;
                    cx.notify();
                });
            }
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let state = cx.global::<State>();
        let current_view = state.get_current_view_sync();

        let content: AnyElement = match current_view {
            AppView::Home => self.home_view.clone().into_any_element(),
            AppView::Songs => self.songs_view.clone().into_any_element(),
        };

        let mut element = flex_col()
            .gap(px(variables.padding_16))
            .p(px(variables.padding_16))
            .size_full()
            .bg(variables.background)
            .child(div().h(px(0.0)).when(
                !(cfg!(target_os = "macos") || cfg!(target_os = "windows")),
                |s| s.hidden(),
            ))
            .child(
                flex_row()
                    .flex_1()
                    .size_full()
                    .gap(px(variables.padding_16))
                    .child(
                        div()
                            .id("library")
                            .w(px(300.0))
                            .flex_shrink_0()
                            .h_full()
                            .on_mouse_move(cx.listener(
                                |this, _event: &MouseMoveEvent, _window: &mut Window, cx| {
                                    this.set_hover(HoverTarget::Library, cx);
                                },
                            ))
                            .child(self.library.clone()),
                    )
                    .child(
                        flex_col()
                            .flex_1()
                            .h_full()
                            .gap(px(variables.padding_16))
                            .child(
                                div()
                                    .id("navbar")
                                    .h(px(48.0))
                                    .w_full()
                                    .on_mouse_move(cx.listener(
                                        |this,
                                         _event: &MouseMoveEvent,
                                         _window: &mut Window,
                                         cx| {
                                            this.set_hover(HoverTarget::Navbar, cx);
                                        },
                                    ))
                                    .child(self.navbar.clone()),
                            )
                            .child(
                                div()
                                    .id("current-view")
                                    .flex_1()
                                    .w_full()
                                    .on_mouse_move(cx.listener(
                                        |this,
                                         _event: &MouseMoveEvent,
                                         _window: &mut Window,
                                         cx| {
                                            this.set_hover(HoverTarget::Content, cx);
                                        },
                                    ))
                                    .child(content),
                            ),
                    ),
            )
            .child(
                div()
                    .id("player")
                    .h(px(100.0))
                    .w_full()
                    .on_mouse_move(cx.listener(
                        |this, _event: &MouseMoveEvent, _window: &mut Window, cx| {
                            this.set_hover(HoverTarget::Player, cx);
                        },
                    ))
                    .child(self.player.clone()),
            );

        let text_styles = element.text_style();
        *text_styles = Some(TextStyleRefinement {
            color: Some(Hsla::from(variables.text)),
            font_family: Some(SharedString::new("Feature Mono")),
            font_size: Some(AbsoluteLength::Pixels(px(14.0))),
            line_height: Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(14.0)))),
            ..Default::default()
        });

        element
    }
}

pub fn find_fonts(cx: &mut App) -> gpui::Result<()> {
    let paths = cx.asset_source().list("!bundled:fonts")?;
    let mut fonts = vec![];
    for path in paths {
        if (path.ends_with(".ttf") || path.ends_with(".otf"))
            && let Some(v) = cx.asset_source().load(&path)?
        {
            fonts.push(v);
        }
    }

    let results = cx.text_system().add_fonts(fonts);
    debug!("loaded fonts: {:?}", cx.text_system().all_font_names());
    results
}

#[tokio::main]
pub async fn run() -> anyhow::Result<()> {
    let data_dir = dirs::data_dir()
        .expect("couldn't get data directory")
        .join("vleer");
    let config_dir = dirs::config_dir()
        .expect("couldn't get config directory")
        .join("vleer");

    fs::create_dir_all(&data_dir).inspect_err(|error| {
        tracing::error!(
            ?error,
            "couldn't create data directory '{}'",
            data_dir.display(),
        )
    })?;

    let pool = create_pool(data_dir.join("library.db")).await?;

    Application::new()
        .with_assets(VleerAssetSource::new())
        .run(move |cx| {
            Database::init(cx, pool).expect("unable to initizalize database");
            Config::init(cx, &config_dir).expect("unable to initizalize settings");
            PlaybackContext::init(cx).expect("unable to initizalize playback context");
            Queue::init(cx);
            Variables::init(cx);
            State::init(cx);

            let config = cx.global::<Config>().clone();
            let state = cx.global::<State>().clone();
            tokio::spawn(async move {
                state.set_config(config.get().clone()).await;
            });

            let config_path = config_dir.join("config.toml");
            match ConfigWatcher::new(config_path) {
                std::result::Result::Ok((_watcher, mut rx)) => {
                    cx.spawn(|cx: &mut gpui::AsyncApp| {
                        let cx = cx.clone();
                        async move {
                            let _watcher = _watcher;
                            while rx.recv().await.is_some() {
                                cx.update(|cx| {
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        if let Err(e) = config.reload() {
                                            tracing::error!("Failed to reload config: {}", e);
                                        } else {
                                            tracing::info!("Config reloaded successfully");
                                        }
                                    });

                                    let config = cx.global::<Config>().clone();
                                    let config_for_state = config.clone();
                                    let state = cx.global::<State>().clone();

                                    tokio::spawn(async move {
                                        state.set_config(config_for_state.get().clone()).await;
                                    });

                                    cx.update_global::<PlaybackContext, _>(|playback, _cx| {
                                        playback.apply_settings(&config);
                                        tracing::debug!("Applied reloaded settings to playback");
                                    });
                                })
                                .ok();
                            }
                        }
                    })
                    .detach();
                }
                Err(e) => {
                    tracing::error!("Failed to initialize config watcher: {}", e);
                }
            }

            let config = cx.global::<Config>();
            let scan_paths = expand_scan_paths(&config.get().scan.paths);
            let db = cx.global::<Database>().clone();
            let covers_dir = data_dir.join("covers");

            let scanner = std::sync::Arc::new(MusicScanner::new(scan_paths, covers_dir));
            let scanner_clone = scanner.clone();

            match MusicWatcher::new(scanner.clone(), std::sync::Arc::new(db.clone())) {
                std::result::Result::Ok((watcher, mut rx)) => {
                    tokio::spawn(async move {
                        let _watcher = watcher;
                        while let Some(stats) = rx.recv().await {
                            tracing::info!(
                                "Library scan completed - Added: {}, Updated: {}, Removed: {}",
                                stats.added,
                                stats.updated,
                                stats.removed
                            );
                        }
                    });

                    let db_clone = cx.global::<Database>().clone();
                    tokio::spawn(async move {
                        tracing::info!("Starting initial library scan...");
                        match scanner_clone.scan_and_save(&db_clone).await {
                            std::result::Result::Ok(stats) => {
                                tracing::info!(
                                    "Initial scan complete - Added: {}, Updated: {}, Removed: {}",
                                    stats.added,
                                    stats.updated,
                                    stats.removed
                                );
                            }
                            Err(e) => {
                                tracing::error!("Initial scan failed: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to initialize music watcher: {}", e);
                }
            }

            find_fonts(cx)
                .inspect_err(|e| tracing::error!(?e, "Failed to load fonts"))
                .expect("unable to load fonts");

            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some(SharedString::new("Vleer")),
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    app_id: Some("vleer".to_string()),
                    kind: gpui::WindowKind::Normal,
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("Vleer");

                    cx.new(|cx| {
                        PlaybackContext::start_playback_monitor(window, cx);

                        MainWindow {
                            library: cx.new(|_cx| Library::new()),
                            navbar: cx.new(|_cx| Navbar::new()),
                            player: cx.new(|_cx| Player::new()),
                            home_view: cx.new(|cx| HomeView::new(window, cx)),
                            songs_view: cx.new(|cx| SongsView::new(window, cx)),
                        }
                    })
                },
            )
            .unwrap();
        });

    Ok(())
}
