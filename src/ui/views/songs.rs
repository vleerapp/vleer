use gpui::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
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
const SONG_QUERY_DEBOUNCE: Duration = Duration::from_millis(180);

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
        number: 0,
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
    pending_pages: FxHashSet<usize>,
    count: Option<usize>,
    count_pending: bool,
    last_query: String,
    last_sort: SongSort,
    last_ascending: bool,
    state_version: u64,
}

impl SongPageCache {
    fn new(page_size: usize) -> Self {
        Self {
            page_size,
            pages: FxHashMap::default(),
            pending_pages: FxHashSet::default(),
            count: None,
            count_pending: false,
            last_query: String::new(),
            last_sort: SongSort::Default,
            last_ascending: false,
            state_version: 0,
        }
    }

    fn ensure_state(&mut self, query: &str, sort: SongSort, ascending: bool) {
        if self.last_query != query || self.last_sort != sort || self.last_ascending != ascending {
            self.last_query = query.to_string();
            self.last_sort = sort;
            self.last_ascending = ascending;
            self.pages.clear();
            self.pending_pages.clear();
            self.count = None;
            self.count_pending = false;
            self.state_version = self.state_version.wrapping_add(1);
        }
    }

    fn get_count(&mut self, query: &str, sort: SongSort, ascending: bool) -> usize {
        self.ensure_state(query, sort, ascending);
        self.count.unwrap_or(0)
    }

    fn state_version(&self) -> u64 {
        self.state_version
    }

    fn should_fetch_count(&mut self) -> bool {
        if self.count.is_some() || self.count_pending {
            return false;
        }
        self.count_pending = true;
        true
    }

    fn set_count(&mut self, state_version: u64, count: usize) -> bool {
        if self.state_version != state_version {
            return false;
        }
        self.count = Some(count);
        self.count_pending = false;
        true
    }

    fn clear_count_pending(&mut self, state_version: u64) {
        if self.state_version != state_version {
            return;
        }
        self.count_pending = false;
    }

    fn get_row(
        &mut self,
        query: &str,
        sort: SongSort,
        ascending: bool,
        index: usize,
    ) -> Option<Arc<SongEntry>> {
        self.ensure_state(query, sort, ascending);

        let page = index / self.page_size;
        let offset = index % self.page_size;

        self.pages.get(&page).and_then(|p| p.get(offset)).cloned()
    }

    fn should_fetch_page(&mut self, page: usize) -> bool {
        if self.pages.contains_key(&page) || self.pending_pages.contains(&page) {
            return false;
        }
        self.pending_pages.insert(page);
        true
    }

    fn page_fetch_params(&self, page: usize) -> (i64, i64) {
        let offset = (page * self.page_size) as i64;
        let limit = self.page_size as i64;
        (offset, limit)
    }

    fn set_page(&mut self, state_version: u64, page: usize, entries: Vec<Arc<SongEntry>>) -> bool {
        if self.state_version != state_version {
            return false;
        }
        self.pending_pages.remove(&page);
        self.pages.insert(page, entries);
        true
    }

    fn clear_page_pending(&mut self, state_version: u64, page: usize) {
        if self.state_version != state_version {
            return;
        }
        self.pending_pages.remove(&page);
    }
}

pub struct SongsView {
    table: Entity<SongTable>,
    last_query: String,
    refresh_task: Option<Task<()>>,
}

impl SongsView {
    fn schedule_table_refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_task = None;
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(SONG_QUERY_DEBOUNCE).await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if cx.global::<ActiveView>().0 != AppView::Songs {
                        return;
                    }

                    let current_query = cx.global::<Search>().query.trim().to_string();
                    if current_query != this.last_query {
                        return;
                    }

