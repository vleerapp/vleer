use crate::media::playback::play_playlist_now;
use gpui::{Context, IntoElement, Render, prelude::FluentBuilder, *};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    data::{db::repo::Database, models::PlaylistListItem},
    ui::{
        app::MainWindow,
        components::{
            card::{CARD_GRID_GAP, Card, calculate_card_layout},
            context_menu::{ContextMenu, LibraryDataChanged, playlist_context_menu_items},
            div::{flex_col, flex_row},
            scrollbar::{Scrollbar, ScrollbarAxis, ScrollbarHandle},
        },
        layout::{library::Search, queue::QueueVisible},
        variables::Variables,
        views::{ActiveView, AppView, SelectedPlaylist},
    },
};

pub struct PlaylistsView {
    page_size: usize,
    total_count: usize,
    page_cache: FxHashMap<usize, Vec<PlaylistListItem>>,
    page_pending: FxHashSet<(u64, usize)>,
    last_query: String,
    query_version: u64,
    pending_query: Option<String>,
    request_version: u64,
    request_task: Option<Task<()>>,
    request_inflight: bool,
    container_width: Option<f32>,
    scroll_handle: UniformListScrollHandle,
    context_menu: Entity<ContextMenu>,
}

impl PlaylistsView {
    fn start_next_query_request(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.pending_query.clone() else {
            return;
        };

        self.request_inflight = true;
        let request_version = self.request_version;
        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();
        let page_size = self.page_size as i64;

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let query_for_spawn = query.clone();
            let (count, first_page) = bg
                .spawn(async move {
                    let count =
                        db.get_playlists_count(&query_for_spawn).unwrap_or(0).max(0) as usize;
                    let first_page = if count > 0 {
                        db.get_playlists(&query_for_spawn, 0, page_size)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    (count, first_page)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.request_version != request_version
                        || this.pending_query.as_deref() != Some(query.as_str())
                    {
                        this.request_inflight = false;
                        if this.pending_query.is_some() {
                            this.start_next_query_request(cx);
                        }
                        return;
                    }

                    let data_changed = this.last_query != query
                        || this.total_count != count
                        || this
                            .page_cache
                            .get(&0)
                            .map_or(!first_page.is_empty(), |existing| existing != &first_page);

                    this.query_version = this.query_version.wrapping_add(1);
                    this.last_query = query;
                    this.page_cache.clear();
                    this.page_pending.clear();
                    this.total_count = count;
                    if count > 0 {
                        this.page_cache.insert(0, first_page);
                    }
                    this.pending_query = None;
                    this.request_inflight = false;

                    if data_changed {
                        cx.notify();
                    }

                    if this.pending_query.is_some() {
                        this.start_next_query_request(cx);
                    }
                })
            })
            .ok();
        });
        self.request_task = Some(task);
    }

    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_query = cx.global::<Search>().query.trim().to_string();
        let mut view = Self {
            page_size: 60,
            total_count: 0,
            page_cache: FxHashMap::default(),
            page_pending: FxHashSet::default(),
            last_query: initial_query.clone(),
            query_version: 0,
            pending_query: None,
            request_version: 0,
            request_task: None,
            request_inflight: false,
            container_width: None,
            scroll_handle: UniformListScrollHandle::default(),
            context_menu: cx.new(|_| ContextMenu::new()),
        };

        if cx.global::<ActiveView>().0 == AppView::Playlists {
            view.request_query(initial_query, cx);
        }

        cx.observe_global::<Search>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Playlists {
                return;
            }
            let query = cx.global::<Search>().query.trim().to_string();
            this.request_query(query, cx);
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Playlists {
                return;
            }
            let query = cx.global::<Search>().query.trim().to_string();
            this.request_query(query, cx);
        })
        .detach();

        cx.observe_global::<LibraryDataChanged>(|this, cx| {
            this.page_cache.clear();
            this.page_pending.clear();
            this.pending_query = None;
            this.query_version = this.query_version.wrapping_add(1);
            let query = this.last_query.clone();
            this.request_query(query, cx);
        })
        .detach();

        view
    }

    fn request_query(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.last_query && self.pending_query.is_none() && !self.page_cache.is_empty() {
            return;
        }
        if self.pending_query.as_deref() == Some(query.as_str()) {
            return;
        }

        self.pending_query = Some(query.clone());
        self.request_version = self.request_version.wrapping_add(1);
        if !self.request_inflight {
            self.start_next_query_request(cx);
        }
    }

    fn ensure_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let pending_key = (self.query_version, page);
        if self.page_cache.contains_key(&page) || self.page_pending.contains(&pending_key) {
            return;
        }

        self.page_pending.insert(pending_key);

        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();
        let query = self.last_query.clone();
        let query_version = self.query_version;
        let page_size = self.page_size;
        let offset = (page * page_size) as i64;

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let query_for_spawn = query.clone();
            let playlists = bg
                .spawn(async move {
                    db.get_playlists(&query_for_spawn, offset, page_size as i64)
                        .unwrap_or_default()
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.last_query != query || this.query_version != query_version {
                        this.page_pending.remove(&(query_version, page));
                        return;
                    }
                    this.page_cache.insert(page, playlists);
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

    fn get_playlist_at(&self, index: usize) -> Option<PlaylistListItem> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        self.page_cache
            .get(&page)
            .and_then(|p| p.get(offset))
            .cloned()
    }

    fn calculate_layout(&self) -> (f32, usize) {
        calculate_card_layout(self.container_width)
    }
}

