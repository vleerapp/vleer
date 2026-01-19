use gpui::*;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::{state::State, types::Cuid},
    ui::{
        components::{
            div::flex_col,
            song_table::{SongColumn, SongEntry, SongTable, SongTableEvent, TableSort},
            title::Title,
        },
        variables::Variables,
        views::HoverableView,
    },
};

fn get_rows(cx: &mut App, sort: Option<TableSort>) -> Vec<Cuid> {
    let state = cx.global::<State>().clone();
    let search_query = state.get_search_query_sync().to_lowercase();
    let all_songs = state.get_all_songs_sync();

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
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(state.get_artist(id))
                    })
                })
                .map_or(false, |a| a.name.to_lowercase().contains(&search_query));

            let album_match = s
                .album_id
                .as_ref()
                .and_then(|id| {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(state.get_album(id))
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
                    if sort.ascending {
                        a.title.to_lowercase().cmp(&b.title.to_lowercase())
                    } else {
                        b.title.to_lowercase().cmp(&a.title.to_lowercase())
                    }
                });
            }
            SongColumn::Album => {
                songs.sort_by(|a, b| {
                    let album_a = a
                        .album_id
                        .as_ref()
                        .and_then(|id| {
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(state.get_album(id))
                            })
                        })
                        .map(|a| a.title.clone())
                        .unwrap_or_default();

                    let album_b = b
                        .album_id
                        .as_ref()
                        .and_then(|id| {
                            tokio::task::block_in_place(|| {
                                tokio::runtime::Handle::current().block_on(state.get_album(id))
                            })
                        })
                        .map(|a| a.title.clone())
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

    songs.iter().map(|s| s.id.clone()).collect()
}

fn get_row(cx: &mut App, id: Cuid) -> Option<Arc<SongEntry>> {
    let state = cx.global::<State>().clone();

    let song = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(state.get_song(&id))
    });

    if let Some(song) = song {
        let artist = song
            .artist_id
            .as_ref()
            .and_then(|id| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(state.get_artist(id))
                })
            })
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let album = song
            .album_id
            .as_ref()
            .and_then(|id| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(state.get_album(id))
                })
            })
            .map(|a| a.title.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let cover_uri = song.cover_uri();

        let minutes = song.duration / 60;
        let seconds = song.duration % 60;
        let duration_str = format!("{}:{:02}", minutes, seconds);

        Some(Arc::new(SongEntry {
            id: song.id.clone(),
            number: 0,
            title: song.title.clone(),
            artist,
            album,
            duration: duration_str,
            cover_uri,
        }))
    } else {
        None
    }
}

pub struct SongsView {
    pub hovered: bool,
    table: Entity<SongTable>,
    last_query: String,
}

impl SongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let get_rows_handler = Rc::new(get_rows);
        let get_row_handler = Rc::new(get_row);

        let table = SongTable::new(cx, get_rows_handler, get_row_handler, None);

        cx.observe_global::<State>(|this, cx| {
            let q = cx.global::<State>().get_search_query_sync();
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
            hovered: false,
            table,
            last_query: String::new(),
        }
    }
}

impl Render for SongsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        div()
            .id("songs-view")
            .relative()
            .size_full()
            .child(
                div()
                    .id("songs-container")
                    .size_full()
                    .overflow_hidden()
                    .child(
                        flex_col()
                            .id("songs-border")
                            .border(px(1.0))
                            .border_color(border_color)
                            .size_full()
                            .p(px(variables.padding_24))
                            .child(self.table.clone()),
                    ),
            )
            .child(Title::new("Songs", self.hovered))
    }
}

impl HoverableView for SongsView {
    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.hovered = hovered;
        cx.notify();
    }
}