                    let table_handle = this.table.clone();
                    cx.update_entity(&table_handle, |_table, cx| {
                        cx.emit(SongTableEvent::NewRows);
                    });
                })
            })
            .ok();
        });
        self.refresh_task = Some(task);
    }

    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_query = cx.global::<Search>().query.trim().to_string();
        let cache = Arc::new(Mutex::new(SongPageCache::new(SONG_PAGE_SIZE)));
        let table_ref: Arc<Mutex<Option<Entity<SongTable>>>> = Arc::new(Mutex::new(None));

        let get_row_count: GetRowCountHandler = {
            let cache = cache.clone();
            let table_ref = table_ref.clone();
            Rc::new(move |cx, sort| {
                let db = cx.global::<Database>().clone();
                let query = cx.global::<Search>().query.trim().to_string();
                let (sort, ascending) = map_sort(sort);

                let (count, should_fetch_count, should_prefetch_page, state_version, offset, limit) = {
                    let mut cache = cache.lock().expect("song cache lock poisoned");
                    let count = cache.get_count(&query, sort, ascending);
                    let should_fetch_count = cache.should_fetch_count();
                    let should_prefetch_page = cache.should_fetch_page(0);
                    let state_version = cache.state_version();
                    let (offset, limit) = cache.page_fetch_params(0);
                    (
                        count,
                        should_fetch_count,
                        should_prefetch_page,
                        state_version,
                        offset,
                        limit,
                    )
                };

                if should_prefetch_page {
                    let cache = cache.clone();
                    let table_ref = table_ref.clone();
                    let db = db.clone();
                    let query = query.clone();
                    cx.spawn(async move |cx: &mut AsyncApp| {
                        let items = match crate::RUNTIME
                            .spawn(async move {
                                db.get_songs_paged_filtered(&query, sort, ascending, offset, limit)
                                    .await
                            })
                            .await
                        {
                            Ok(Ok(items)) => items,
                            Ok(Err(err)) => {
                                error!("Failed to fetch songs page 0: {}", err);
                                let mut cache = cache.lock().expect("song cache lock poisoned");
                                cache.clear_page_pending(state_version, 0);
                                return;
                            }
                            Err(err) => {
                                error!("Songs page 0 task failed: {}", err);
                                let mut cache = cache.lock().expect("song cache lock poisoned");
                                cache.clear_page_pending(state_version, 0);
                                return;
                            }
                        };
                        let entries = items.into_iter().map(song_entry_from_list_item).collect();

                        let should_notify = {
                            let mut cache = cache.lock().expect("song cache lock poisoned");
                            cache.set_page(state_version, 0, entries)
                        };

                        if should_notify {
                            let table = table_ref.lock().ok().and_then(|guard| (*guard).clone());
                            if let Some(table) = table {
                                cx.update(|cx| {
                                    cx.update_entity(&table, |_table, cx| {
                                        cx.emit(SongTableEvent::NewRows);
                                    });
                                });
                            }
                        }
                    })
                    .detach();
                }

                if should_fetch_count {
                    let cache = cache.clone();
                    let table_ref = table_ref.clone();
                    let db = db.clone();
                    cx.spawn(async move |cx: &mut AsyncApp| {
                        let count = match crate::RUNTIME
                            .spawn(async move { db.get_songs_count_filtered(&query).await })
                            .await
                        {
                            Ok(Ok(count)) => count,
                            Ok(Err(err)) => {
                                error!("Failed to fetch songs count: {}", err);
                                let mut cache = cache.lock().expect("song cache lock poisoned");
                                cache.clear_count_pending(state_version);
                                return;
                            }
                            Err(err) => {
                                error!("Songs count task failed: {}", err);
                                let mut cache = cache.lock().expect("song cache lock poisoned");
                                cache.clear_count_pending(state_version);
                                return;
                            }
                        };
                        let should_notify = {
                            let mut cache = cache.lock().expect("song cache lock poisoned");
                            cache.set_count(state_version, count)
                        };

                        if should_notify {
                            let table = table_ref.lock().ok().and_then(|guard| (*guard).clone());
                            if let Some(table) = table {
                                cx.update(|cx| {
                                    cx.update_entity(&table, |_table, cx| {
                                        cx.emit(SongTableEvent::NewRows);
                                    });
                                });
                            }
                        }
                    })
                    .detach();
                }

                count
            })
        };

        let get_row_handler: GetRowHandler = {
            let cache = cache.clone();
            let table_ref = table_ref.clone();
            Rc::new(move |cx, index, sort| {
                let db = cx.global::<Database>().clone();
                let query = cx.global::<Search>().query.trim().to_string();
                let (sort, ascending) = map_sort(sort);
                let page = index / SONG_PAGE_SIZE;

                let (row, should_fetch, state_version, offset, limit) = {
                    let mut cache = cache.lock().expect("song cache lock poisoned");
                    let row = cache.get_row(&query, sort, ascending, index);
                    let should_fetch = cache.should_fetch_page(page);
                    let state_version = cache.state_version();
                    let (offset, limit) = cache.page_fetch_params(page);
                    (row, should_fetch, state_version, offset, limit)
                };

                if should_fetch {
                    let cache = cache.clone();
                    let table_ref = table_ref.clone();
                    cx.spawn(async move |cx: &mut AsyncApp| {
                        let items = match crate::RUNTIME
                            .spawn(async move {
                                db.get_songs_paged_filtered(&query, sort, ascending, offset, limit)
                                    .await
                            })
                            .await
                        {
                            Ok(Ok(items)) => items,
                            Ok(Err(err)) => {
                                error!("Failed to fetch songs page {}: {}", page, err);
                                let mut cache = cache.lock().expect("song cache lock poisoned");
                                cache.clear_page_pending(state_version, page);
                                return;
                            }
                            Err(err) => {
                                error!("Songs page {} task failed: {}", page, err);
                                let mut cache = cache.lock().expect("song cache lock poisoned");
                                cache.clear_page_pending(state_version, page);
                                return;
                            }
                        };
                        let entries = items.into_iter().map(song_entry_from_list_item).collect();

                        let should_notify = {
                            let mut cache = cache.lock().expect("song cache lock poisoned");
                            cache.set_page(state_version, page, entries)
                        };

                        if should_notify {
                            let table = table_ref.lock().ok().and_then(|guard| (*guard).clone());
                            if let Some(table) = table {
                                cx.update(|cx| {
                                    cx.update_entity(&table, |_table, cx| {
                                        cx.emit(SongTableEvent::NewRows);
                                    });
                                });
                            }
                        }
                    })
                    .detach();
                }

                row
            })
        };

        let queue_handler: QueueHandler = Rc::new(move |cx, current_id, index, sort| {
            let db = cx.global::<Database>().clone();
            let query = cx.global::<Search>().query.trim().to_string();
            let (sort, ascending) = map_sort(sort);

            cx.spawn(async move |cx: &mut AsyncApp| {
                let song_ids = match crate::RUNTIME
                    .spawn(async move {
                        db.get_song_ids_from_offset_filtered(
                            &query,
                            sort,
                            ascending,
                            (index + 1) as i64,
                        )
                        .await
                    })
                    .await
                {
                    Ok(Ok(song_ids)) => song_ids,
                    Ok(Err(err)) => {
                        error!("Failed to fetch queued song ids: {}", err);
                        return;
                    }
                    Err(err) => {
                        error!("Queued song ids task failed: {}", err);
                        return;
                    }
                };

                if song_ids.is_empty() {
                    return;
                }

                cx.update(|cx| {
                    let matches = cx
                        .global::<Queue>()
                        .get_current_song_id()
                        .map_or(false, |id| id == current_id);

                    if !matches {
                        return;
                    }

                    cx.update_global::<Queue, _>(|queue, _cx| {
                        queue.add_songs(song_ids);
                    });
                });
            })
            .detach();
        });

        let table = SongTable::new(
            cx,
            get_row_count,
            get_row_handler,
            Some(queue_handler),
            None,
        );
        if let Ok(mut guard) = table_ref.lock() {
            *guard = Some(table.clone());
        }

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
            this.schedule_table_refresh(cx);
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
            refresh_task: None,
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
