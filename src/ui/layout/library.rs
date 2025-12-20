use gpui::prelude::FluentBuilder;
use gpui::*;
use std::sync::Arc;

use crate::data::state::State;
use crate::data::types::{Cuid, Song};
use crate::media::playback::Playback;
use crate::media::queue::Queue;

use crate::ui::components::icons::icon::icon;
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
}

impl Library {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| TextInput::new(cx, "Search Library").with_icon(icons::SEARCH));

        cx.subscribe(
            &search_input,
            |_this: &mut Self, _input, event: &InputEvent, _cx| match event {
                InputEvent::Change(text) => {
                    println!("{}", text);
                }
                InputEvent::Submit(text) => {
                    println!("{}", text);
                }
            },
        )
        .detach();

        Self {
            hovered: false,
            search_input,
        }
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

    div()
        .flex()
        .group("pinned-item")
        .items_center()
        .bg(variables.element)
        .gap(px(variables.padding_8))
        .pr(px(variables.padding_8))
        .child(
            div()
                .size(px(36.0))
                .map(|this| if is_artist { this.rounded_full() } else { this })
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

        let pinned_items = state.get_pinned_items_sync();
        let has_pinned = !pinned_items.is_empty();

        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        div()
            .relative()
            .size_full()
            .child(
                flex_col()
                    .id("library")
                    .border(px(1.0))
                    .border_color(border_color)
                    .h_full()
                    .overflow_y_scroll()
                    .p(px(variables.padding_16))
                    .gap(px(variables.padding_16))
                    .child(self.search_input.clone())
                    .child(
                        flex_col()
                            .gap(px(variables.padding_16))
                            .id("links")
                            .child(NavButton::new(icons::SONGS, Some("Songs"), AppView::Songs))
                            .child(NavButton::new(
                                icons::ALBUM,
                                Some("Albums"),
                                AppView::Albums,
                            ))
                            .child(NavButton::new(
                                icons::ARTIST,
                                Some("Artists"),
                                AppView::Artists,
                            ))
                            .child(NavButton::new(
                                icons::PLAYLIST,
                                Some("Playlists"),
                                AppView::Playlists,
                            )),
                    )
                    .when(has_pinned, |this| {
                        this.child(
                            flex_col()
                                .id("pinned")
                                .w_full()
                                .gap(px(variables.padding_8))
                                .pt(px(variables.padding_16))
                                .border_t(px(1.0))
                                .border_color(variables.border)
                                .children(pinned_items.into_iter().map(
                                    |(id, name, cover, item_type)| {
                                        pinned_item(id, name, cover, item_type, variables)
                                    },
                                )),
                        )
                    }),
            )
            .child(Title::new("Library", self.hovered))
    }
}
