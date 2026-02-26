use gpui::*;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tracing::error;

use crate::{
    data::{
        db::repo::Database,
        models::{SongListItem, SongSort},
    },
    media::queue::Queue,
    ui::{
        assets::image_cache::app_image_cache,
        components::{
            div::flex_col,
            song_table::{
                GetRowCountHandler, GetRowHandler, QueueHandler, SongColumn, SongEntry, SongTable,
                SongTableEvent, TableSort,
            },
        },
        layout::library::Search,
        views::{ActiveView, AppView},
    },
};

const SONG_PAGE_SIZE: usize = 100;

fn run_sync<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        crate::RUNTIME.block_on(future)
    }
}

fn map_sort(sort: Option<TableSort>) -> (SongSort, bool) {
    match sort {
        Some(TableSort {
            column: SongColumn::Title,
            ascending,
        }) => (SongSort::Title, ascending),
        Some(TableSort {
            column: SongColumn::Album,
            ascending,
        }) => (SongSort::Album, ascending),
        Some(TableSort {
            column: SongColumn::Duration,
            ascending,
        }) => (SongSort::Duration, ascending),
        _ => (SongSort::Default, false),
    }
}

fn song_entry_from_list_item(item: SongListItem) -> Arc<SongEntry> {
    let artist = item.artist_name.unwrap_or_else(|| "Unknown".to_string());
    let album = item.album_title.unwrap_or_else(|| "Unknown".to_string());
    let minutes = item.duration / 60;
    let seconds = item.duration % 60;

    Arc::new(SongEntry {
        id: item.id,
        title: item.title,
        artist,
        album,
        duration: format!("{}:{:02}", minutes, seconds),
        cover_uri: item.image_id.map(|id| format!("!image://{}", id)),
    })
}

struct SongPageCache {
    page_size: usize,
    pages: FxHashMap<usize, Vec<Arc<SongEntry>>>,
    count: Option<usize>,
    last_query: String,
    last_sort: SongSort,
    last_ascending: bool,
}

impl SongPageCache {
    fn new(page_size: usize) -> Self {
        Self {
            page_size,
            pages: FxHashMap::default(),
            count: None,
            last_query: String::new(),
            last_sort: SongSort::Default,
            last_ascending: false,
        }
    }

    fn ensure_state(&mut self, query: &str, sort: SongSort, ascending: bool) {
        if self.last_query != query || self.last_sort != sort || self.last_ascending != ascending {
            self.last_query = query.to_string();
            self.last_sort = sort;
            self.last_ascending = ascending;

            self.pages.clear();
            self.count = None;
        }
    }

    fn count(&self) -> usize {
        self.count.unwrap_or(0)
    }

    fn needs_count_fetch(&self) -> bool {
        self.count.is_none()
    }

    fn get_row(&self, index: usize) -> Option<Arc<SongEntry>> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        self.pages.get(&page).and_then(|p| p.get(offset)).cloned()
    }

    fn has_page(&self, page: usize) -> bool {
        self.pages.contains_key(&page)
    }

    fn set_count(&mut self, count: usize) {
        self.count = Some(count);
    }

    fn set_page(&mut self, page: usize, entries: Vec<Arc<SongEntry>>) {
        self.pages.insert(page, entries);
    }
}

pub struct SongsView {
    table: Entity<SongTable>,
    last_query: String,
}

