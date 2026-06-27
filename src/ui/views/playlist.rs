use gpui::*;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::{
        db::repo::Database,
        models::{Cuid, Playlist, PlaylistTrack},
    },
    media::{
        playback::{Playback, play_playlist_now},
        queue::Queue,
    },
    ui::{
        components::{
            button::Button,
            context_menu::{
                ContextMenu, LibraryDataChanged, QueueChanged, playlist_context_menu_items,
            },
            div::{flex_col, flex_row},
            icons,
            input::{InputEvent, TextInput},
            song_table::{
                GetRowCountHandler, GetRowHandler, QueueHandler, SongEntry, SongTable,
                SongTableEvent, join_artists,
            },
        },
        variables::Variables,
        views::{ActiveView, AppView, SelectedPlaylist},
    },
};

type SongCache = Rc<RefCell<Vec<Arc<SongEntry>>>>;

pub struct PlaylistView {
    playlist_id: Option<Cuid>,
    playlist: Option<Playlist>,
    total_duration_secs: i32,
    songs_cache: SongCache,
    load_task: Option<Task<()>>,
    table: Entity<SongTable>,
    title_input: Entity<TextInput>,
    context_menu: Entity<ContextMenu>,
    pending_title_focus: bool,
}

fn song_entry_from_track(track: &PlaylistTrack) -> Arc<SongEntry> {
    let song = &track.song;
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
        album: track.album_title.clone().unwrap_or_default(),
        album_id: song.album_id.clone(),
        duration: format!("{}:{:02}", minutes, seconds),
        cover_uri: song.image_id.clone().map(|id| format!("!image://{}", id)),
        track_number: song.track_number,
        genre: String::new(),
    })
}

