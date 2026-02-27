use crate::data::db::repo::Database;
use crate::data::models::{Album, Artist, Cuid, Playlist, Song};
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::icon::icon;
use crate::ui::components::icons::icons::{
    ALBUM, ARTIST, FAVORITE, PIN, PLAY, PLAY_LAST, PLAY_NEXT, PLUS, PROPERTIES, TRASH, UNFAVORITE,
    UNPIN, X,
};
use crate::ui::variables::Variables;
use gpui::{prelude::*, *};
use std::rc::Rc;
use tracing::error;

#[derive(Default)]
pub struct PinnedItemsChanged;
impl Global for PinnedItemsChanged {}

#[derive(Default)]
pub struct LibraryDataChanged;
impl Global for LibraryDataChanged {}

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
    let id1 = song_id.clone();
    let id2 = song_id.clone();
    let id3 = song_id.clone();
    let id6 = song_id.clone();
    let id7 = song_id.clone();
    let id8 = song_id.clone();
    let id9 = song_id.clone();

    vec![
        ContextMenuItem::entry("Play next", PLAY_NEXT, move |_, _| {
            let _ = &id1;
        }),
        ContextMenuItem::entry("Play last", PLAY_LAST, move |_, _| {
            let _ = &id2;
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Add to playlist", PLUS, move |_, _| {
            let _ = &id3;
        }),
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
        ContextMenuItem::entry("Go to artist", ARTIST, move |_, _| {
            let _ = &id6;
        }),
        ContextMenuItem::entry("Go to album", ALBUM, move |_, _| {
            let _ = &id7;
        }),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {
            let _ = &id8;
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Remove from library", TRASH, move |_, _| {
            let _ = &id9;
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
    let id1 = album_id.clone();
    let id2 = album_id.clone();
    let id5 = album_id.clone();
    let id6 = album_id.clone();
    let id7 = album_id.clone();

    vec![
        ContextMenuItem::entry("Play all songs", PLAY, move |_, _| {
            let _ = &id1;
        }),
        ContextMenuItem::entry("Add all to playlist", PLUS, move |_, _| {
            let _ = &id2;
        }),
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
        ContextMenuItem::entry("Go to artist", ARTIST, move |_, _| {
            let _ = &id5;
        }),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {
            let _ = &id6;
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Remove from library", TRASH, move |_, _| {
            let _ = &id7;
        }),
    ]
}

pub fn artist_context_menu_items(artist_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let artist = run_sync(db.get_artist(artist_id.clone())).ok().flatten();
    let (favorite, pinned) = artist
        .map(|a| (a.favorite, a.pinned))
        .unwrap_or((false, false));

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite { UNFAVORITE } else { FAVORITE };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { UNPIN } else { PIN };

    let id_fav = artist_id.clone();
    let id_pin = artist_id.clone();
    let id1 = artist_id.clone();
    let id2 = artist_id.clone();
    let id5 = artist_id.clone();
    let id6 = artist_id.clone();

    vec![
        ContextMenuItem::entry("Play all songs", PLAY, move |_, _| {
            let _ = &id1;
        }),
        ContextMenuItem::entry("Add all to playlist", PLUS, move |_, _| {
            let _ = &id2;
        }),
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
        ContextMenuItem::entry("Go to albums", ALBUM, move |_, _| {
            let _ = &id5;
        }),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {
            let _ = &id6;
        }),
    ]
}

#[allow(dead_code)]
pub fn playlist_context_menu_items(playlist_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let playlist = run_sync(db.get_playlist(&playlist_id)).ok().flatten();
    let pinned = playlist.map(|p| p.pinned).unwrap_or(false);

    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { UNPIN } else { PIN };

    let id_pin = playlist_id.clone();
    let id1 = playlist_id.clone();
    let id2 = playlist_id.clone();
    let id4 = playlist_id.clone();
    let id5 = playlist_id.clone();
    let id6 = playlist_id.clone();

    vec![
        ContextMenuItem::entry("Play playlist", PLAY, move |_, _| {
            let _ = &id1;
        }),
        ContextMenuItem::entry("Add to library", PLUS, move |_, _| {
            let _ = &id2;
        }),
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
        ContextMenuItem::entry("Clear playlist", X, move |_, _| {
            let _ = &id4;
        }),
        ContextMenuItem::entry("Properties", PROPERTIES, move |_, _| {
            let _ = &id5;
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Delete playlist", TRASH, move |_, _| {
            let _ = &id6;
        }),
    ]
}
