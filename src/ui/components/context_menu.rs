use crate::data::db::repo::Database;
use crate::data::models::{Album, Artist, Cuid, Playlist, Song};
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::icon::icon;
use crate::ui::components::icons::icons::{
    ALBUM, ARTIST, FAVORITE, PIN, PLAY, PLAY_LAST, PLAY_NEXT, PLUS, PROPERTIES, TRASH, UNFAVORITE,
    UNPIN, X,
};
use crate::ui::variables::Variables;
use gpui::{prelude::*, *};
use std::rc::Rc;
use tokio::sync::mpsc;
use tracing::error;

#[derive(Default)]
pub struct PinnedItemsChanged;
impl Global for PinnedItemsChanged {}

#[derive(Default)]
pub struct LibraryDataChanged;
impl Global for LibraryDataChanged {}

#[derive(Default)]
pub struct HomeDataChanged;
impl Global for HomeDataChanged {}

#[derive(Default)]
pub struct QueueChanged;
impl Global for QueueChanged {}

#[derive(Clone, Copy, Debug)]
pub enum BackgroundUiEvent {
    HomeDataChanged,
    LibraryDataChanged,
}

#[derive(Clone)]
pub struct BackgroundUiNotifier {
    tx: mpsc::UnboundedSender<BackgroundUiEvent>,
}

impl BackgroundUiNotifier {
    pub fn new(tx: mpsc::UnboundedSender<BackgroundUiEvent>) -> Self {
        Self { tx }
    }

    pub fn notify(&self, event: BackgroundUiEvent) {
        let _ = self.tx.send(event);
    }
}

impl Global for BackgroundUiNotifier {}

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

pub struct ContextMenuItem {
    pub label: SharedString,
    pub icon: &'static str,
    pub handler: Rc<dyn Fn(&mut Window, &mut App) + 'static>,
    pub disabled: bool,
    pub is_separator: bool,
    pub is_destructive: bool,
}

impl ContextMenuItem {
    pub fn entry(
        label: impl Into<SharedString>,
        icon_path: &'static str,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon: icon_path,
            handler: Rc::new(handler),
            disabled: false,
            is_separator: false,
            is_destructive: false,
        }
    }

    pub fn destructive(
        label: impl Into<SharedString>,
        icon_path: &'static str,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon: icon_path,
            handler: Rc::new(handler),
            disabled: false,
            is_separator: false,
            is_destructive: true,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: "".into(),
            icon: "",
            handler: Rc::new(|_, _| {}),
            disabled: false,
            is_separator: true,
            is_destructive: false,
        }
    }
}

pub struct ContextMenu {
    position: Option<Point<Pixels>>,
    items: Vec<ContextMenuItem>,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            position: None,
            items: Vec::new(),
        }
    }

    pub fn show(
        &mut self,
        position: Point<Pixels>,
        items: Vec<ContextMenuItem>,
        cx: &mut Context<Self>,
    ) {
        self.position = Some(position);
        self.items = items;
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.position.is_some() {
            self.position = None;
            self.items.clear();
            cx.notify();
        }
    }
}

impl Render for ContextMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(position) = self.position else {
            return div().into_any_element();
        };

        let variables = *cx.global::<Variables>();
        let entity = cx.entity().downgrade();
        let entity_out = entity.clone();

        let menu_items: Vec<AnyElement> = self
            .items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                if item.is_separator {
                    return div()
                        .id(ElementId::Name(format!("ctx-sep-{}", idx).into()))
                        .w_full()
                        .h(px(1.0))
                        .bg(variables.border)
                        .into_any_element();
                }

                let is_destructive = item.is_destructive;
                let is_disabled = item.disabled;
                let label = item.label.clone();
                let icon_path = item.icon;
                let handler = item.handler.clone();
                let entity_item = entity.clone();

                let text_color = if is_disabled {
                    variables.text_muted
                } else if is_destructive {
                    variables.destructive
                } else {
                    variables.text
                };

                flex_row()
                    .id(ElementId::Name(format!("ctx-item-{}", idx).into()))
                    .w_full()
                    .p(px(variables.padding_8))
                    .gap(px(variables.padding_8))
                    .text_color(text_color)
                    .when(!is_disabled, |this| {
                        this.cursor_pointer()
                            .hover(|s| s.bg(variables.element_hover))
                            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                (handler)(window, cx);
                                entity_item
                                    .update(cx, |this, cx| {
                                        this.hide(cx);
                                    })
                                    .ok();
                            })
                    })
                    .child(icon(icon_path).text_color(if is_destructive {
                        variables.destructive
                    } else {
                        variables.text_secondary
                    }))
                    .child(label)
                    .into_any_element()
            })
            .collect();

        deferred(
            anchored().position(position).child(
                div()
                    .id("context-menu-container")
                    .occlude()
                    .on_mouse_down_out(move |_event, _window, cx| {
                        entity_out
                            .update(cx, |this, cx| {
                                this.hide(cx);
                            })
                            .ok();
                    })
                    .w(px(250.0))
                    .bg(variables.element)
                    .border_1()
                    .border_color(variables.border)
                    .child(flex_col().w_full().children(menu_items)),
            ),
        )
        .with_priority(1)
        .into_any_element()
    }
}