impl PlaylistView {
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
            false,
            true,
            true,
            false,
        );

        let title_input = cx.new(|cx| {
            TextInput::new(cx, "Playlist name")
                .with_background(transparent_black())
                .no_padding()
                .with_height(px(22.0))
        });

        let selected = cx.global::<SelectedPlaylist>();
        let initial_id = selected.id.clone();
        let initial_focus = selected.focus_title;

        let mut view = Self {
            playlist_id: initial_id,
            playlist: None,
            total_duration_secs: 0,
            songs_cache,
            load_task: None,
            table,
            title_input: title_input.clone(),
            context_menu: cx.new(|_| ContextMenu::new()),
            pending_title_focus: initial_focus,
        };

        if cx.global::<ActiveView>().0 == AppView::Playlist {
            view.reload(cx);
        }

        cx.subscribe(&title_input, |this, _, event: &InputEvent, cx| {
            let (text, is_submit) = match event {
                InputEvent::Submit(t) => (t.clone(), true),
                InputEvent::Change(t) => (t.clone(), false),
            };
            let name = text.trim().to_string();
            if name.is_empty() {
                return;
            }
            let Some(playlist_id) = this.playlist_id.clone() else {
                return;
            };
            let Some(playlist) = this.playlist.as_ref() else {
                return;
            };
            let image_id = playlist.image_id.clone();
            let description = playlist.description.clone();
            let pinned = playlist.pinned;
            let db = cx.global::<Database>().clone();
            if let Err(e) = db.upsert_playlist(
                &playlist_id,
                &name,
                description.as_deref(),
                image_id.as_deref(),
                pinned,
            ) {
                tracing::error!("Failed to rename playlist: {}", e);
                return;
            }
            if let Some(pl) = this.playlist.as_mut() {
                pl.name = name;
            }
            if is_submit {
                cx.set_global(LibraryDataChanged);
            }
            cx.notify();
        })
        .detach();

        cx.observe_global::<SelectedPlaylist>(|this, cx| {
            let sel = cx.global::<SelectedPlaylist>();
            let new_id = sel.id.clone();
            let focus = sel.focus_title;
            if new_id == this.playlist_id && this.playlist.is_some() {
                return;
            }
            this.playlist_id = new_id;
            this.pending_title_focus = focus;
            this.reload(cx);
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Playlist {
                return;
            }
            let sel = cx.global::<SelectedPlaylist>();
            let new_id = sel.id.clone();
            let focus = sel.focus_title;
            if new_id != this.playlist_id || this.playlist.is_none() {
                this.playlist_id = new_id;
                this.pending_title_focus = focus;
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
        let Some(playlist_id) = self.playlist_id.clone() else {
            self.playlist = None;
            self.total_duration_secs = 0;
            self.songs_cache.borrow_mut().clear();
            let table = self.table.clone();
            cx.update_entity(&table, |_t, cx| cx.emit(SongTableEvent::NewRows));
            cx.notify();
            return;
        };

        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();
        let title_input = self.title_input.clone();

        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let id_for = playlist_id.clone();
            let (playlist, songs) = bg
                .spawn(async move {
                    let playlist = db.get_playlist(&id_for).ok().flatten();
                    let songs = db.get_playlist_songs(&id_for).unwrap_or_default();
                    (playlist, songs)
                })
                .await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.playlist_id.as_ref() != Some(&playlist_id) {
                        return;
                    }
                    let name = playlist
                        .as_ref()
                        .map(|p| p.name.clone())
                        .unwrap_or_default();
                    title_input.update(cx, |inp, cx| inp.set_text(name, cx));
                    this.total_duration_secs = songs.iter().map(|t| t.song.duration).sum();
                    {
                        let mut cache = this.songs_cache.borrow_mut();
                        cache.clear();
                        cache.extend(songs.iter().map(song_entry_from_track));
                    }
                    this.playlist = playlist;
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

impl Render for PlaylistView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let context_menu = self.context_menu.clone();

        if self.pending_title_focus {
            self.pending_title_focus = false;
            let handle = self.title_input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }

        let body = if let Some(playlist) = self.playlist.clone() {
            let cover_size = 220.0_f32;

            let playlist_id_for_cover = playlist.id.clone();
            let playlist_name_for_cover = playlist.name.clone();
            let playlist_desc_for_cover = playlist.description.clone();
            let playlist_pinned_for_cover = playlist.pinned;

            let cover: AnyElement = match playlist.image_id.clone() {
                Some(uri) => div()
                    .id("playlist-cover")
                    .size(px(cover_size))
                    .overflow_hidden()
                    .relative()
                    .cursor_pointer()
                    .child(
                        img(format!("!image://{}", uri))
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(black().opacity(0.4))
                            .invisible()
                            .hover(|s| s.visible())
                            .child(
                                div()
                                    .text_color(variables.text)
                                    .text_sm()
                                    .child("Change image"),
                            ),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        let id = playlist_id_for_cover.clone();
                        let name = playlist_name_for_cover.clone();
                        let desc = playlist_desc_for_cover.clone();
                        let pinned = playlist_pinned_for_cover;
                        open_image_picker(id, name, desc, pinned, window, cx);
                    })
                    .into_any_element(),
                None => div()
                    .id("playlist-cover-placeholder")
                    .size(px(cover_size))
                    .bg(variables.border)
                    .relative()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_color(variables.text_secondary)
                            .text_sm()
                            .child("Add image"),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        let id = playlist.id.clone();
                        let name = playlist.name.clone();
                        let desc = playlist.description.clone();
                        let pinned = playlist.pinned;
                        open_image_picker(id, name, desc, pinned, window, cx);
                    })
                    .into_any_element(),
            };

            let song_count = self.songs_cache.borrow().len();
            let duration = self.total_duration_string();
            let playlist_id_play = self.playlist_id.clone().unwrap_or_default();
            let playlist_id_menu = playlist_id_play.clone();
            let songs_for_shuffle = self.songs_cache.clone();
            let menu_for_button = context_menu.clone();

            let meta_line = format!("{} songs \u{00B7} {}", song_count, duration);

            let title = div()
                .font_weight(FontWeight::BOLD)
                .text_size(px(18.0))
                .text_ellipsis()
                .overflow_x_hidden()
                .w_full()
                .min_w_0()
                .child(self.title_input.clone());

            let header = flex_col()
                .w_full()
                .flex_shrink_0()
                .gap(px(variables.padding_8))
                .child(title)
                .child(div().text_color(variables.text_secondary).child(meta_line))
                .child(
                    flex_row()
                        .gap(px(variables.padding_8))
                        .pt(px(variables.padding_8))
                        .child(
                            Button::new("playlist-play-button")
                                .icon(icons::PLAY)
                                .items_center()
                                .gap(px(variables.padding_8))
                                .bg_color(variables.accent)
                                .color(variables.background)
                                .hover_color(variables.background)
                                .hover(|s| s.bg(variables.accent_background))
                                .on_click(move |_event, _window, cx| {
                                    play_playlist_now(playlist_id_play.clone(), cx);
                                }),
                        )
                        .child(
                            Button::new("playlist-shuffle-button")
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
                            Button::new("playlist-more-button")
                                .icon(icons::DOTS)
                                .items_center()
                                .gap(px(variables.padding_8))
                                .on_click(move |event, _window, cx| {
                                    let items =
                                        playlist_context_menu_items(playlist_id_menu.clone(), cx);
                                    menu_for_button.update(cx, |menu, cx| {
                                        menu.show(event.position(), items, cx);
                                    });
                                }),
                        ),
                );

            let sidebar = flex_col()
                .w(px(cover_size))
                .flex_shrink_0()
                .gap(px(variables.padding_16))
                .child(cover);

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
                .id("playlist-loading")
                .w_full()
                .p(px(variables.padding_24))
                .text_color(variables.text_secondary)
                .child("Loading...")
                .into_any_element()
        };

        flex_col().size_full().child(
            div()
                .id("playlist-scroll-container")
                .flex_1()
                .size_full()
                .min_h_0()
                .relative()
                .child(div().id("playlist-content").size_full().child(body))
                .child(self.context_menu.clone()),
        )
    }
}

fn open_image_picker(
    playlist_id: Cuid,
    name: String,
    description: Option<String>,
    pinned: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let options = PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    };
    let receiver = cx.prompt_for_paths(options);
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = receiver.await else {
            return;
        };
        let Some(path) = paths.into_iter().next() else {
            return;
        };

        let result = cx
            .background_executor()
            .spawn(async move {
                let bytes = std::fs::read(&path)?;
                let img = image::load_from_memory(&bytes)?;
                let img = img.resize(500, 500, image::imageops::FilterType::Lanczos3);
                let mut out = Vec::new();
                img.write_to(
                    &mut std::io::Cursor::new(&mut out),
                    image::ImageFormat::Jpeg,
                )?;
                let mut hasher = Sha256::new();
                hasher.update(&out);
                let image_id = hasher
                    .finalize()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>();
                anyhow::Ok((image_id, out))
            })
            .await;

        let Ok((image_id, data)) = result else {
            return;
        };

        cx.update(|cx| {
            let db = cx.global::<Database>().clone();
            if let Err(e) = db.upsert_image(&image_id, &data) {
                tracing::error!("Failed to upsert playlist cover image: {}", e);
                return;
            }
            if let Err(e) = db.upsert_playlist(
                &playlist_id,
                &name,
                description.as_deref(),
                Some(&image_id),
                pinned,
            ) {
                tracing::error!("Failed to update playlist with new cover: {}", e);
                return;
            }
            cx.set_global(LibraryDataChanged);
        });
    })
    .detach();

    let _ = window;
}
