use anyhow::Ok;
use gpui::*;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use std::{collections::HashMap, fs, sync::Arc};
use tracing::{debug, error};

use crate::{
    data::{config::Config, db::repo::Database, scanner::Scanner, telemetry::Telemetry},
    media::{controller::MediaController, playback::Playback, queue::Queue},
    ui::{
        assets::VleerAssetSource,
        components::{
            div::{flex_col, flex_row},
            input::bind_input_keys,
            pane::pane,
            window_controls::WindowControls,
        },
        discord_presence::DiscordPresence,
        global_actions::register_actions,
        layout::{
            library::{Library, Search},
            navbar::Navbar,
            player::Player,
        },
        variables::Variables,
        views::{AppView, ViewRegistry},
    },
};

pub(crate) struct MainWindow {
    library: Entity<Library>,
    navbar: Entity<Navbar>,
    player: Entity<Player>,
    views: HashMap<AppView, AnyView>,
    current_view: AppView,
    titlebar_should_move: bool,
}

impl MainWindow {
    pub fn current_view(&self) -> AppView {
        self.current_view
    }

    pub fn set_current_view(&mut self, view: AppView, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_view == view {
            return;
        }
        self.current_view = view;
        window.refresh();
        cx.notify();
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        let content: AnyElement = self
            .views
            .get(&self.current_view)
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element());

        let show_linux_controls = cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. });
        let show_titlebar = cfg!(target_os = "windows") || show_linux_controls;
        let is_macos = cfg!(target_os = "macos");
        let titlebar_height = px(32.0);

        let mut element = flex_col().size_full().min_h_0().bg(variables.background);

        if show_titlebar {
            let mut titlebar = flex_row()
                .id("window-titlebar")
                .w_full()
                .h(titlebar_height)
                .justify_between()
                .bg(variables.background)
                .window_control_area(WindowControlArea::Drag)
                .child(div().flex_1().pl(px(variables.padding_16)))
                .child(WindowControls::new(titlebar_height).into_any_element());

            if show_linux_controls {
                titlebar = titlebar
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _ev, _window, _cx| {
                            this.titlebar_should_move = true;
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _ev, _window, _cx| {
                            this.titlebar_should_move = false;
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, _ev, window, _cx| {
                        if this.titlebar_should_move {
                            this.titlebar_should_move = false;
                            window.start_window_move();
                        }
                    }));
            }

            element = element.child(titlebar);
        }

        let content_pt = if show_titlebar {
            px(0.0)
        } else if is_macos {
            px(variables.padding_16 * 2.0)
        } else {
            px(variables.padding_16)
        };

        element = element.child(
            flex_col()
                .pt(content_pt)
                .pr(px(variables.padding_16))
                .pb(px(variables.padding_16))
                .pl(px(variables.padding_16))
                .gap(px(variables.padding_16))
                .flex_1()
                .min_h_0()
                .size_full()
                .child(
                    flex_row()
                        .flex_1()
                        .min_h_0()
                        .size_full()
                        .gap(px(variables.padding_16))
                        .child(
                            div()
                                .id("library-container")
                                .w(px(300.0))
                                .flex_shrink_0()
                                .min_h_0()
                                .h_full()
                                .child(
                                    pane("library").title("Library").child(self.library.clone()),
                                ),
                        )
                        .child(
                            flex_col()
                                .flex_1()
                                .min_h_0()
                                .h_full()
                                .gap(px(variables.padding_16))
                                .child(
                                    div()
                                        .id("navbar-container")
                                        .h(px(48.0))
                                        .w_full()
                                        .flex_shrink_0()
                                        .child(
                                            pane("navbar")
                                                .title("Navbar")
                                                .child(self.navbar.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("current-view-container")
                                        .flex_1()
                                        .min_h_0()
                                        .size_full()
                                        .child(
                                            pane("current-view")
                                                .title(self.current_view.title())
                                                .child(content),
                                        ),
                                ),
                        ),
                )
                .child(
                    div()
                        .id("player-container")
                        .h(px(100.0))
                        .flex_shrink_0()
                        .w_full()
                        .child(
                            pane("player")
                                .title("Player")
                                .child(self.player.clone())
                                .into_any_element(),
                        ),
                ),
        );

        let text_styles = element.text_style();
        *text_styles = TextStyleRefinement {
            color: Some(Hsla::from(variables.text)),
            font_family: Some(SharedString::new("Feature Mono")),
            font_size: Some(AbsoluteLength::Pixels(px(14.0))),
            line_height: Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(14.0)))),
            ..Default::default()
        };

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

    let pool = {
        let options = SqliteConnectOptions::new()
            .filename(data_dir.join("library.db"))
            .optimize_on_close(true, None)
            .synchronous(SqliteSynchronous::Normal)
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true);

        let pool = SqlitePool::connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Arc::new(pool)
    };

    Application::new()
        .with_assets(VleerAssetSource::new(pool.clone()))
        .run(move |cx| {
            cx.set_global(Database { pool: pool.clone() });
            cx.set_global(Search::default());

            Config::init(cx, &config_dir).expect("unable to initizalize settings");
            Playback::init(cx).expect("unable to initizalize playback context");
            DiscordPresence::init(cx);
            Queue::init(cx);
            Variables::init(cx);
            Telemetry::init(cx, data_dir.clone());
            Scanner::init(cx);
            MediaController::init(cx);

            find_fonts(cx)
                .inspect_err(|e| error!(?e, "Failed to load fonts"))
                .ok();
            register_actions(cx);
            bind_input_keys(cx);

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

                    #[cfg(target_os = "windows")]
                    if let Some(mc) = cx.try_global::<MediaController>() {
                        mc.set_window_handle(window);
                    }

                    cx.new(|cx| {
                        Playback::start_monitor(window, cx);

                        let library_entity = cx.new(|cx| Library::new(cx));
                        let navbar_entity = cx.new(|_cx| Navbar::new());
                        let player_entity = cx.new(|_cx| Player::new());

                        let views = ViewRegistry::register_all(window, cx);

                        MainWindow {
                            library: library_entity,
                            navbar: navbar_entity,
                            player: player_entity,
                            views,
                            current_view: AppView::Home,
                            titlebar_should_move: false,
                        }
                    })
                },
            )
            .unwrap();
        });

    Ok(())
}
