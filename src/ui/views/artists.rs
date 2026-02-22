use gpui::{Context, IntoElement, Render, prelude::FluentBuilder, *};
use rustc_hash::{FxHashMap, FxHashSet};
use std::time::Duration;

use crate::{
    data::{db::repo::Database, models::ArtistListItem},
    ui::{
        assets::image_cache::app_image_cache,
        components::{
            div::{flex_col, flex_row},
            scrollbar::{Scrollbar, ScrollbarAxis},
        },
        layout::library::Search,
        variables::Variables,
        views::{ActiveView, AppView},
    },
};

const MIN_COVER_SIZE: f32 = 180.0;
const MAX_COVER_SIZE: f32 = 400.0;
const GAP_SIZE: f32 = 16.0;
const ARTIST_QUERY_DEBOUNCE: Duration = Duration::from_millis(180);

pub struct ArtistsView {
    page_size: usize,
    total_count: usize,
    page_cache: FxHashMap<usize, Vec<ArtistListItem>>,
    page_pending: FxHashSet<(u64, usize)>,
    last_query: String,
    query_version: u64,
    container_width: Option<f32>,
    scroll_handle: UniformListScrollHandle,
    refresh_task: Option<Task<()>>,
}

impl ArtistsView {
    fn schedule_query_refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_task = None;
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(ARTIST_QUERY_DEBOUNCE).await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if cx.global::<ActiveView>().0 != AppView::Artists {
                        return;
                    }

                    let current_query = cx.global::<Search>().query.trim().to_string();
                    if current_query != this.last_query {
                        return;
                    }

                    this.refresh_query(cx);
                })
            })
            .ok();
        });
        self.refresh_task = Some(task);
    }

    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_query = cx.global::<Search>().query.trim().to_string();
        let mut view = Self {
            page_size: 60,
            total_count: 0,
            page_cache: FxHashMap::default(),
            page_pending: FxHashSet::default(),
            last_query: initial_query,
            query_version: 0,
            container_width: None,
            scroll_handle: UniformListScrollHandle::default(),
            refresh_task: None,
        };

        if cx.global::<ActiveView>().0 == AppView::Artists {
            view.refresh_query(cx);
        }

        cx.observe_global::<Search>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Artists {
                return;
            }

            let query = cx.global::<Search>().query.trim().to_string();
            if query == this.last_query {
                return;
            }
            this.last_query = query;
            this.schedule_query_refresh(cx);
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Artists {
                return;
            }

            let query = cx.global::<Search>().query.trim().to_string();
            if query == this.last_query && !this.page_cache.is_empty() {
                return;
            }
            this.last_query = query;
            this.refresh_query(cx);
        })
        .detach();

        view
    }

    fn refresh_query(&mut self, cx: &mut Context<Self>) {
        self.query_version = self.query_version.wrapping_add(1);
        self.page_cache.clear();
        self.page_pending.clear();
        self.total_count = 0;
        cx.notify();

        let db = cx.global::<Database>().clone();
        let query = self.last_query.clone();
        let query_version = self.query_version;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (tx, rx) = std::sync::mpsc::channel();
            let db = db.clone();
            let query_for_spawn = query.clone();
            crate::RUNTIME.spawn(async move {
                let result = db.get_artists_count_filtered(&query_for_spawn).await;
                let _ = tx.send(result);
            });

            let count = rx.recv().ok().and_then(|r| r.ok()).unwrap_or(0);

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.last_query != query || this.query_version != query_version {
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
        let pending_key = (self.query_version, page);
        if self.page_cache.contains_key(&page) || self.page_pending.contains(&pending_key) {
            return;
        }

        self.page_pending.insert(pending_key);

        let db = cx.global::<Database>().clone();
        let query = self.last_query.clone();
        let query_version = self.query_version;
        let page_size = self.page_size;
        let offset = (page * page_size) as i64;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let (tx, rx) = std::sync::mpsc::channel();
            let db = db.clone();
            let query_for_spawn = query.clone();
            crate::RUNTIME.spawn(async move {
                let result = db.get_artists_paged_filtered(&query_for_spawn, offset, page_size as i64).await;
                let _ = tx.send(result);
            });

            let artists = rx.recv().ok().and_then(|r| r.ok()).unwrap_or_default();

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.last_query != query || this.query_version != query_version {
                        this.page_pending.remove(&(query_version, page));
                        return;
                    }
                    this.page_cache.insert(page, artists);
                    this.page_pending.remove(&(query_version, page));
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

    fn get_artist_at(&self, index: usize) -> Option<ArtistListItem> {
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

fn artist_tile(
    idx: usize,
    artist: &ArtistListItem,
    cover_size: f32,
    variables: &Variables,
) -> impl IntoElement {
    let cover_element = if let Some(uri) = &artist.image_id {
        img(format!("!image://{}", uri))
            .id(ElementId::Name(format!("artist-cover-{}", idx).into()))
            .size(px(cover_size))
            .object_fit(ObjectFit::Cover)
            .rounded_full()
            .into_any_element()
    } else {
        div()
            .id(ElementId::Name(
                format!("artist-cover-placeholder-{}", idx).into(),
            ))
            .size(px(cover_size))
            .bg(variables.border)
            .rounded_full()
            .into_any_element()
    };

    flex_col()
        .id(ElementId::Name(format!("artist-item-{}", idx).into()))
        .w(px(cover_size))
        .gap(px(8.0))
        .child(cover_element)
        .child(
            div()
                .id(ElementId::Name(format!("artist-title-{}", idx).into()))
                .text_ellipsis()
                .font_weight(FontWeight(500.0))
                .overflow_x_hidden()
                .max_w(px(cover_size))
                .child(artist.name.clone()),
        )
}

impl Render for ArtistsView {
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
                .id("artists-empty")
                .w_full()
                .child("No Data")
                .text_color(variables.text_secondary)
                .into_any_element()
        } else {
            let scroll_handle = self.scroll_handle.clone();

            div()
                .size_full()
                .child(
                    uniform_list(
                        ElementId::Name("artists-rows".into()),
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
                                            format!("artists-row-{}", row_idx).into(),
                                        ))
                                        .w_full()
                                        .gap(px(GAP_SIZE))
                                        .pb(px(GAP_SIZE));

                                    for col_idx in 0..items_per_row {
                                        let item_idx = row_idx * items_per_row + col_idx;
                                        if item_idx >= view_handle.read(cx).total_count {
                                            break;
                                        }

                                        let artist_opt =
                                            view_handle.read(cx).get_artist_at(item_idx);

                                        if let Some(artist) = artist_opt {
                                            row = row.child(artist_tile(
                                                item_idx, &artist, cover_size, variables,
                                            ));
                                        } else {
                                            row = row.child(
                                                div()
                                                    .id(ElementId::Name(
                                                        format!("artist-placeholder-{}", item_idx)
                                                            .into(),
                                                    ))
                                                    .w(px(cover_size))
                                                    .h(px(cover_size + 44.0))
                                                    .rounded_full()
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
                    .id("artists-scroll-container")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .relative()
                    .child(
                        div()
                            .id("artists-content")
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
