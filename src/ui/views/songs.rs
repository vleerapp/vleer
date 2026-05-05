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
        components::{
            context_menu::{LibraryDataChanged, QueueChanged},
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
    version: u64,
    count_pending: bool,
    in_flight_page: Option<usize>,
    next_wanted_page: Option<usize>,
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
            version: 0,
            count_pending: false,
            in_flight_page: None,
            next_wanted_page: None,
        }
    }

    fn ensure_state(&mut self, query: &str, sort: SongSort, ascending: bool) {
        if self.last_query != query || self.last_sort != sort || self.last_ascending != ascending {
            self.last_query = query.to_string();
            self.last_sort = sort;
            self.last_ascending = ascending;
            self.invalidate();
        }
    }

    fn invalidate(&mut self) {
        self.pages.clear();
        self.count = None;
        self.version = self.version.wrapping_add(1);
        self.count_pending = false;
        self.in_flight_page = None;
        self.next_wanted_page = None;
    }

    fn count(&self) -> usize {
        self.count.unwrap_or(0)
    }

    fn get_row(&self, index: usize) -> Option<Arc<SongEntry>> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        self.pages.get(&page).and_then(|p| p.get(offset)).cloned()
    }

    fn request_page(&mut self, page: usize) -> Option<usize> {
        if self.pages.contains_key(&page) {
            return None;
        }
        if self.in_flight_page == Some(page) {
            return None;
        }
        if self.in_flight_page.is_none() {
            self.in_flight_page = Some(page);
            Some(page)
        } else {
            self.next_wanted_page = Some(page);
            None
        }
    }

    fn complete_page(
        &mut self,
        page: usize,
        entries: Vec<Arc<SongEntry>>,
        version: u64,
    ) -> (bool, Option<usize>) {
        if self.version != version {
            return (false, None);
        }
        self.pages.insert(page, entries);
        if self.in_flight_page == Some(page) {
            self.in_flight_page = None;
        }
        if let Some(p) = self.next_wanted_page {
            if self.pages.contains_key(&p) {
                self.next_wanted_page = None;
            }
        }
        let next = if self.in_flight_page.is_none() {
            self.next_wanted_page.take().inspect(|&p| {
                self.in_flight_page = Some(p);
            })
        } else {
            None
        };
        (true, next)
    }
}

type CacheHandle = Rc<RefCell<SongPageCache>>;
type TableHandle = Rc<RefCell<Option<WeakEntity<SongTable>>>>;

fn spawn_count_fetch(
    cx: &mut App,
    cache: CacheHandle,
    table_weak: TableHandle,
    query: String,
    sort: SongSort,
    ascending: bool,
    version: u64,
) {
    let db = cx.global::<Database>().clone();
    let bg = cx.background_executor().clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        let q = query.clone();
        let count = match bg
            .spawn(async move { crate::RUNTIME.block_on(async { db.get_songs_count(Some(&q)).await }) })
            .await
        {
            Ok(c) => c as usize,
            Err(e) => {
                error!("songs count query failed: {}", e);
                0
            }
        };

        let _ = cx.update(|cx| {
            let table_entity = {
                let mut c = cache.borrow_mut();
                c.count_pending = false;
                if c.version == version
                    && c.last_query == query
                    && c.last_sort == sort
                    && c.last_ascending == ascending
                {
                    c.count = Some(count);
                    table_weak.borrow().as_ref().and_then(|w| w.upgrade())
                } else {
                    None
                }
            };
            if let Some(table) = table_entity {
                table.update(cx, |_, cx| cx.emit(SongTableEvent::NewRows));
            }
        });
    })
    .detach();
}

fn spawn_page_fetch(
    cx: &mut App,
    cache: CacheHandle,
    table_weak: TableHandle,
    query: String,
    sort: SongSort,
    ascending: bool,
    version: u64,
    page: usize,
) {
    let db = cx.global::<Database>().clone();
    let bg = cx.background_executor().clone();
    let offset = (page * SONG_PAGE_SIZE) as i64;
    let limit = SONG_PAGE_SIZE as i64;

    cx.spawn(async move |cx: &mut AsyncApp| {
        let q = query.clone();
        let items = match bg
            .spawn(async move {
                crate::RUNTIME.block_on(async {
                    db.get_songs(Some(&q), sort, ascending, offset, limit).await
                })
            })
            .await
        {
            Ok(items) => items,
            Err(e) => {
                error!("songs page query failed: {}", e);
                Vec::new()
            }
        };
        let entries: Vec<Arc<SongEntry>> =
            items.into_iter().map(song_entry_from_list_item).collect();

        let _ = cx.update(|cx| {
            let (applied, next_page) = cache.borrow_mut().complete_page(page, entries, version);

            if applied {
                if let Some(table) = table_weak.borrow().as_ref().and_then(|w| w.upgrade()) {
                    let start = page * SONG_PAGE_SIZE;
                    let end = start + SONG_PAGE_SIZE;
                    table.update(cx, |_, cx| {
                        cx.emit(SongTableEvent::InvalidateRange(start..end))
                    });
                }
            }

            if let Some(next_page) = next_page {
                spawn_page_fetch(
                    cx,
                    cache.clone(),
                    table_weak.clone(),
                    query.clone(),
                    sort,
                    ascending,
                    version,
                    next_page,
                );
            }
        });
    })
    .detach();
}

