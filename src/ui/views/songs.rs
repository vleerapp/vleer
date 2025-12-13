use gpui::*;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::types::Cuid,
    ui::{
        components::{
            div::flex_col,
            song_table::{SongColumn, SongEntry, SongTable, TableSort},
            title::Title,
        },
        state::State,
        variables::Variables,
        views::HoverableView,
    },
};

fn get_rows(cx: &mut App, sort: Option<TableSort>) -> Vec<Cuid> {
    let state = cx.global::<State>().clone();
    let mut song_ids = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(state.get_all_song_ids())
    });

    if sort.is_none() {
        return song_ids;
    }

    let sort = sort.unwrap();

    let mut songs: Vec<_> = song_ids
        .iter()
        .filter_map(|id| {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(state.get_song(id))
            })
        })
        .collect();

    match sort.column {
        SongColumn::Number => {
            if !sort.ascending {
                song_ids.reverse();
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
            song_ids = songs.iter().map(|s| s.id.clone()).collect();
        }
        SongColumn::Album => {
            songs.sort_by(|a, b| {
                let album_a = a
                    .album
                    .as_ref()
                    .map(|a| a.title.clone())
                    .unwrap_or_default();
                let album_b = b
                    .album
                    .as_ref()
                    .map(|a| a.title.clone())
                    .unwrap_or_default();

                if sort.ascending {
                    album_a.to_lowercase().cmp(&album_b.to_lowercase())
                } else {
                    album_b.to_lowercase().cmp(&album_a.to_lowercase())
                }
            });
            song_ids = songs.iter().map(|s| s.id.clone()).collect();
        }

        SongColumn::Duration => {
            songs.sort_by(|a, b| {
                if sort.ascending {
                    a.duration.cmp(&b.duration)
                } else {
                    b.duration.cmp(&a.duration)
                }
            });
            song_ids = songs.iter().map(|s| s.id.clone()).collect();
        }
    }

    song_ids
}

fn get_row(cx: &mut App, id: Cuid) -> Option<Arc<SongEntry>> {
    let state = cx.global::<State>().clone();
    let covers_dir = dirs::data_dir().unwrap().join("vleer").join("covers");

    let song = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(state.get_song(&id))
    });

    if let Some(song) = song {
        let artist = song
            .artist
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let album = song
            .album
            .as_ref()
            .map(|a| a.title.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let cover_uri = song.cover.as_ref().and_then(|cover_hash| {
            let cover_path = covers_dir.join(cover_hash);
            if cover_path.exists() {
                Some(format!("!file://{}", cover_path.to_string_lossy()))
            } else {
                None
            }
        });

        let minutes = song.duration / 60;
        let seconds = song.duration % 60;
        let duration_str = format!("{}:{:02}", minutes, seconds);

        let song_ids = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(state.get_all_song_ids())
        });
        let number = song_ids.iter().position(|sid| sid == &id).unwrap_or(0) + 1;

        Some(Arc::new(SongEntry {
            id: song.id.clone(),
            number,
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
}

impl SongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let get_rows_handler = Rc::new(get_rows);
        let get_row_handler = Rc::new(get_row);

        let table = SongTable::new(cx, get_rows_handler, get_row_handler, None);

        Self {
            hovered: false,
            table,
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