fn write_and_notify(cx: &mut App, write: impl FnOnce(&Database)) {
    let db = cx.global::<Database>().clone();
    write(&db);

    cx.set_global(LibraryDataChanged);
}

fn write_and_notify_pinned(cx: &mut App, write: impl FnOnce(&Database)) {
    let db = cx.global::<Database>().clone();
    write(&db);
    cx.set_global(LibraryDataChanged);
    cx.set_global(PinnedItemsChanged);
}

pub fn play_song_ids_now(song_ids: Vec<Cuid>, cx: &mut App) {
    if song_ids.is_empty() {
        return;
    }

    cx.update_global::<Queue, _>(|queue, _| {
        queue.clear();
        queue.add_songs(song_ids);
    });

    cx.update_global::<Playback, _>(|playback, cx| {
        playback.play_queue(cx);
    });

    cx.set_global(QueueChanged::default());
}

pub fn play_song_now(song_id: Cuid, cx: &mut App) {
    play_song_ids_now(vec![song_id], cx);
}

pub fn play_album_now(album_id: Cuid, cx: &mut App) {
    let db = cx.global::<Database>().clone();

    cx.spawn(async move |cx| {
        let song_ids = db
            .get_album_songs(&album_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|song| song.id)
            .collect::<Vec<_>>();

        if song_ids.is_empty() {
            return;
        }

        cx.update(|cx| {
            play_song_ids_now(song_ids, cx);
        });
    })
    .detach();
}

pub fn song_context_menu_items(song_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let song = run_sync(db.get_song(song_id.clone())).ok().flatten();
    let (favorite, pinned) = song
        .map(|s| (s.favorite, s.pinned))
        .unwrap_or((false, false));

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite { UNFAVORITE } else { FAVORITE };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { UNPIN } else { PIN };

    let id_fav = song_id.clone();
    let id_pin = song_id.clone();
    let id_play_next = song_id.clone();
    let id_play_last = song_id.clone();
    let id_remove = song_id.clone();

    vec![
        ContextMenuItem::entry("Play next", PLAY_NEXT, move |_, cx| {
            let id = id_play_next.clone();
            cx.update_global::<Queue, _>(|queue, _| {
                queue.add_song_next(id);
            });
            cx.set_global(QueueChanged::default());
        }),
        ContextMenuItem::entry("Play last", PLAY_LAST, move |_, cx| {
            let id = id_play_last.clone();
            cx.update_global::<Queue, _>(|queue, _| {
                queue.add_song(id);
            });
            cx.set_global(QueueChanged::default());
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Add to playlist", PLUS, move |_, _| {}),
        ContextMenuItem::entry(fav_label, fav_icon, move |_, cx| {
            let id = id_fav.clone();
            let new_val = !favorite;
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.set_favorite::<Song>(&id, new_val)) {
                    error!("set_favorite song failed: {e}");
                }
            });
        }),
        ContextMenuItem::entry(pin_label, pin_icon, move |_, cx| {
            let id = id_pin.clone();
            let new_val = !pinned;
            write_and_notify_pinned(cx, move |db| {
                if let Err(e) = run_sync(db.set_pinned::<Song>(&id, new_val)) {
                    error!("set_pinned song failed: {e}");
                }
            });
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Go to artist", ARTIST, move |_, _| {}),
        ContextMenuItem::entry("Go to album", ALBUM, move |_, _| {}),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Remove from library", TRASH, move |_, cx| {
            let id = id_remove.clone();
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.delete_song(&id)) {
                    error!("remove_song failed: {e}");
                }
            });
        }),
    ]
}

