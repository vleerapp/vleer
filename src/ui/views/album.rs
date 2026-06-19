use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::{
        db::repo::Database,
        models::{Album, Cuid},
    },
    media::{
        playback::{Playback, play_album_now},
        queue::Queue,
    },
    ui::{
        components::{
            button::Button,
            context_menu::{
                ContextMenu, LibraryDataChanged, QueueChanged, album_context_menu_items,
            },
            div::{flex_col, flex_row},
            icons,
            song_table::{
                GetRowCountHandler, GetRowHandler, QueueHandler, SongEntry, SongTable,
                SongTableEvent, join_artists,
            },
        },
        variables::Variables,
        views::{ActiveView, AppView, SelectedAlbum},
    },
};

type SongCache = Rc<RefCell<Vec<Arc<SongEntry>>>>;

type ArtistInfo = (String, Option<String>);

pub struct AlbumView {
    album_id: Option<Cuid>,
    album: Option<Album>,
    artist_id: Option<Cuid>,
    artist_name: Option<String>,
    artist_image_id: Option<String>,
    artists_data: Vec<ArtistInfo>,
    year: Option<String>,
    genres: Vec<String>,
    total_duration_secs: i32,
    songs_cache: SongCache,
    load_task: Option<Task<()>>,
    table: Entity<SongTable>,
    context_menu: Entity<ContextMenu>,
}

fn song_entry_from_song(song: &crate::data::models::Song) -> Arc<SongEntry> {
    let artists = if song.artists.is_empty() {
        vec!["Unknown".to_string()]
    } else {
        song.artists.clone()
    };
    let (artist, artist_ranges) = join_artists(&artists);
    let minutes = song.duration / 60;
    let seconds = song.duration % 60;
    Arc::new(SongEntry {
        id: song.id.clone(),
        title: song.title.clone(),
        artist,
        artist_ranges,
        album: String::new(),
        album_id: song.album_id.clone(),
        duration: format!("{}:{:02}", minutes, seconds),
        cover_uri: song.image_id.clone().map(|id| format!("!image://{}", id)),
        track_number: song.track_number,
        genre: String::new(),
    })
}

