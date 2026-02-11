use gpui::{Context, IntoElement, Render, prelude::FluentBuilder, *};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    data::{db::repo::Database, models::AlbumListItem},
    ui::{
        assets::image_cache::app_image_cache,
        components::{
            div::{flex_col, flex_row},
            scrollbar::{Scrollbar, ScrollbarAxis},
        },
        layout::library::Search,
        variables::Variables,
    },
};

const MIN_COVER_SIZE: f32 = 180.0;
const MAX_COVER_SIZE: f32 = 400.0;
const GAP_SIZE: f32 = 16.0;

pub struct AlbumsView {
    page_size: usize,
    total_count: usize,
    page_cache: FxHashMap<usize, Vec<AlbumListItem>>,
    page_pending: FxHashSet<usize>,
    last_query: String,
    container_width: Option<f32>,
    scroll_handle: UniformListScrollHandle,
}

impl AlbumsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            page_size: 60,
            total_count: 0,
            page_cache: FxHashMap::default(),
            page_pending: FxHashSet::default(),
            last_query: String::new(),
            container_width: None,
            scroll_handle: UniformListScrollHandle::default(),
        };

        view.refresh_query(cx);

        cx.observe_global::<Search>(|this, cx| {
            let query = cx.global::<Search>().query.to_string();
            if query == this.last_query {
                return;
            }
            this.last_query = query;
            this.refresh_query(cx);
        })
        .detach();

        view
    }

    fn refresh_query(&mut self, cx: &mut Context<Self>) {
        self.page_cache.clear();
        self.page_pending.clear();
        self.total_count = 0;
        cx.notify();

        let db = cx.global::<Database>().clone();
        let query = self.last_query.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let count = db.get_albums_count_filtered(&query).await.unwrap_or(0);

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.last_query != query {
                        return;
                    }
                    this.total_count = count;
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn ensure_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.page_cache.contains_key(&page) || self.page_pending.contains(&page) {
            return;
        }

        self.page_pending.insert(page);

        let db = cx.global::<Database>().clone();
        let query = self.last_query.clone();
        let page_size = self.page_size;
        let offset = (page * page_size) as i64;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let albums = db
                .get_albums_paged_filtered(&query, offset, page_size as i64)
                .await
                .unwrap_or_default();

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.last_query != query {
                        return;
                    }
                    this.page_cache.insert(page, albums);
                    this.page_pending.remove(&page);
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }

    fn ensure_pages_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        items_per_row: usize,
        cx: &mut Context<Self>,
    ) {
        if self.total_count == 0 {
            return;
        }

        let start_item = range.start.saturating_mul(items_per_row);
        let end_item = (range.end.saturating_mul(items_per_row)).min(self.total_count);
        if start_item >= end_item {
            return;
        }

        let page_start = start_item / self.page_size;
        let page_end = (end_item - 1) / self.page_size;
        let buffer = 1usize;

        let begin = page_start.saturating_sub(buffer);
        let end = page_end.saturating_add(buffer);

        for page in begin..=end {
            self.ensure_page(page, cx);
        }
    }

    fn get_album_at(&self, index: usize) -> Option<AlbumListItem> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        self.page_cache
            .get(&page)
            .and_then(|p| p.get(offset))
            .cloned()
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
}

