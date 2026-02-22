use crate::data::db::repo::Database;
use crate::data::models::{Cuid, PinnedItem};
use crate::media::playback::Playback;
use crate::media::queue::Queue;
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

const SEARCH_RESULT_LIMIT: i64 = 30;
const SEARCH_QUERY_DEBOUNCE: Duration = Duration::from_millis(140);
const SEARCH_COUNT_DEBOUNCE: Duration = Duration::from_millis(180);

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
    search_request_seq: u64,
    search_query_task: Option<Task<()>>,
    search_count_task: Option<Task<()>>,
}

impl Library {
    fn cancel_inflight_search(&mut self) {
        self.search_query_task = None;
        self.search_count_task = None;
    }

    fn on_search_query_changed(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.last_query {
            return;
        }
        self.last_query = query.clone();
        self.cancel_inflight_search();

        if query.is_empty() {
            self.search_request_seq = self.search_request_seq.wrapping_add(1);
            self.search_results.clear();
            self.search_counts = (0, 0, 0, 0);
            self.search_pending = false;
            cx.notify();
            return;
        }

        self.search_pending = true;
        self.search_request_seq = self.search_request_seq.wrapping_add(1);
        let request_seq = self.search_request_seq;
        cx.notify();

        let search_query = query.clone();
        let db = cx.global::<Database>().clone();
        let query_task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(SEARCH_QUERY_DEBOUNCE).await;

            let query_for_spawn = search_query.clone();
            let results = match crate::RUNTIME
                .spawn(async move {
                    db.search_library(&query_for_spawn, SEARCH_RESULT_LIMIT)
                        .await
                })
                .await
            {
                Ok(Ok(results)) => results,
                _ => Vec::new(),
            };

            cx.update(|cx| {
                this.update(cx, |lib, cx| {
                    if lib.last_query != search_query || lib.search_request_seq != request_seq {
                        return;
                    }

                    lib.search_results = results
                        .into_iter()
                        .map(|(id, name, image_id, item_type)| PinnedItem {
                            id,
                            name,
                            image_id,
                            item_type,
                        })
                        .collect();
                    lib.search_pending = false;
                    cx.notify();
                })
            })
            .ok();
        });
        self.search_query_task = Some(query_task);

        let count_query = query.clone();
        let db = cx.global::<Database>().clone();
        let count_task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(SEARCH_COUNT_DEBOUNCE).await;

            let query_for_spawn = count_query.clone();
            let counts = match crate::RUNTIME
                .spawn(async move { db.get_search_match_counts(&query_for_spawn).await })
                .await
            {
                Ok(Ok(counts)) => counts,
                _ => (0, 0, 0, 0),
            };

            cx.update(|cx| {
                this.update(cx, |lib, cx| {
                    if lib.last_query != count_query || lib.search_request_seq != request_seq {
                        return;
                    }

                    lib.search_counts = counts;
                    cx.notify();
                })
            })
            .ok();
        });
        self.search_count_task = Some(count_task);
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
            cx.update_global::<Search, _>(|s, _cx| {
                s.query = query.clone().into();
            });
            this.on_search_query_changed(query, cx);
        })
        .detach();

        cx.observe_global::<Search>(|this, cx| {
            let query = cx.global::<Search>().query.to_string();
            let query = query.trim().to_string();
            this.on_search_query_changed(query, cx);
        })
        .detach();

        Self {
            search_input,
            pinned_items: Vec::new(),
            search_results: Vec::new(),
            search_counts: (0, 0, 0, 0),
            search_pending: false,
            last_query: String::new(),
            search_request_seq: 0,
            search_query_task: None,
            search_count_task: None,
        }
    }
}

fn pinned_item(
    id: Cuid,
    name: String,
    image_id: Option<String>,
    item_type: String,
    variables: &Variables,
) -> impl IntoElement {
    let is_artist = item_type == "Artist";
    let item_type_clone = item_type.clone();
    let id_clone = id.clone();

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
                                    let song_ids = match crate::RUNTIME
                                        .spawn(async move {
                                            match item_type.as_str() {
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
                                            }
                                        })
                                        .await
                                    {
                                        Ok(song_ids) => song_ids,
                                        Err(_) => Vec::new(),
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

        div()
            .image_cache(app_image_cache())
            .size_full()
            .min_w_0()
            .min_h_0()
            .group("library")
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