impl AlbumView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let songs_cache: SongCache = Rc::new(RefCell::new(Vec::new()));

        let get_row_count: GetRowCountHandler = {
            let cache = songs_cache.clone();
            Rc::new(move |_cx, _sort| cache.borrow().len())
        };

        let get_row: GetRowHandler = {
            let cache = songs_cache.clone();
            Rc::new(move |_cx, idx, _sort| cache.borrow().get(idx).cloned())
        };

        let queue_handler: QueueHandler = {
            let cache = songs_cache.clone();
            Rc::new(move |cx, current_id, index, _sort| {
                let rest: Vec<Cuid> = {
                    let cache = cache.borrow();
                    if cache.get(index).map(|e| &e.id) != Some(&current_id) {
                        return;
                    }
                    cache.iter().skip(index + 1).map(|e| e.id.clone()).collect()
                };
                if rest.is_empty() {
                    return;
                }
                cx.update_global::<Queue, _>(|q, _| {
                    q.add_songs(rest);
                });
                cx.set_global(QueueChanged);
            })
        };

        let table = SongTable::new(
            cx,
            get_row_count,
            get_row,
            Some(queue_handler),
            None,
            true,
            false,
            false,
            false,
        );

        let mut view = Self {
            album_id: cx.global::<SelectedAlbum>().0.clone(),
            album: None,
            artist_id: None,
            artist_name: None,
            artist_image_id: None,
            artists_data: Vec::new(),
            year: None,
            genres: Vec::new(),
            total_duration_secs: 0,
            songs_cache,
            load_task: None,
            table,
            context_menu: cx.new(|_| ContextMenu::new()),
        };

        if cx.global::<ActiveView>().0 == AppView::Album {
            view.reload(cx);
        }

        cx.observe_global::<SelectedAlbum>(|this, cx| {
            let new_id = cx.global::<SelectedAlbum>().0.clone();
            if new_id == this.album_id {
                return;
            }
            this.album_id = new_id;
            this.reload(cx);
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Album {
                return;
            }
            let new_id = cx.global::<SelectedAlbum>().0.clone();
            if new_id != this.album_id || this.album.is_none() {
                this.album_id = new_id;
                this.reload(cx);
            }
        })
        .detach();

        cx.observe_global::<LibraryDataChanged>(|this, cx| {
            this.reload(cx);
        })
        .detach();

        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(album_id) = self.album_id.clone() else {
            self.album = None;
            self.artist_id = None;
            self.artist_name = None;
            self.artist_image_id = None;
            self.artists_data = Vec::new();
            self.year = None;
            self.genres = Vec::new();
            self.total_duration_secs = 0;
            self.songs_cache.borrow_mut().clear();
            let table = self.table.clone();
            cx.update_entity(&table, |_table, cx| cx.emit(SongTableEvent::NewRows));
            cx.notify();
            return;
        };

        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let id_for = album_id.clone();
            let (album, artist_id, artist_name, artist_image_id, year, songs, artists_data) = bg
                .spawn(async move {
                    let album = db.get_album(&id_for).ok().flatten();
                    let songs = db.get_album_songs(&id_for).unwrap_or_default();
                    let primary_artist_name = album
                        .as_ref()
                        .and_then(|a| a.artists.first().cloned())
                        .or_else(|| songs.first().and_then(|s| s.artists.first().cloned()));
                    let artist = primary_artist_name
                        .as_deref()
                        .and_then(|name| db.get_artist_by_name(name).ok().flatten());
                    let artist_id = artist.as_ref().map(|a| a.id.clone());
                    let artist_image_id = artist.as_ref().and_then(|a| a.image_id.clone());
                    let artist_name = album
                        .as_ref()
                        .map(|a| {
                            if a.artists.is_empty() {
                                String::new()
                            } else {
                                join_artists(&a.artists).0
                            }
                        })
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            songs
                                .first()
                                .map(|s| {
                                    if s.artists.is_empty() {
                                        String::new()
                                    } else {
                                        join_artists(&s.artists).0
                                    }
                                })
                                .filter(|s| !s.is_empty())
                        });
                    let artist_names_list: Vec<String> = album
                        .as_ref()
                        .map(|a| a.artists.clone())
                        .filter(|a| !a.is_empty())
                        .or_else(|| {
                            songs
                                .first()
                                .map(|s| s.artists.clone())
                                .filter(|a| !a.is_empty())
                        })
                        .unwrap_or_default();
                    let mut artists_data: Vec<ArtistInfo> = Vec::new();
                    for name in &artist_names_list {
                        let img = db
                            .get_artist_by_name(name)
                            .ok()
                            .flatten()
                            .and_then(|a| a.image_id.clone());
                        artists_data.push((name.clone(), img));
                    }
                    let year = songs
                        .iter()
                        .find_map(|s| s.date.clone())
                        .map(|d| d.chars().take(4).collect::<String>())
                        .filter(|y| !y.is_empty());
                    (
                        album,
                        artist_id,
                        artist_name,
                        artist_image_id,
                        year,
                        songs,
                        artists_data,
                    )
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.album_id.as_ref() != Some(&album_id) {
                        return;
                    }
                    this.album = album;
                    this.artist_id = artist_id;
                    this.artist_name = artist_name;
                    this.artist_image_id = artist_image_id;
                    this.artists_data = artists_data;
                    this.year = year;
                    this.total_duration_secs = songs.iter().map(|s| s.duration).sum();
                    {
                        let mut seen = std::collections::BTreeSet::new();
                        for s in &songs {
                            for g in &s.genres {
                                seen.insert(g.clone());
                            }
                        }
                        this.genres = seen.into_iter().collect();
                    }
                    {
                        let mut cache = this.songs_cache.borrow_mut();
                        cache.clear();
                        cache.extend(songs.iter().map(song_entry_from_song));
                    }
                    let table = this.table.clone();
                    cx.update_entity(&table, |_t, cx| cx.emit(SongTableEvent::NewRows));
                    cx.notify();
                })
            })
            .ok();
        });

        self.load_task = Some(task);
    }

    fn total_duration_string(&self) -> String {
        let total = self.total_duration_secs;
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        let seconds = total % 60;
        if hours > 0 {
            format!("{}h {}m", hours, minutes)
        } else {
            format!("{}m {}s", minutes, seconds)
        }
    }
}

