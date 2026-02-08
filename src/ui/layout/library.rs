use crate::data::db::repo::Database;
use crate::data::models::{Cuid, PinnedItem};
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use crate::ui::components::div::flex_row;
use crate::ui::components::icons::icon::icon;
use crate::ui::components::scrollbar::ScrollableElement;
use crate::ui::{
    assets::image_cache::vleer_cache,
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

#[derive(Default)]
pub struct SearchState {
    pub query: SharedString,
}

impl Global for SearchState {}

pub struct Library {
    search_input: Entity<TextInput>,
    pinned_items: Vec<PinnedItem>,
}

impl Library {
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

        cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| {
            let text = match event {
                InputEvent::Change(text) | InputEvent::Submit(text) => text,
            };
            cx.update_global::<SearchState, _>(|s, _cx| {
                s.query = text.clone().into();
            });
        })
        .detach();

        cx.observe_global::<SearchState>(|_this, cx| cx.notify())
            .detach();

        Self {
            search_input,
            pinned_items: Vec::new(),
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

                                            let _ = Playback::play_queue(cx);
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
        let search = cx.global::<SearchState>();
        let query = search.query.to_string();
        let is_searching = !query.is_empty();

        let (s_count, al_count, ar_count, p_count) = if is_searching {
            let db = cx.global::<Database>().clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(db.get_search_match_counts(&query))
                    .unwrap_or((0, 0, 0, 0))
            })
        } else {
            (0, 0, 0, 0)
        };

        let total_matches = s_count + al_count + ar_count + p_count;
        let has_items = !self.pinned_items.is_empty();
        let show_pinned = !is_searching && has_items;
        let show_no_results = is_searching && total_matches == 0;

        div()
            .image_cache(vleer_cache("library-image-cache", 200))
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
                    .when(show_pinned || show_no_results, |this| {
                        this.child(
                            flex_col()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(
                                    div().pr(px(variables.padding_16)).child(
                                        div()
                                            .w_full()
                                            .h(if show_no_results { px(0.5) } else { px(1.0) })
                                            .bg(variables.border)
                                            .flex_shrink_0(),
                                    ),
                                )
                                .child(if show_no_results {
                                    div()
                                        .pt(px(variables.padding_16))
                                        .text_color(variables.text_secondary)
                                        .child("No Results Found")
                                        .into_any_element()
                                } else {
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scrollbar()
                                        .child(
                                            flex_col()
                                                .gap(px(variables.padding_8))
                                                .pr(px(variables.padding_16))
                                                .py(px(variables.padding_16))
                                                .children(self.pinned_items.iter().take(30).map(
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
                                }),
                        )
                    })
                    .when(!has_items && is_searching, |this| {
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
                                .child(
                                    div()
                                        .pt(px(variables.padding_16))
                                        .text_color(variables.text_secondary)
                                        .child("No Results Found"),
                                ),
                        )
                    }),
            )
    }
}
