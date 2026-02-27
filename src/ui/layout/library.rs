use crate::data::db::repo::Database;
use crate::data::models::{Cuid, PinnedItem};
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use crate::ui::components::context_menu::{
    ContextMenu, PinnedItemsChanged, album_context_menu_items, artist_context_menu_items,
    playlist_context_menu_items, song_context_menu_items,
};
use crate::ui::components::div::flex_row;
use crate::ui::components::icons::icon::icon;
use crate::ui::components::scrollbar::ScrollableElement;
use crate::ui::{
    assets::image_cache::app_image_cache,
    components::{
        div::flex_col,
        icons::icons::{self, PLAY},
        input::{InputEvent, TextInput},
        nav_button::NavButton,
    },
    variables::Variables,
    views::AppView,
};
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::Duration;
use tracing::{error, info};

const SEARCH_RESULT_LIMIT: i64 = 20;

fn run_sync<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        crate::RUNTIME.block_on(future)
    }
}

#[derive(Default)]
pub struct Search {
    pub query: SharedString,
}

impl Global for Search {}

pub struct Library {
    search_input: Entity<TextInput>,
    pinned_items: Vec<PinnedItem>,
    search_results: Vec<PinnedItem>,
    search_counts: (usize, usize, usize, usize),
    search_pending: bool,
    last_query: String,
    _search_debounce: Option<Task<()>>,
    context_menu: Entity<ContextMenu>,
}

impl Library {
    fn run_search_sync(&mut self, query: &str, cx: &mut Context<Self>) {
        let db = cx.global::<Database>().clone();
        let results = match run_sync(db.search_library(query, SEARCH_RESULT_LIMIT)) {
            Ok(results) => results,
            Err(e) => {
                error!("library search failed: {}", e);
                Vec::new()
            }
        };
        let counts = match run_sync(db.get_search_match_counts(query)) {
            Ok(counts) => counts,
            Err(e) => {
                error!("library search counts failed: {}", e);
                (0, 0, 0, 0)
            }
        };

        info!(
            "Library search resolved, query: '{}', results: {}, counts: ({}, {}, {}, {})",
            query,
            results.len(),
            counts.0,
            counts.1,
            counts.2,
            counts.3
        );

        let mapped_results: Vec<PinnedItem> = results
            .into_iter()
            .map(|(id, name, image_id, item_type)| PinnedItem {
                id,
                name,
                image_id,
                item_type,
            })
            .collect();
        let data_changed = self.search_results != mapped_results || self.search_counts != counts;
        let pending_changed = self.search_pending;

        self.search_results = mapped_results;
        self.search_counts = counts;
        self.search_pending = false;

        if data_changed || pending_changed {
            cx.notify();
        }
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| TextInput::new(cx, "Search Library").with_icon(icons::SEARCH));

        let db = cx.global::<Database>().clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let items = db.get_pinned_items().await;

            cx.update(|cx| {
                this.update(cx, |lib, cx| {
                    lib.pinned_items = items;
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();

        cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
            let text = match event {
                InputEvent::Change(text) | InputEvent::Submit(text) => text,
            };
            let query = text.trim().to_string();
            info!("Library search_input subscribe, query: '{}'", query);
            let task = cx.spawn(async move |_, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                cx.update(|cx| {
                    cx.update_global::<Search, _>(|s, _cx| {
                        s.query = query.into();
                    });
                });
            });
            this._search_debounce = Some(task);
        })
        .detach();

        cx.observe_global::<Search>(|this, cx| {
            let query = cx.global::<Search>().query.trim().to_string();
            info!("Library observe_global::<Search>, query: '{}'", query);

            if query == this.last_query {
                return;
            }
            this.last_query = query.clone();

            if query.is_empty() {
                this.search_results.clear();
                this.search_counts = (0, 0, 0, 0);
                this.search_pending = false;
                cx.notify();
                return;
            }

            this.search_pending = true;
            cx.notify();
            this.run_search_sync(&query, cx);
        })
        .detach();

        cx.observe_global::<PinnedItemsChanged>(|_, cx| {
            let db = cx.global::<Database>().clone();
            cx.spawn(async move |this, cx: &mut AsyncApp| {
                let items = db.get_pinned_items().await;
                cx.update(|cx| {
                    this.update(cx, |lib, cx| {
                        lib.pinned_items = items;
                        cx.notify();
                    })
                })
                .ok();
            })
            .detach();
        })
        .detach();

        Self {
            search_input,
            pinned_items: Vec::new(),
            search_results: Vec::new(),
            search_counts: (0, 0, 0, 0),
            search_pending: false,
            last_query: String::new(),
            _search_debounce: None,
            context_menu: cx.new(|_| ContextMenu::new()),
        }
    }
}

