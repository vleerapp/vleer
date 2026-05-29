use gpui::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::{
        db::repo::Database,
        models::{Album, Cuid},
    },
    media::{playback::Playback, queue::Queue},
    ui::{
        components::{
            context_menu::{
                ContextMenu, LibraryDataChanged, QueueChanged, album_context_menu_items,
                play_album_now,
            },
            div::{flex_col, flex_row},
            icons::{self, icon},
            song_table::{
                GetRowCountHandler, GetRowHandler, QueueHandler, SongEntry, SongTable,
                SongTableEvent,
            },
        },
        variables::Variables,
        views::{ActiveView, AppView, SelectedAlbum},
    },
};

type SongCache = Rc<RefCell<Vec<Arc<SongEntry>>>>;

pub struct AlbumView {
    album_id: Option<Cuid>,
    album: Option<Album>,
    artist_id: Option<Cuid>,
    artist_name: Option<String>,
    artist_image_id: Option<String>,
    year: Option<String>,
    total_duration_secs: i32,
    songs_cache: SongCache,
    load_task: Option<Task<()>>,
    table: Entity<SongTable>,
    context_menu: Entity<ContextMenu>,
}

fn song_entry_from_song(song: &crate::data::models::Song) -> Arc<SongEntry> {
    let artist = song
        .artist_name
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let minutes = song.duration / 60;
    let seconds = song.duration % 60;
    Arc::new(SongEntry {
        id: song.id.clone(),
        title: song.title.clone(),
        artist,
        album: String::new(),
        album_id: song.album_id.clone(),
        duration: format!("{}:{:02}", minutes, seconds),
        cover_uri: song.image_id.clone().map(|id| format!("!image://{}", id)),
        track_number: song.track_number,
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
        );

        let mut view = Self {
            album_id: cx.global::<SelectedAlbum>().0.clone(),
            album: None,
            artist_id: None,
            artist_name: None,
            artist_image_id: None,
            year: None,
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
            self.year = None;
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
            let (album, artist_id, artist_name, artist_image_id, year, songs) = bg
                .spawn(async move {
                    let album = db.get_album(&id_for).ok().flatten();
                    let songs = db.get_album_songs(&id_for).unwrap_or_default();
                    let artist = album
                        .as_ref()
                        .and_then(|a| a.artist_id.as_ref())
                        .and_then(|aid| db.get_artist(aid).ok().flatten());
                    let artist_id = artist.as_ref().map(|a| a.id.clone());
                    let artist_image_id = artist.as_ref().and_then(|a| a.image_id.clone());
                    let artist_name = artist
                        .map(|a| a.name)
                        .or_else(|| songs.first().and_then(|s| s.artist_name.clone()));
                    let year = songs
                        .iter()
                        .find_map(|s| s.date.clone())
                        .map(|d| d.chars().take(4).collect::<String>())
                        .filter(|y| !y.is_empty());
                    (album, artist_id, artist_name, artist_image_id, year, songs)
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
                    this.year = year;
                    this.total_duration_secs = songs.iter().map(|s| s.duration).sum();
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

            let artist = self
                .artist_name
                .clone()
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let song_count = self.songs_cache.borrow().len();
            let duration = self.total_duration_string();
            let year = self.year.clone();
            let album_id_play = album.id.clone();
            let album_id_menu = album.id.clone();
            let menu_for_button = context_menu.clone();
            let songs_for_shuffle = self.songs_cache.clone();
            let artist_image_id = self.artist_image_id.clone();
            let _artist_id = self.artist_id.clone();

            let meta_line = match year {
                Some(y) => format!("{} \u{00B7} {} songs \u{00B7} {}", y, song_count, duration),
                None => format!("{} songs \u{00B7} {}", song_count, duration),
            };

            let header = flex_col()
                .w_full()
                .flex_shrink_0()
                .gap(px(variables.padding_8))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(18.0))
                        .text_ellipsis()
                        .overflow_x_hidden()
                        .child(album.title.clone()),
                )
                .child(div().text_color(variables.text_secondary).child(meta_line))
                .child(
                    flex_row()
                        .gap(px(variables.padding_8))
                        .pt(px(variables.padding_8))
                        .child(
                            flex_row()
                                .id("album-play-button")
                                .items_center()
                                .gap(px(variables.padding_8))
                                .px(px(variables.padding_8))
                                .py(px(variables.padding_8))
                                .bg(variables.accent)
                                .text_color(variables.background)
                                .cursor_pointer()
                                .hover(|s| s.bg(variables.accent_background))
                                .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                                    play_album_now(album_id_play.clone(), cx);
                                })
                                .child(
                                    icon(icons::PLAY)
                                        .size(px(variables.padding_16))
                                        .text_color(variables.background),
                                ),
                        )
                        .child(
                            flex_row()
                                .id("album-shuffle-button")
                                .items_center()
                                .gap(px(variables.padding_8))
                                .px(px(variables.padding_8))
                                .py(px(variables.padding_8))
                                .cursor_pointer()
                                .hover(|s| s.text_color(variables.text))
                                .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
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
                                })
                                .child(icon(icons::SHUFFLE).size(px(variables.padding_16))),
                        )
                        .child(
                            flex_row()
                                .id("album-more-button")
                                .items_center()
                                .gap(px(variables.padding_8))
                                .px(px(variables.padding_8))
                                .py(px(variables.padding_8))
                                .cursor_pointer()
                                .hover(|s| s.bg(variables.element_hover))
                                .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                                    let items = album_context_menu_items(album_id_menu.clone(), cx);
                                    menu_for_button.update(cx, |menu, cx| {
                                        menu.show(event.position, items, cx);
                                    });
                                })
                                .child(icon(icons::DOTS).size(px(variables.padding_16))),
                        ),
                );

            let sidebar = flex_col()
                .w(px(cover_size))
                .flex_shrink_0()
                .gap(px(variables.padding_24))
                .child(image)
                .child(
                    flex_row()
                        .gap(px(variables.padding_8))
                        .items_center()
                        .child(
                            div()
                                .size(px(36.0))
                                .rounded_full()
                                .relative()
                                .overflow_hidden()
                                .child(match artist_image_id {
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
                                .flex_1()
                                .min_w_0()
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .font_weight(FontWeight(500.0))
                                .child(artist.clone()),
                        ),
                );

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