impl SongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_query = cx.global::<Search>().query.trim().to_string();
        let cache = Rc::new(RefCell::new(SongPageCache::new(SONG_PAGE_SIZE)));

        let get_row_count: GetRowCountHandler = {
            let cache = cache.clone();
            Rc::new(move |cx, sort| {
                let db = cx.global::<Database>().clone();
                let query = cx.global::<Search>().query.trim().to_string();
                let (sort, ascending) = map_sort(sort);

                let needs_count_fetch = {
                    let mut cache_ref = cache.borrow_mut();
                    cache_ref.ensure_state(&query, sort, ascending);
                    cache_ref.needs_count_fetch()
                };

                if needs_count_fetch {
                    let count = match run_sync(db.get_songs_count(Some(&query))) {
                        Ok(count) => count as usize,
                        Err(e) => {
                            error!("songs count query failed: {}", e);
                            0
                        }
                    };
                    cache.borrow_mut().set_count(count);
                }

                let count = cache.borrow().count();
                if count > 0 && !cache.borrow().has_page(0) {
                    let items = match run_sync(db.get_songs(
                        Some(&query),
                        sort,
                        ascending,
                        0,
                        SONG_PAGE_SIZE as i64,
                    )) {
                        Ok(items) => items,
                        Err(e) => {
                            error!("songs bootstrap page query failed: {}", e);
                            Vec::new()
                        }
                    };
                    let entries = items.into_iter().map(song_entry_from_list_item).collect();
                    cache.borrow_mut().set_page(0, entries);
                }

                cache.borrow().count()
            })
        };

        let get_row_handler: GetRowHandler = {
            let cache = cache.clone();
            Rc::new(move |cx, index, sort| {
                let db = cx.global::<Database>().clone();
                let query = cx.global::<Search>().query.trim().to_string();
                let (sort, ascending) = map_sort(sort);

                let page = index / SONG_PAGE_SIZE;
                let needs_page_fetch = {
                    let mut cache_ref = cache.borrow_mut();
                    cache_ref.ensure_state(&query, sort, ascending);
                    !cache_ref.has_page(page)
                };

                if needs_page_fetch {
                    let offset = (page * SONG_PAGE_SIZE) as i64;
                    let limit = SONG_PAGE_SIZE as i64;
                    let items = match run_sync(db.get_songs(
                        Some(&query),
                        sort,
                        ascending,
                        offset,
                        limit,
                    )) {
                        Ok(items) => items,
                        Err(e) => {
                            error!("songs page query failed: {}", e);
                            Vec::new()
                        }
                    };
                    let entries = items.into_iter().map(song_entry_from_list_item).collect();
                    cache.borrow_mut().set_page(page, entries);
                }

                cache.borrow().get_row(index)
            })
        };

        let queue_handler: QueueHandler = Rc::new(move |cx, current_id, index, sort| {
            let db = cx.global::<Database>().clone();
            let query = cx.global::<Search>().query.trim().to_string();
            let (sort, ascending) = map_sort(sort);

            let song_ids = match run_sync(db.get_song_ids_from_offset(
                &query,
                sort,
                ascending,
                (index + 1) as i64,
            )) {
                Ok(ids) => ids,
                Err(e) => {
                    error!("queued song ids: {}", e);
                    return;
                }
            };

            if song_ids.is_empty() {
                return;
            }

            let matches = cx
                .global::<Queue>()
                .get_current_song_id()
                .is_some_and(|id| id == current_id);

            if !matches {
                return;
            }

            cx.update_global::<Queue, _>(|queue, _cx| {
                queue.add_songs(song_ids);
            });
        });

        let table = SongTable::new(
            cx,
            get_row_count,
            get_row_handler,
            Some(queue_handler),
            None,
            false,
        );

        if cx.global::<ActiveView>().0 == AppView::Songs {
            let table_handle = table.clone();
            cx.update_entity(&table_handle, |_table, cx| {
                cx.emit(SongTableEvent::NewRows);
            });
        }

        cx.observe_global::<Search>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Songs {
                return;
            }

            let q = cx.global::<Search>().query.trim().to_string();
            if q == this.last_query {
                return;
            }

            this.last_query = q;
            let table_handle = this.table.clone();
            cx.update_entity(&table_handle, |_table, cx| {
                cx.emit(SongTableEvent::NewRows);
            });
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Songs {
                return;
            }

            let q = cx.global::<Search>().query.trim().to_string();
            if q != this.last_query {
                this.last_query = q;
            }

            let table_handle = this.table.clone();
            cx.update_entity(&table_handle, |_table, cx| {
                cx.emit(SongTableEvent::NewRows);
            });
        })
        .detach();

        Self {
            table,
            last_query: initial_query,
        }
    }
}

impl Render for SongsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        flex_col()
            .image_cache(app_image_cache())
            .id("songs-border")
            .size_full()
            .child(self.table.clone())
    }
}
