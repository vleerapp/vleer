use crate::media::playback::play_song_ids_now;
use crate::ui::components::scroller::SmoothScrollable;
use gpui::{Context, IntoElement, Render, prelude::FluentBuilder, *};
use rand::seq::SliceRandom;

use crate::ui::assets::genre_cover_uri;
use crate::{
    data::{db::repo::Database, models::GenreListItem},
    ui::{
        components::{
            card::{
                CARD_GRID_GAP, Card, ViewportWidth, card_columns, card_row_height, card_spacer,
            },
            context_menu::LibraryDataChanged,
            div::{flex_col, flex_row},
            scrollbar::{Scrollbar, ScrollbarAxis, ScrollbarHandle},
        },
        layout::{SidePanel, library::Search},
        variables::Variables,
        views::{ActiveView, AppView},
    },
};

pub struct GenresView {
    genres: Vec<GenreListItem>,
    last_query: Option<String>,
    request_version: u64,
    request_task: Option<Task<()>>,
    container_width: Option<f32>,
    scroll_handle: UniformListScrollHandle,
}

impl GenresView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            genres: Vec::new(),
            last_query: None,
            request_version: 0,
            request_task: None,
            container_width: None,
            scroll_handle: UniformListScrollHandle::default(),
        };

        if cx.global::<ActiveView>().0 == AppView::Genres {
            view.reload(cx);
        }

        cx.observe_global::<Search>(|this, cx| {
            if cx.global::<ActiveView>().0 == AppView::Genres {
                this.reload(cx);
            }
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 == AppView::Genres {
                this.reload(cx);
            }
        })
        .detach();

        cx.observe_global::<LibraryDataChanged>(|this, cx| {
            this.last_query = None;
            this.reload(cx);
        })
        .detach();

        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let query = cx.global::<Search>().query.trim().to_string();
        if self.last_query.as_deref() == Some(query.as_str()) {
            return;
        }

        self.request_version = self.request_version.wrapping_add(1);
        let version = self.request_version;
        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();

        self.request_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let query_for_spawn = query.clone();
            let genres = bg
                .spawn(async move { db.get_genres(&query_for_spawn).unwrap_or_default() })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.request_version != version {
                        return;
                    }
                    this.last_query = Some(query);
                    if this.genres != genres {
                        this.genres = genres;
                        cx.notify();
                    }
                })
            })
            .ok();
        }));
    }
}

fn genre_tile(idx: usize, genre: &GenreListItem) -> impl IntoElement {
    let subtitle = match genre.song_count {
        1 => "1 song".to_string(),
        n => format!("{n} songs"),
    };
    let genre_id = genre.id.clone();

    Card::new(format!("genre-item-{}", idx), genre.name.clone())
        .subtitle(subtitle)
        .image_uri(Some(genre_cover_uri(&genre.id)))
        .on_play(move |_window, cx| {
            let db = cx.global::<Database>().clone();
            let bg = cx.background_executor().clone();
            let genre_id = genre_id.clone();
            cx.spawn(async move |cx| {
                let song_ids = bg
                    .spawn(async move {
                        let mut ids = db.get_genre_song_ids(&genre_id).unwrap_or_default();
                        ids.shuffle(&mut rand::rng());
                        ids
                    })
                    .await;
                cx.update(|cx| play_song_ids_now(song_ids, cx));
            })
            .detach();
        })
}

impl Render for GenresView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let side_panel = cx.global::<SidePanel>();

        let bounds = window.bounds();
        let window_width: f32 = bounds.size.width.into();
        let mut estimated_width = window_width - 300.0 - 98.0;
        if side_panel.is_open() {
            estimated_width -= 316.0;
        }
        if let Some(width) = cx.global::<ViewportWidth>().get() {
            self.container_width = Some((width - variables.padding_24 * 2.0).max(1.0));
        } else if estimated_width > 0.0 {
            self.container_width = Some(estimated_width);
        }

        let items_per_row = card_columns(self.container_width);
        let items_per_row = items_per_row.max(1);
        let row_height =
            card_row_height(self.container_width.unwrap_or(1000.0), items_per_row, true);

        let total_count = self.genres.len();
        let row_count = total_count.div_ceil(items_per_row);
        let view_handle = cx.entity();

        let grid_content = if row_count == 0 {
            flex_row()
                .id("genres-empty")
                .w_full()
                .p(px(variables.padding_24))
                .child("No Results Found")
                .text_color(variables.text_secondary)
                .into_any_element()
        } else {
            let scroll_handle = self.scroll_handle.clone();

            div()
                .size_full()
                .child(
                    uniform_list(
                        ElementId::Name("genres-rows".into()),
                        row_count,
                        move |range, _, cx| {
                            range
                                .map(|row_idx| {
                                    let variables = cx.global::<Variables>();
                                    let mut row = flex_row()
                                        .id(ElementId::Name(
                                            format!("genres-row-{}", row_idx).into(),
                                        ))
                                        .w_full()
                                        .px(px(variables.padding_24))
                                        .gap(px(CARD_GRID_GAP))
                                        .pb(px(CARD_GRID_GAP))
                                        .h(px(row_height));

                                    for col_idx in 0..items_per_row {
                                        let item_idx = row_idx * items_per_row + col_idx;
                                        let Some(genre) =
                                            view_handle.read(cx).genres.get(item_idx).cloned()
                                        else {
                                            break;
                                        };
                                        row = row.child(genre_tile(item_idx, &genre));
                                    }

                                    let filled = total_count
                                        .saturating_sub(row_idx * items_per_row)
                                        .min(items_per_row);
                                    for _ in filled..items_per_row {
                                        row = row.child(card_spacer());
                                    }

                                    row.into_any_element()
                                })
                                .collect()
                        },
                    )
                    .track_scroll(&scroll_handle)
                    .size_full()
                    .pt(px(variables.padding_24))
                    .pb(px(variables.padding_24 - CARD_GRID_GAP)),
                )
                .into_any_element()
        };

        flex_col()
            .size_full()
            .child(
                div()
                    .id("genres-scroll-container")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .relative()
                    .child(div().id("genres-content").size_full().child(grid_content)),
            )
            .when(row_count > 0, |this| {
                let scroll_handle = self.scroll_handle.clone();
                let padding_extra =
                    px(variables.padding_24 + (variables.padding_24 - CARD_GRID_GAP));
                let mut content_size = scroll_handle.content_size();
                content_size.height += padding_extra;
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .child(
                            Scrollbar::new(&scroll_handle)
                                .axis(ScrollbarAxis::Vertical)
                                .scroll_size(content_size),
                        ),
                )
            })
            .smooth_scroll(&self.scroll_handle)
    }
}