fn playlist_tile(
    idx: usize,
    playlist: &PlaylistListItem,
    cover_size: f32,
    context_menu: Entity<ContextMenu>,
) -> impl IntoElement {
    let subtitle = format!(
        "{} song{}",
        playlist.song_count,
        if playlist.song_count == 1 { "" } else { "s" }
    );

    let playlist_id = playlist.id.clone();
    let play_id = playlist_id.clone();
    let nav_id = playlist_id.clone();

    Card::new(
        format!("playlist-item-{}", idx),
        playlist.name.clone(),
        cover_size,
    )
    .subtitle(subtitle)
    .image_uri(playlist.image_id.clone())
    .on_play(move |_window, cx| {
        play_playlist_now(play_id.clone(), cx);
    })
    .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
        cx.update_global::<SelectedPlaylist, _>(|sel, _| {
            sel.id = Some(nav_id.clone());
            sel.focus_title = false;
        });
        if let Some(Some(root)) = window.root::<MainWindow>() {
            root.update(cx, |view, cx| {
                view.set_current_view(AppView::Playlist, window, cx);
            });
        }
    })
    .on_mouse_down(MouseButton::Right, move |event, _window, cx| {
        let items = playlist_context_menu_items(playlist_id.clone(), cx);
        context_menu.update(cx, |menu, cx| {
            menu.show(event.position, items, cx);
        });
    })
}

impl Render for PlaylistsView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let queue_visible = cx.global::<QueueVisible>();

        let bounds = window.bounds();
        let window_width: f32 = bounds.size.width.into();
        let mut estimated_width = window_width - 300.0 - 98.0;
        if queue_visible.0 {
            estimated_width -= 316.0;
        }
        if estimated_width > 0.0 {
            self.container_width = Some(estimated_width);
        }

        let (cover_size, items_per_row) = self.calculate_layout();
        let items_per_row = items_per_row.max(1);

        let row_count = if self.total_count == 0 {
            0
        } else {
            self.total_count.div_ceil(items_per_row)
        };

        let view_handle = cx.entity();
        let context_menu = self.context_menu.clone();

        let grid_content = if row_count == 0 {
            flex_row()
                .id("playlists-empty")
                .w_full()
                .p(px(variables.padding_24))
                .child("No Playlists Found")
                .text_color(variables.text_secondary)
                .into_any_element()
        } else {
            let scroll_handle = self.scroll_handle.clone();

            div()
                .size_full()
                .child(
                    uniform_list(
                        ElementId::Name("playlists-rows".into()),
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
                                            format!("playlists-row-{}", row_idx).into(),
                                        ))
                                        .w_full()
                                        .px(px(variables.padding_24))
                                        .gap(px(CARD_GRID_GAP))
                                        .pb(px(CARD_GRID_GAP));

                                    for col_idx in 0..items_per_row {
                                        let item_idx = row_idx * items_per_row + col_idx;
                                        if item_idx >= view_handle.read(cx).total_count {
                                            break;
                                        }

                                        let playlist_opt =
                                            view_handle.read(cx).get_playlist_at(item_idx);

                                        if let Some(playlist) = playlist_opt {
                                            row = row.child(playlist_tile(
                                                item_idx,
                                                &playlist,
                                                cover_size,
                                                context_menu.clone(),
                                            ));
                                        } else {
                                            row = row.child(
                                                div()
                                                    .id(ElementId::Name(
                                                        format!(
                                                            "playlist-placeholder-{}",
                                                            item_idx
                                                        )
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
                    .id("playlists-scroll-container")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .relative()
                    .child(
                        div()
                            .id("playlists-content")
                            .size_full()
                            .child(grid_content),
                    )
                    .child(self.context_menu.clone()),
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
    }
}
