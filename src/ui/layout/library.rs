use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

use crate::data::state::State;
use crate::data::types::{Cuid, Song};
use crate::media::playback::Playback;
use crate::media::queue::Queue;

use crate::ui::components::div::flex_row;
use crate::ui::components::icons::icon::icon;
use crate::ui::components::scrollbar::ScrollableElement;
use crate::ui::{
    components::{
        div::flex_col,
        icons::icons::{self, PLAY},
        input::{InputEvent, TextInput},
        nav_button::NavButton,
        title::Title,
    },
    variables::Variables,
    views::AppView,
};

pub struct Library {
    pub hovered: bool,
    search_input: Entity<TextInput>,
    search_query: String,
}

impl Library {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| TextInput::new(cx, "Search Library").with_icon(icons::SEARCH));

        cx.subscribe(
            &search_input,
            |this: &mut Self, _input, event: &InputEvent, cx| match event {
                InputEvent::Change(text) => {
                    this.search_query = text.clone();
                    cx.update_global::<State, _>(|state, _| {
                        state.set_search_query_sync(text.clone());
                    });
                    cx.notify();
                }
                InputEvent::Submit(text) => {
                    this.search_query = text.clone();
                    cx.update_global::<State, _>(|state, _| {
                        state.set_search_query_sync(text.clone());
                    });
                    cx.notify();
                }
            },
        )
        .detach();

        Self {
            hovered: false,
            search_input,
            search_query: String::new(),
        }
    }

    fn get_match_counts(&self, cx: &App) -> (usize, usize, usize, usize) {
        let state = cx.global::<State>();
        state.get_search_match_counts_sync(&self.search_query)
    }
}

fn pinned_item(
    id: Cuid,
    name: String,
    image_hash: Option<String>,
    item_type: String,
    variables: &Variables,
) -> impl IntoElement {
    let is_artist = item_type == "Artist";
    let item_type_clone = item_type.clone();
    let id_clone = id.clone();

    let covers_dir = dirs::data_dir()
        .expect("couldn't get data directory")
        .join("vleer")
        .join("covers");

    let cover_uri = image_hash.and_then(|hash| {
        let cover_path = covers_dir.join(hash);
        if cover_path.exists() {
            Some(format!("!file://{}", cover_path.to_string_lossy()))
        } else {
            None
        }
    });

    let cover_element = if let Some(uri) = cover_uri {
        img(uri)
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
                                let state = cx.global::<State>().clone();

                                cx.spawn(move |cx: &mut gpui::AsyncApp| {
                                    let cx = cx.clone();
                                    async move {
                                        let songs: Vec<Arc<Song>> = match item_type.as_str() {
                                            "Album" => {
                                                state.get_album_songs(&id).await.unwrap_or_default()
                                            }
                                            "Playlist" => state
                                                .get_playlist_songs(&id)
                                                .await
                                                .unwrap_or_default(),
                                            "Song" => state
                                                .get_song(&id)
                                                .await
                                                .map(|s| vec![s])
                                                .unwrap_or_default(),
                                            _ => Vec::new(),
                                        };

                                        if !songs.is_empty() {
                                            cx.update(|cx| {
                                                cx.update_global::<Queue, _>(|queue, _cx| {
                                                    queue.clear_and_queue_songs(&songs);
                                                });

                                                if let Err(e) = Playback::play_queue(cx) {
                                                    tracing::error!(
                                                        "Failed to start playback: {}",
                                                        e
                                                    );
                                                }
                                            })
                                            .ok();
                                        }
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
        let state = cx.global::<State>();

        let is_searching = !self.search_query.is_empty();
        let pinned_items = if is_searching {
            state.search_all_items_sync(&self.search_query)
        } else {
            state.get_pinned_items_sync()
        };
        let has_items = !pinned_items.is_empty();

        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        let (s_count, al_count, ar_count, p_count) = if is_searching {
            self.get_match_counts(cx)
        } else {
            (0, 0, 0, 0)
        };

        div()
            .id("library")
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                flex_col()
                    .size_full()
                    .min_h_0()
                    .border(px(1.0))
                    .border_color(border_color)
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
                    .when(has_items, |this| {
                        this.child(
                            flex_col()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(
                                    div().pr(px(variables.padding_16)).child(
                                        div()
                                            .w_full()
                                            .h(px(1.0))
                                            .bg(variables.border)
                                            .flex_shrink_0(),
                                    ),
                                )
                                .child(
                                    div().flex_1().min_h_0().overflow_y_scrollbar().child(
                                        flex_col()
                                            .gap(px(variables.padding_8))
                                            .pr(px(variables.padding_16))
                                            .py(px(variables.padding_16))
                                            .children(pinned_items.into_iter().take(30).map(
                                                |(id, name, cover, item_type)| {
                                                    pinned_item(
                                                        id, name, cover, item_type, variables,
                                                    )
                                                },
                                            )),
                                    ),
                                ),
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
            .child(Title::new("Library", self.hovered))
    }
}
