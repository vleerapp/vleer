use anyhow::Ok;
use gpui::{prelude::FluentBuilder, *};
use std::{collections::HashMap, fs};
use tracing::{debug, error, info};

use crate::{
    data::{
        config::Config,
        db::{Database, create_pool},
        scan::Scanner,
        state::State, telemetry::Telemetry,
    },
    media::{playback::Playback, queue::Queue},
    ui::{
        assets::VleerAssetSource,
        components::{
            div::{flex_col, flex_row},
            input::bind_input_keys,
        },
        global_actions::register_actions,
        layout::{library::Library, navbar::Navbar, player::Player},
        media_keys::MediaKeyHandler,
        variables::Variables,
        views::{AppView, ViewRegistry},
    },
};

struct MainWindow {
    library: Entity<Library>,
    navbar: Entity<Navbar>,
    player: Entity<Player>,
    views: HashMap<AppView, AnyView>,
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

        for (view_type, entity) in &self.views {
            let is_current = *view_type == current_view;
            let should_hover = is_current && target == HoverTarget::Content;

            ViewRegistry::set_hovered(*view_type, entity, should_hover, cx);
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let state = cx.global::<State>();
        let current_view = state.get_current_view_sync();

        let content: AnyElement = self
            .views
            .get(&current_view)
            .map(|view| view.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element());

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
                                    .size_full()
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

    let pool = create_pool(data_dir.join("library.db")).await?;

    Application::new()
        .with_assets(VleerAssetSource::new())
        .run(move |cx| {
            Database::init(cx, pool).expect("unable to initizalize database");
            Config::init(cx, &config_dir).expect("unable to initizalize settings");
            Playback::init(cx).expect("unable to initizalize playback context");
            Queue::init(cx);
            Variables::init(cx);
            Telemetry::init(cx, data_dir.clone());
            State::init(cx);
            Scanner::init(cx);

            find_fonts(cx)
                .inspect_err(|e| error!(?e, "Failed to load fonts"))
                .ok();
            register_actions(cx);
            bind_input_keys(cx);

            match MediaKeyHandler::new(cx) {
                std::result::Result::Ok(_handler) => {
                    info!("Media key handler initialized");
                    cx.set_global(_handler);
                }
                std::result::Result::Err(e) => {
                    error!("Failed to setup media keys: {}", e);
                }
            }

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
                        Playback::start_playback_monitor(window, cx);

                        let views = ViewRegistry::register_all(window, cx);

                        MainWindow {
                            library: cx.new(|cx| Library::new(cx)),
                            navbar: cx.new(|_cx| Navbar::new()),
                            player: cx.new(|_cx| Player::new()),
                            views,
                        }
                    })
                },
            )
            .unwrap();
        });

    Ok(())
}
