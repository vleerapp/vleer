use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::ui::{
    components::{
        div::flex_col,
        icons::icons,
        input::{InputEvent, TextInput},
        nav_button::NavButton,
        title::Title,
    },
    state::State,
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
    name: String,
    image_hash: Option<String>,
    item_type: String,
    variables: &Variables,
) -> impl IntoElement {
    let is_artist = item_type == "Artist";

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
        .items_center()
        .bg(variables.element)
        .gap(px(variables.padding_8))
        .pr(px(variables.padding_8))
        .child(
            div()
                .size(px(36.0))
                .map(|this| if is_artist { this.rounded_full() } else { this })
                .child(cover_element),
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
                                    |(name, cover, item_type)| {
                                        pinned_item(name, cover, item_type, variables)
                                    },
                                )),
                        )
                    }),
            )
            .child(Title::new("Library", self.hovered))
    }
}
