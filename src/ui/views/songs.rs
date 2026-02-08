use gpui::*;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::{db::repo::Database, models::Cuid},
    ui::{
        assets::image_cache::vleer_cache,
        components::{
            div::flex_col,
            song_table::{
                GetRowHandler, GetRowsHandler, SongColumn, SongEntry, SongTable, SongTableEvent,
                TableSort,
            },
        },
        layout::library::Search,
        variables::Variables,
    },
};

fn get_rows(cx: &mut App, sort: Option<TableSort>) -> Vec<Cuid> {
    let db = cx.global::<Database>().clone();
    let search_query = cx.global::<Search>().query.to_string().to_lowercase();

    let all_songs = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(db.get_songs_paged(0, i64::MAX))
            .unwrap_or_default()
    });

    let mut songs: Vec<_> = all_songs
        .into_iter()
        .filter(|s| {
            if search_query.is_empty() {
                return true;
            }

            let title_match = s.title.to_lowercase().contains(&search_query);

            let artist_match = s
                .artist_id
                .as_ref()
                .and_then(|id| {
                    let id = id.clone();
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(db.get_artist(id))
                            .ok()
                            .flatten()
                    })
                })
                .map_or(false, |a| a.name.to_lowercase().contains(&search_query));

            let album_match = s
                .album_id
                .as_ref()
                .and_then(|id| {
                    let id = id.clone();
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(db.get_album(id))
                            .ok()
                            .flatten()
                    })
                })
                .map_or(false, |a| a.title.to_lowercase().contains(&search_query));

            title_match || artist_match || album_match
        })
        .collect();

    if let Some(sort) = sort {
        match sort.column {
            SongColumn::Number => {
                if !sort.ascending {
                    songs.reverse();
                }
            }
            SongColumn::Title => {
                songs.sort_by(|a, b| {
                    let a = a.title.to_lowercase();
                    let b = b.title.to_lowercase();
                    if sort.ascending { a.cmp(&b) } else { b.cmp(&a) }
                });
            }
            SongColumn::Album => {
                songs.sort_by(|a, b| {
                    let album_a = a
                        .album_id
                        .as_ref()
                        .and_then(|id| {
                            let id = id.clone();
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current()
                                    .block_on(db.get_album(id))
                                    .ok()
                                    .flatten()
                            })
                        })
                        .map(|a| a.title)
                        .unwrap_or_default();

                    let album_b = b
                        .album_id
                        .as_ref()
                        .and_then(|id| {
                            let id = id.clone();
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current()
                                    .block_on(db.get_album(id))
                                    .ok()
                                    .flatten()
                            })
                        })
                        .map(|a| a.title)
                        .unwrap_or_default();

                    if sort.ascending {
                        album_a.to_lowercase().cmp(&album_b.to_lowercase())
                    } else {
                        album_b.to_lowercase().cmp(&album_a.to_lowercase())
                    }
                });
            }
            SongColumn::Duration => {
                songs.sort_by(|a, b| {
                    if sort.ascending {
                        a.duration.cmp(&b.duration)
                    } else {
                        b.duration.cmp(&a.duration)
                    }
                });
            }
        }
    }

    songs.into_iter().map(|s| s.id).collect()
}

fn get_row(cx: &mut App, id: Cuid) -> Option<Arc<SongEntry>> {
    let db = cx.global::<Database>().clone();

    let song = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current()
            .block_on(db.get_song(id))
            .ok()
            .flatten()
    })?;

    let artist = song
        .artist_id
        .and_then(|id| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(db.get_artist(id))
                    .ok()
                    .flatten()
            })
        })
        .map(|a| a.name)
        .unwrap_or_else(|| "Unknown".into());

    let album = song
        .album_id
        .and_then(|id| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(db.get_album(id))
                    .ok()
                    .flatten()
            })
        })
        .map(|a| a.title)
        .unwrap_or_else(|| "Unknown".into());

    let minutes = song.duration / 60;
    let seconds = song.duration % 60;

    Some(Arc::new(SongEntry {
        id: song.id,
        number: 0,
        title: song.title,
        artist,
        album,
        duration: format!("{}:{:02}", minutes, seconds),
        cover_uri: song.image_id.map(|id| format!("!image://{}", id)),
    }))
}

pub struct SongsView {
    table: Entity<SongTable>,
    last_query: String,
}

impl SongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let get_rows_handler: GetRowsHandler = Rc::new(|cx, sort| get_rows(cx, sort));
        let get_row_handler: GetRowHandler = Rc::new(|cx, id| get_row(cx, id));

        let table = SongTable::new(cx, get_rows_handler, get_row_handler, None);

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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        flex_col()
            .image_cache(vleer_cache("songs-image-cache", 20))
            .id("songs-border")
            .size_full()
            .p(px(variables.padding_24))
            .child(self.table.clone())
    }
}