impl Render for AlbumView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let context_menu = self.context_menu.clone();

        let body = if let Some(album) = self.album.clone() {
            let cover_size = 220.0_f32;
            let image: AnyElement = match album.image_id.clone() {
                Some(uri) => img(format!("!image://{}", uri))
                    .id("album-cover")
                    .size(px(cover_size))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                None => div()
                    .id("album-cover-placeholder")
                    .size(px(cover_size))
                    .bg(variables.border)
                    .into_any_element(),
            };

            let song_count = self.songs_cache.borrow().len();
            let duration = self.total_duration_string();
            let year = self.year.clone();
            let album_id_play = album.id.clone();
            let album_id_menu = album.id.clone();
            let menu_for_button = context_menu.clone();
            let songs_for_shuffle = self.songs_cache.clone();
            let _artist_id = self.artist_id.clone();

            let genres_str = if self.genres.is_empty() {
                String::new()
            } else {
                format!(" \u{00B7} {}", self.genres.join(", "))
            };
            let meta_line = match year {
                Some(y) => format!(
                    "{} \u{00B7} {} songs \u{00B7} {}{}",
                    y, song_count, duration, genres_str
                ),
                None => format!("{} songs \u{00B7} {}{}", song_count, duration, genres_str),
            };

            let header = flex_col()
                .w_full()
                .flex_shrink_0()
                .gap(px(variables.padding_8))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(18.0))
                        .line_height(px(22.0))
                        .text_ellipsis()
                        .overflow_x_hidden()
                        .w_full()
                        .min_w_0()
                        .child(album.title.clone()),
                )
                .child(div().text_color(variables.text_secondary).child(meta_line))
                .child(
                    flex_row()
                        .gap(px(variables.padding_8))
                        .pt(px(variables.padding_8))
                        .child(
                            Button::new("album-play-button")
                                .icon(icons::PLAY)
                                .items_center()
                                .gap(px(variables.padding_8))
                                .bg_color(variables.accent)
                                .color(variables.background)
                                .hover_color(variables.background)
                                .hover(|s| s.bg(variables.accent_background))
                                .on_click(move |_event, _window, cx| {
                                    play_album_now(album_id_play.clone(), cx);
                                }),
                        )
                        .child(
                            Button::new("album-shuffle-button")
                                .icon(icons::SHUFFLE)
                                .items_center()
                                .gap(px(variables.padding_8))
                                .on_click(move |_event, _window, cx| {
                                    let song_ids: Vec<Cuid> = songs_for_shuffle
                                        .borrow()
                                        .iter()
                                        .map(|e| e.id.clone())
                                        .collect();
                                    if song_ids.is_empty() {
                                        return;
                                    }
                                    cx.update_global::<Queue, _>(|queue, _| {
                                        queue.clear();
                                        queue.add_songs(song_ids);
                                        queue.set_shuffle(true);
                                    });
                                    cx.update_global::<Playback, _>(|playback, cx| {
                                        playback.play_queue(cx);
                                    });
                                    cx.set_global(QueueChanged);
                                }),
                        )
                        .child(
                            Button::new("album-more-button")
                                .icon(icons::DOTS)
                                .items_center()
                                .gap(px(variables.padding_8))
                                .on_click(move |event, _window, cx| {
                                    let items = album_context_menu_items(album_id_menu.clone(), cx);
                                    menu_for_button.update(cx, |menu, cx| {
                                        menu.show(event.position(), items, cx);
                                    });
                                }),
                        ),
                );

            let artists_data = self.artists_data.clone();

            let sidebar =
                flex_col()
                    .w(px(cover_size))
                    .flex_shrink_0()
                    .gap(px(variables.padding_16))
                    .child(image)
                    .children(artists_data.into_iter().enumerate().map(
                        |(i, (name, image_uri))| {
                            let tile_id = format!("album-artist-{}", i);
                            flex_row()
                                .id(ElementId::Name(tile_id.clone().into()))
                                .gap(px(variables.padding_8))
                                .items_center()
                                .child(
                                    div()
                                        .id(ElementId::Name(format!("{}-avatar", tile_id).into()))
                                        .size(px(36.0))
                                        .rounded_full()
                                        .relative()
                                        .overflow_hidden()
                                        .child(match image_uri {
                                            Some(uri) => img(format!("!image://{}", uri))
                                                .size_full()
                                                .rounded_full()
                                                .object_fit(ObjectFit::Cover)
                                                .into_any_element(),
                                            None => div()
                                                .size_full()
                                                .rounded_full()
                                                .bg(variables.border)
                                                .into_any_element(),
                                        }),
                                )
                                .child(
                                    div()
                                        .id(ElementId::Name(format!("{}-name", tile_id).into()))
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .hover(|s| s.underline())
                                        .cursor_pointer()
                                        .child(name),
                                )
                                .into_any_element()
                        },
                    ));

            flex_row()
                .size_full()
                .p(px(variables.padding_24))
                .child(
                    flex_col()
                        .size_full()
                        .gap(px(variables.padding_24))
                        .child(header)
                        .child(self.table.clone()),
                )
                .gap(px(variables.padding_24))
                .items_start()
                .child(sidebar)
                .into_any_element()
        } else {
            flex_row()
                .id("album-loading")
                .w_full()
                .p(px(variables.padding_24))
                .text_color(variables.text_secondary)
                .child("Loading...")
                .into_any_element()
        };

        flex_col().size_full().child(
            div()
                .id("album-scroll-container")
                .flex_1()
                .size_full()
                .min_h_0()
                .relative()
                .child(div().id("album-content").size_full().child(body))
                .child(self.context_menu.clone()),
        )
    }
}
