use gpui::*;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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

    fn get_count(&mut self, db: &Database, query: &str, sort: SongSort, ascending: bool) -> usize {
        self.ensure_state(query, sort, ascending);

        if let Some(count) = self.count {
            return count;
        }

        let count = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(db.get_songs_count_filtered(query))
                .unwrap_or(0)
        });

        self.count = Some(count);
        count
    }

    fn ensure_page(
        &mut self,
        db: &Database,
        query: &str,
        sort: SongSort,
        ascending: bool,
        page: usize,
    ) {
        if self.pages.contains_key(&page) {
            return;
        }

        let offset = (page * self.page_size) as i64;
        let limit = self.page_size as i64;

        let items = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(db.get_songs_paged_filtered(query, sort, ascending, offset, limit))
                .unwrap_or_default()
        });

        let entries = items.into_iter().map(song_entry_from_list_item).collect();
        self.pages.insert(page, entries);
    }

    fn get_row(
        &mut self,
        db: &Database,
        query: &str,
        sort: SongSort,
        ascending: bool,
        index: usize,
    ) -> Option<Arc<SongEntry>> {
        self.ensure_state(query, sort, ascending);

        let page = index / self.page_size;
        let offset = index % self.page_size;
        self.ensure_page(db, query, sort, ascending, page);

        self.pages.get(&page).and_then(|p| p.get(offset)).cloned()
    }
}

pub struct SongsView {
    table: Entity<SongTable>,
    last_query: String,
}

impl SongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cache = Rc::new(RefCell::new(SongPageCache::new(SONG_PAGE_SIZE)));

        let get_row_count: GetRowCountHandler = {
            let cache = cache.clone();
            Rc::new(move |cx, sort| {
                let db = cx.global::<Database>().clone();
                let query = cx.global::<Search>().query.to_string();
                let (sort, ascending) = map_sort(sort);
                cache.borrow_mut().get_count(&db, &query, sort, ascending)
            })
        };

        let get_row_handler: GetRowHandler = {
            let cache = cache.clone();
            Rc::new(move |cx, index, sort| {
                let db = cx.global::<Database>().clone();
                let query = cx.global::<Search>().query.to_string();
                let (sort, ascending) = map_sort(sort);
                cache
                    .borrow_mut()
                    .get_row(&db, &query, sort, ascending, index)
            })
        };

        let queue_handler: QueueHandler = Rc::new(move |cx, current_id, index, sort| {
            let db = cx.global::<Database>().clone();
            let query = cx.global::<Search>().query.to_string();
            let (sort, ascending) = map_sort(sort);

            cx.spawn(async move |cx: &mut AsyncApp| {
                let song_ids = db
                    .get_song_ids_from_offset_filtered(&query, sort, ascending, (index + 1) as i64)
                    .await
                    .unwrap_or_default();

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

        cx.observe_global::<Search>(|this, cx| {
            let q = cx.global::<Search>().query.to_string();
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

        Self {
            table,
            last_query: String::new(),
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