fn album_tile(
    idx: usize,
    album: &AlbumListItem,
    cover_size: f32,
    variables: &Variables,
) -> impl IntoElement {
    let artist = album
        .artist_name
        .clone()
        .unwrap_or_else(|| "Unknown Artist".to_string());
    let subtitle = if let Some(year) = &album.year {
        format!("{} · {}", year, artist)
    } else {
        artist
    };

    let cover_element = if let Some(uri) = &album.image_id {
        img(format!("!image://{}", uri))
            .id(ElementId::Name(format!("album-cover-{}", idx).into()))
            .size(px(cover_size))
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    } else {
        div()
            .id(ElementId::Name(
                format!("album-cover-placeholder-{}", idx).into(),
            ))
            .size(px(cover_size))
            .bg(variables.border)
            .into_any_element()
    };

    flex_col()
        .id(ElementId::Name(format!("album-item-{}", idx).into()))
        .w(px(cover_size))
        .gap(px(8.0))
        .child(cover_element)
        .child(
            flex_col()
                .id(ElementId::Name(format!("album-item-info-{}", idx).into()))
                .gap(px(4.0))
                .child(
                    div()
                        .id(ElementId::Name(format!("album-title-{}", idx).into()))
                        .text_ellipsis()
                        .font_weight(FontWeight(500.0))
                        .overflow_x_hidden()
                        .max_w(px(cover_size))
                        .child(album.title.clone()),
                )
                .child(
                    div()
                        .id(ElementId::Name(format!("album-subtitle-{}", idx).into()))
                        .text_ellipsis()
                        .overflow_x_hidden()
                        .max_w(px(cover_size))
                        .text_color(variables.text_secondary)
                        .child(subtitle),
                ),
        )
}

impl Render for AlbumsView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        let bounds = window.bounds();
        let window_width: f32 = bounds.size.width.into();
        let estimated_width = window_width - 300.0 - 98.0;
        if estimated_width > 0.0 {
            self.container_width = Some(estimated_width);
        }

        let (cover_size, items_per_row) = self.calculate_layout();
        let items_per_row = items_per_row.max(1);

        let row_count = if self.total_count == 0 {
            0
        } else {
            (self.total_count + items_per_row - 1) / items_per_row
        };

        let view_handle = cx.entity();

        let grid_content = if row_count == 0 {
            flex_row()
                .id("albums-empty")
                .w_full()
                .child("No Results Found")
                .text_color(variables.text_secondary)
                .into_any_element()
        } else {
            let scroll_handle = self.scroll_handle.clone();

            div()
                .size_full()
                .child(
                    uniform_list(
                        ElementId::Name("albums-rows".into()),
                        row_count,
                        move |range, _, cx| {
                            view_handle.update(cx, |this, cx| {
                                this.ensure_pages_for_range(range.clone(), items_per_row, cx);
                            });

                            range
                                .map(|row_idx| {
                                    let variables = cx.global::<Variables>();
                                    let mut row = flex_row()
                                        .id(ElementId::Name(
                                            format!("albums-row-{}", row_idx).into(),
                                        ))
                                        .w_full()
                                        .gap(px(GAP_SIZE))
                                        .pb(px(GAP_SIZE));

                                    for col_idx in 0..items_per_row {
                                        let item_idx = row_idx * items_per_row + col_idx;
                                        if item_idx >= view_handle.read(cx).total_count {
                                            break;
                                        }

                                        let album_opt = view_handle.read(cx).get_album_at(item_idx);

                                        if let Some(album) = album_opt {
                                            row = row.child(album_tile(
                                                item_idx, &album, cover_size, variables,
                                            ));
                                        } else {
                                            row = row.child(
                                                div()
                                                    .id(ElementId::Name(
                                                        format!("album-placeholder-{}", item_idx)
                                                            .into(),
                                                    ))
                                                    .w(px(cover_size))
                                                    .h(px(cover_size + 44.0))
                                                    .bg(variables.border),
                                            );
                                        }
                                    }

                                    row.into_any_element()
                                })
                                .collect()
                        },
                    )
                    .track_scroll(&scroll_handle)
                    .size_full(),
                )
                .into_any_element()
        };

        flex_col()
            .image_cache(app_image_cache())
            .size_full()
            .child(
                div()
                    .id("albums-scroll-container")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .relative()
                    .child(
                        div()
                            .id("albums-content")
                            .size_full()
                            .p(px(variables.padding_24))
                            .child(grid_content),
                    ),
            )
            .when(row_count > 0, |this| {
                let scroll_handle = self.scroll_handle.clone();
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .child(Scrollbar::new(&scroll_handle).axis(ScrollbarAxis::Vertical)),
                )
            })
    }
}
