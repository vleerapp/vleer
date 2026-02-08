use crate::{
    data::{db::repo::Database, models::RecentItem},
    ui::{
        assets::image_cache::vleer_cache,
        components::{
            div::{flex_col, flex_row},
            icons::{
                icon::icon,
                icons::{ARROW_LEFT, ARROW_RIGHT},
            },
        },
        variables::Variables,
    },
};
use gpui::{prelude::FluentBuilder, *};

pub struct HomeView {
    recently_added: Vec<RecentItem>,
    recently_added_offset: usize,
    container_width: Option<f32>,
}

const MIN_COVER_SIZE: f32 = 180.0;
const MAX_COVER_SIZE: f32 = 220.0;
const GAP_SIZE: f32 = 16.0;

impl HomeView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            recently_added: Vec::new(),
            recently_added_offset: 0,
            container_width: None,
        };

        view.load_recently_added(window, cx);
        view
    }

    fn load_recently_added(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = cx.global::<Database>().clone();

        cx.spawn_in(
            window,
            |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
                let mut cx = cx.clone();
                async move {
                    let items = db.get_recently_added_items(100).await.unwrap_or_default();

                    this.update(&mut cx, |this, cx| {
                        this.recently_added = items;
                        cx.notify();
                    })
                    .ok();
                }
            },
        )
        .detach();
    }

    fn calculate_layout(&self) -> (f32, usize) {
        let width = self.container_width.unwrap_or(1000.0);

        let num_items = ((width + GAP_SIZE) / (MIN_COVER_SIZE + GAP_SIZE)).floor() as usize;
        let num_items = num_items.max(1);

        let cover_size = if num_items > 0 {
            ((width - (num_items - 1) as f32 * GAP_SIZE) / num_items as f32)
                .clamp(MIN_COVER_SIZE, MAX_COVER_SIZE)
        } else {
            MIN_COVER_SIZE
        };

        (cover_size, num_items)
    }

    fn scroll_recently_added_left(&mut self, cx: &mut Context<Self>) {
        let (_, items_per_page) = self.calculate_layout();
        if self.recently_added_offset >= items_per_page {
            self.recently_added_offset -= items_per_page;
        } else {
            self.recently_added_offset = 0;
        }
        cx.notify();
    }

    fn scroll_recently_added_right(&mut self, cx: &mut Context<Self>) {
        let (_, items_per_page) = self.calculate_layout();
        let max_offset = self.recently_added.len().saturating_sub(items_per_page);
        if self.recently_added_offset + items_per_page <= max_offset {
            self.recently_added_offset += items_per_page;
        } else {
            self.recently_added_offset = max_offset;
        }
        cx.notify();
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        let bounds = window.bounds();
        let window_width: f32 = bounds.size.width.into();
        let estimated_width = window_width - 300.0 - 98.0;
        if estimated_width > 0.0 {
            self.container_width = Some(estimated_width);
        }

        let (cover_size, items_per_page) = self.calculate_layout();

        let can_scroll_left = self.recently_added_offset > 0;
        let can_scroll_right =
            self.recently_added_offset + items_per_page < self.recently_added.len();

        let recently_added_content = if self.recently_added.is_empty() {
            flex_row()
                .id("recently-added-empty")
                .w_full()
                .child("No Data")
                .text_color(variables.text_secondary)
                .into_any_element()
        } else {
            let visible_items: Vec<_> = self
                .recently_added
                .iter()
                .skip(self.recently_added_offset)
                .take(items_per_page)
                .enumerate()
                .collect();

            flex_row()
                .id("recently-added-grid")
                .w_full()
                .gap(px(GAP_SIZE))
                .children(visible_items.into_iter().map(|(idx, item)| {
                    let (title, subtitle, cover_uri) = match item {
                        RecentItem::Song {
                            title,
                            artist_name,
                            image_id,
                        } => {
                            let artist = artist_name
                                .clone()
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            (title.clone(), artist, image_id.clone())
                        }
                        RecentItem::Album {
                            title,
                            artist_name,
                            image_id,
                            year,
                        } => {
                            let artist = artist_name
                                .clone()
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            let subtitle = if let Some(y) = year {
                                format!("{} · {}", y, artist)
                            } else {
                                artist
                            };
                            (title.clone(), subtitle, image_id.clone())
                        }
                    };

                    let cover_element = if let Some(uri) = cover_uri {
                        img(format!("!image://{}", uri))
                            .id(ElementId::Name(format!("recent-cover-{}", idx).into()))
                            .size(px(cover_size))
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    } else {
                        div()
                            .id(ElementId::Name(
                                format!("recent-cover-placeholder-{}", idx).into(),
                            ))
                            .size(px(cover_size))
                            .bg(variables.border)
                            .into_any_element()
                    };

                    flex_col()
                        .id(ElementId::Name(format!("recent-item-{}", idx).into()))
                        .w(px(cover_size))
                        .gap(px(8.0))
                        .child(cover_element)
                        .child(
                            flex_col()
                                .id(ElementId::Name(format!("recent-item-info-{}", idx).into()))
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .id(ElementId::Name(format!("recent-title-{}", idx).into()))
                                        .text_ellipsis()
                                        .font_weight(FontWeight(500.0))
                                        .overflow_x_hidden()
                                        .max_w(px(cover_size))
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .id(ElementId::Name(
                                            format!("recent-subtitle-{}", idx).into(),
                                        ))
                                        .text_ellipsis()
                                        .overflow_x_hidden()
                                        .max_w(px(cover_size))
                                        .text_color(variables.text_secondary)
                                        .child(subtitle),
                                ),
                        )
                }))
                .into_any_element()
        };

        let recently_played = flex_col()
            .id("recently-played-section")
            .w_full()
            .child(
                flex_row()
                    .w_full()
                    .id("recently-played-header")
                    .child(
                        div()
                            .id("recently-played-title")
                            .gap(px(variables.padding_16))
                            .child("Recently Played")
                            .font_weight(FontWeight(600.0))
                            .text_size(px(18.0)),
                    )
                    .child(
                        flex_row()
                            .id("recently-played-arrows")
                            .gap(px(variables.padding_8))
                            .child(icon(ARROW_LEFT))
                            .child(icon(ARROW_RIGHT)),
                    )
                    .items_center()
                    .justify_between(),
            )
            .child(
                flex_row()
                    .id("recently-played-content")
                    .child("No Data")
                    .text_color(variables.text_secondary),
            )
            .gap(px(variables.padding_16));

        let left_arrow_color = if can_scroll_left {
            variables.text_secondary
        } else {
            variables.text_muted
        };

        let right_arrow_color = if can_scroll_right {
            variables.text_secondary
        } else {
            variables.text_muted
        };

        let recently_added = flex_col()
            .id("recently-added-section")
            .w_full()
            .child(
                flex_row()
                    .id("recently-added-header")
                    .child(
                        div()
                            .id("recently-added-title")
                            .gap(px(variables.padding_16))
                            .child("Recently Added")
                            .font_weight(FontWeight(600.0))
                            .text_size(px(18.0)),
                    )
                    .child(
                        flex_row()
                            .id("recently-added-arrows")
                            .gap(px(variables.padding_8))
                            .child(
                                icon(ARROW_LEFT)
                                    .when(can_scroll_left, |this| this.cursor_pointer())
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            this.scroll_recently_added_left(cx);
                                        }),
                                    )
                                    .text_color(left_arrow_color)
                                    .hover(|this| {
                                        if can_scroll_left {
                                            this.text_color(variables.text)
                                        } else {
                                            this
                                        }
                                    }),
                            )
                            .child(
                                icon(ARROW_RIGHT)
                                    .when(can_scroll_right, |this| this.cursor_pointer())
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            this.scroll_recently_added_right(cx);
                                        }),
                                    )
                                    .text_color(right_arrow_color)
                                    .hover(|this| {
                                        if can_scroll_right {
                                            this.text_color(variables.text)
                                        } else {
                                            this
                                        }
                                    }),
                            ),
                    )
                    .items_center()
                    .justify_between(),
            )
            .child(recently_added_content)
            .gap(px(variables.padding_16));

        div()
            .image_cache(vleer_cache("home-image-cache", 20))
            .id("home-container")
            .flex()
            .flex_col()
            .size_full()
            .p(px(variables.padding_24))
            .child(
                div()
                    .id("home-scroll-container")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        flex_col()
                            .id("home-content")
                            .gap(px(variables.padding_24))
                            .child(
                                flex_row()
                                    .id("home-welcome")
                                    .w_full()
                                    .text_color(variables.accent)
                                    .child(div().h(px(100.0)).child(
                                        r"
                __
 _      _____  / /________  ____ ___  ___
| | /| / / _ \/ / ___/ __ \/ __ `__ \/ _ \
| |/ |/ /  __/ / /__/ /_/ / / / / / /  __/
|__/|__/\___/_/\___/\____/_/ /_/ /_/\___/ ",
                                    )),
                            )
                            .child(recently_played)
                            .child(recently_added),
                    ),
            )
    }
}