pub fn album_context_menu_items(album_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let album = run_sync(db.get_album(album_id.clone())).ok().flatten();
    let (favorite, pinned) = album
        .map(|a| (a.favorite, a.pinned))
        .unwrap_or((false, false));

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite { UNFAVORITE } else { FAVORITE };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { UNPIN } else { PIN };

    let id_fav = album_id.clone();
    let id_pin = album_id.clone();
    let id_play = album_id.clone();
    let id_remove = album_id.clone();

    vec![
        ContextMenuItem::entry("Play all songs", PLAY, move |_, cx| {
            let id = id_play.clone();
            let db = cx.global::<Database>().clone();
            let ids = run_sync(db.get_album_songs(&id));
            if let Ok(song_ids) = ids {
                if !song_ids.is_empty() {
                    cx.update_global::<Queue, _>(|q, _| {
                        q.add_songs(song_ids.into_iter().map(|s| s.id).collect());
                    });
                    cx.update_global::<Playback, _>(|playback, cx| {
                        playback.play_queue(cx);
                    });
                    cx.set_global(QueueChanged::default());
                }
            }
        }),
        ContextMenuItem::entry("Add all to playlist", PLUS, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::entry(fav_label, fav_icon, move |_, cx| {
            let id = id_fav.clone();
            let new_val = !favorite;
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.set_favorite::<Album>(&id, new_val)) {
                    error!("set_favorite album failed: {e}");
                }
            });
        }),
        ContextMenuItem::entry(pin_label, pin_icon, move |_, cx| {
            let id = id_pin.clone();
            let new_val = !pinned;
            write_and_notify_pinned(cx, move |db| {
                if let Err(e) = run_sync(db.set_pinned::<Album>(&id, new_val)) {
                    error!("set_pinned album failed: {e}");
                }
            });
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Go to artist", ARTIST, move |_, _| {}),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Remove from library", TRASH, move |_, cx| {
            let id = id_remove.clone();
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.delete_album(&id)) {
                    error!("remove album failed: {e}");
                }
            });
        }),
    ]
}

pub fn artist_context_menu_items(artist_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let artist = run_sync(db.get_artist(artist_id.clone())).ok().flatten();
    let favorite = artist.as_ref().map(|a| a.favorite).unwrap_or(false);
    let pinned = artist.as_ref().map(|a| a.pinned).unwrap_or(false);

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite { UNFAVORITE } else { FAVORITE };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { UNPIN } else { PIN };

    let id_fav = artist_id.clone();
    let id_pin = artist_id.clone();

    vec![
        ContextMenuItem::entry("Play all songs", PLAY, move |_, _| {}),
        ContextMenuItem::entry("Add all to playlist", PLUS, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::entry(fav_label, fav_icon, move |_, cx| {
            let id = id_fav.clone();
            let new_val = !favorite;
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.set_favorite::<Artist>(&id, new_val)) {
                    error!("set_favorite artist failed: {e}");
                }
            });
        }),
        ContextMenuItem::entry(pin_label, pin_icon, move |_, cx| {
            let id = id_pin.clone();
            let new_val = !pinned;
            write_and_notify_pinned(cx, move |db| {
                if let Err(e) = run_sync(db.set_pinned::<Artist>(&id, new_val)) {
                    error!("set_pinned artist failed: {e}");
                }
            });
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Go to albums", ALBUM, move |_, _| {}),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {}),
    ]
}

pub fn playlist_context_menu_items(playlist_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let playlist = run_sync(db.get_playlist(&playlist_id)).ok().flatten();
    let pinned = playlist.map(|p| p.pinned).unwrap_or(false);

    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { UNPIN } else { PIN };

    let id_pin = playlist_id.clone();
    let id_play = playlist_id.clone();
    let id_clear = playlist_id.clone();
    let id_delete = playlist_id.clone();

    vec![
        ContextMenuItem::entry("Play playlist", PLAY, move |_, cx| {
            let id = id_play.clone();
            let db = cx.global::<Database>().clone();
            let songs = run_sync(db.get_playlist_songs(&id));
            if let Ok(songs) = songs {
                let song_ids: Vec<Cuid> = songs.into_iter().map(|t| t.song.id).collect();
                if !song_ids.is_empty() {
                    cx.update_global::<Queue, _>(|q, _| {
                        q.add_songs(song_ids);
                    });
                    cx.update_global::<Playback, _>(|playback, cx| {
                        playback.play_queue(cx);
                    });
                    cx.set_global(QueueChanged::default());
                }
            }
        }),
        ContextMenuItem::entry("Add to library", PLUS, move |_, _| {}),
        ContextMenuItem::entry(pin_label, pin_icon, move |_, cx| {
            let id = id_pin.clone();
            let new_val = !pinned;
            write_and_notify_pinned(cx, move |db| {
                if let Err(e) = run_sync(db.set_pinned::<Playlist>(&id, new_val)) {
                    error!("set_pinned playlist failed: {e}");
                }
            });
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Clear playlist", X, move |_, cx| {
            let id = id_clear.clone();
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.clear_playlist(&id)) {
                    error!("clear_playlist failed: {e}");
                }
            });
        }),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Delete playlist", TRASH, move |_, cx| {
            let id = id_delete.clone();
            write_and_notify(cx, move |db| {
                if let Err(e) = run_sync(db.delete_playlist(&id)) {
                    error!("delete_playlist failed: {e}");
                }
            });
        }),
    ]
}