pub struct SongsView {
    table: Entity<SongTable>,
    last_query: String,
    cache: CacheHandle,
}

impl SongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_query = cx.global::<Search>().query.trim().to_string();
        let cache: CacheHandle = Rc::new(RefCell::new(SongPageCache::new(SONG_PAGE_SIZE)));
        let table_weak: TableHandle = Rc::new(RefCell::new(None));

        let get_row_count: GetRowCountHandler = {
            let cache = cache.clone();
            let table_weak = table_weak.clone();
            Rc::new(move |cx, sort| {
                let query = cx.global::<Search>().query.trim().to_string();
                let (sort, ascending) = map_sort(sort);

                let version_for_fetch = {
                    let mut c = cache.borrow_mut();
                    c.ensure_state(&query, sort, ascending);
                    if c.count.is_none() && !c.count_pending {
                        c.count_pending = true;
                        Some(c.version)
                    } else {
                        None
                    }
                };

                if let Some(version) = version_for_fetch {
                    spawn_count_fetch(
                        cx,
                        cache.clone(),
                        table_weak.clone(),
                        query,
                        sort,
                        ascending,
                        version,
                    );
                }

                cache.borrow().count()
            })
        };

        let get_row_handler: GetRowHandler = {
            let cache = cache.clone();
            let table_weak = table_weak.clone();
            Rc::new(move |cx, index, sort| {
                let query = cx.global::<Search>().query.trim().to_string();
                let (sort, ascending) = map_sort(sort);
                let page = index / SONG_PAGE_SIZE;

                let dispatch = {
                    let mut c = cache.borrow_mut();
                    c.ensure_state(&query, sort, ascending);
                    c.request_page(page).map(|p| (c.version, p))
                };

                if let Some((version, dispatch_page)) = dispatch {
                    spawn_page_fetch(
                        cx,
                        cache.clone(),
                        table_weak.clone(),
                        query,
                        sort,
                        ascending,
                        version,
                        dispatch_page,
                    );
                }

                cache.borrow().get_row(index)
            })
        };

        let queue_handler: QueueHandler = Rc::new(move |cx, current_id, index, sort| {
            let db = cx.global::<Database>().clone();
            let query = cx.global::<Search>().query.trim().to_string();
            let (sort, ascending) = map_sort(sort);
            let current_id_for_check = current_id.clone();
            let bg = cx.background_executor().clone();

            cx.spawn(async move |cx: &mut AsyncApp| {
                let song_ids = match bg
                    .spawn(async move {
                        crate::RUNTIME.block_on(async {
                            db.get_song_ids_from_offset(&query, sort, ascending, (index + 1) as i64)
                                .await
                        })
                    })
                    .await
                {
                    Ok(ids) => ids,
                    Err(e) => {
                        error!("queued song ids: {}", e);
                        return;
                    }
                };

                if song_ids.is_empty() {
                    return;
                }

                let _ = cx.update(|cx| {
                    let matches = cx
                        .global::<Queue>()
                        .get_current_song_id()
                        .is_some_and(|id| id == current_id_for_check);

                    if !matches {
                        return;
                    }

                    cx.update_global::<Queue, _>(|queue, _cx| {
                        queue.add_songs(song_ids);
                    });
                    cx.set_global(QueueChanged::default());
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
            false,
        );
        *table_weak.borrow_mut() = Some(table.downgrade());

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

        cx.observe_global::<LibraryDataChanged>(|this, cx| {
            this.cache.borrow_mut().invalidate();
            let table_handle = this.table.clone();
            cx.update_entity(&table_handle, |_table, cx| {
                cx.emit(SongTableEvent::NewRows);
            });
        })
        .detach();

        Self {
            table,
            last_query: initial_query,
            cache,
        }
    }
}

impl Render for SongsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        flex_col()
            .id("songs-border")
            .size_full()
            .child(self.table.clone())
    }
}