fn pinned_item(
    id: Cuid,
    name: String,
    image_id: Option<String>,
    item_type: String,
    variables: &Variables,
    context_menu: Entity<ContextMenu>,
) -> impl IntoElement {
    let is_artist = item_type == "Artist";
    let item_type_clone = item_type.clone();
    let item_type_for_ctx = item_type.clone();
    let id_clone = id.clone();
    let id_for_ctx = id.clone();

    let cover_element = if let Some(uri) = image_id {
        img(format!("!image://{}", uri))
            .size_full()
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    } else {
        div().bg(variables.border).into_any_element()
    };

    flex_row()
        .group("pinned-item")
        .bg(variables.element)
        .hover(|s| s.bg(variables.element_hover))
        .gap(px(variables.padding_8))
        .pr(px(variables.padding_8))
        .on_mouse_down(MouseButton::Right, move |event, _window, cx| {
            let items = match item_type_for_ctx.as_str() {
                "Song" => song_context_menu_items(id_for_ctx.clone(), cx),
                "Album" => album_context_menu_items(id_for_ctx.clone(), cx),
                "Artist" => artist_context_menu_items(id_for_ctx.clone(), cx),
                "Playlist" => playlist_context_menu_items(id_for_ctx.clone(), cx),
                _ => vec![],
            };
            context_menu.update(cx, |menu, cx| {
                menu.show(event.position, items, cx);
            });
        })
        .child(
            div()
                .size(px(36.0))
                .map(|this| {
                    if is_artist {
                        this.rounded_tr(px(0.0))
                            .rounded_br(px(0.0))
                            .rounded_bl(px(18.0))
                            .rounded_tl(px(18.0))
                    } else {
                        this.rounded_full()
                    }
                })
                .rounded(px(18.0))
                .relative()
                .child(cover_element)
                .when(!is_artist, |this| {
                    this.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(black().opacity(0.5))
                            .invisible()
                            .group_hover("pinned-item", |s| s.visible())
                            .child(icon(PLAY).size(px(16.0)).text_color(white()))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                                let item_type = item_type_clone.clone();
                                let id = id_clone.clone();
                                let db = cx.global::<Database>().clone();

                                cx.spawn(async move |cx: &mut AsyncApp| {
                                    let song_ids = match item_type.as_str() {
                                        "Album" => db
                                            .get_album_songs(&id)
                                            .await
                                            .unwrap_or_default()
                                            .into_iter()
                                            .map(|s| s.id)
                                            .collect(),
                                        "Playlist" => db
                                            .get_playlist_songs(&id)
                                            .await
                                            .unwrap_or_default()
                                            .into_iter()
                                            .map(|pt| pt.song.id)
                                            .collect(),
                                        "Song" => vec![id],
                                        _ => Vec::new(),
                                    };

                                    if !song_ids.is_empty() {
                                        cx.update(|cx| {
                                            cx.update_global::<Queue, _>(|queue, _| {
                                                queue.clear();
                                                queue.add_songs(song_ids);
                                            });

                                            cx.update_global::<Playback, _>(|playback, cx| {
                                                playback.play_queue(cx);
                                            });
                                        })
                                    }
                                })
                                .detach();
                            }),
                    )
                }),
        )
        .child(
            div()
                .overflow_x_hidden()
                .text_ellipsis()
                .font_weight(FontWeight(500.0))
                .child(name),
        )
}

impl Render for Library {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let search = cx.global::<Search>();
        let query = search.query.trim().to_string();
        let is_searching = !query.is_empty();

        let (s_count, al_count, ar_count, p_count) = if is_searching {
            self.search_counts
        } else {
            (0, 0, 0, 0)
        };

        let displayed_items: Vec<PinnedItem> = if is_searching {
            self.search_results.clone()
        } else {
            self.pinned_items.clone()
        };

        let has_display = !displayed_items.is_empty();
        let is_search_pending = is_searching && self.search_pending;
        let context_menu = self.context_menu.clone();

        div()
            .image_cache(app_image_cache())
            .size_full()
            .min_w_0()
            .min_h_0()
            .group("library")
            .child(div().absolute().size_0().child(context_menu.clone()))
            .child(
                flex_col()
                    .size_full()
                    .min_h_0()
                    .group_hover("library", |s| s.border_color(variables.accent))
                    .pl(px(variables.padding_16))
                    .pr(px(0.0))
                    .pt(px(variables.padding_16))
                    .pb(px(0.0))
                    .gap(px(variables.padding_16))
                    .child(
                        div()
                            .pr(px(variables.padding_16))
                            .child(self.search_input.clone()),
                    )
                    .child(
                        flex_col()
                            .id("links")
                            .pr(px(variables.padding_16))
                            .gap(px(variables.padding_16))
                            .flex_shrink_0()
                            .child(NavButton::new(
                                icons::SONGS,
                                Some("Songs"),
                                Some(s_count),
                                AppView::Songs,
                            ))
                            .child(NavButton::new(
                                icons::ALBUM,
                                Some("Albums"),
                                Some(al_count),
                                AppView::Albums,
                            ))
                            .child(NavButton::new(
                                icons::ARTIST,
                                Some("Artists"),
                                Some(ar_count),
                                AppView::Artists,
                            ))
                            .child(NavButton::new(
                                icons::PLAYLIST,
                                Some("Playlists"),
                                Some(p_count),
                                AppView::Playlists,
                            )),
                    )
                    .when(has_display || (is_searching && !has_display), |this| {
                        this.child(
                            flex_col()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(
                                    div().pr(px(variables.padding_16)).child(
                                        div()
                                            .w_full()
                                            .h(px(0.5))
                                            .bg(variables.border)
                                            .flex_shrink_0(),
                                    ),
                                )
                                .child(if has_display {
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scrollbar()
                                        .child(
                                            flex_col()
                                                .gap(px(variables.padding_8))
                                                .pr(px(variables.padding_16))
                                                .py(px(variables.padding_16))
                                                .children(displayed_items.iter().take(30).map(
                                                    |item| {
                                                        pinned_item(
                                                            item.id.clone(),
                                                            item.name.clone(),
                                                            item.image_id.clone(),
                                                            item.item_type.clone(),
                                                            variables,
                                                            context_menu.clone(),
                                                        )
                                                    },
                                                )),
                                        )
                                        .into_any_element()
                                } else {
                                    if is_search_pending {
                                        div()
                                            .pt(px(variables.padding_16))
                                            .text_color(variables.text_secondary)
                                            .child("")
                                            .into_any_element()
                                    } else {
                                        div()
                                            .pt(px(variables.padding_16))
                                            .text_color(variables.text_secondary)
                                            .child("No Results Found")
                                            .into_any_element()
                                    }
                                }),
                        )
                    }),
            )
    }
}
