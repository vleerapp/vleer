use crate::data::db::repo::Database;
use crate::data::models::{Album, Artist, Cuid, Playlist, PlaylistListItem, Song};
use crate::media::playback::{
    play_album_last, play_album_next, play_playlist_last, play_playlist_next,
};
use crate::media::queue::Queue;
use crate::ui::app::MainWindow;
use crate::ui::assets::image_cache::vleer_cache;
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::{self, icon};
use crate::ui::variables::Variables;
use crate::ui::views::{AppView, SelectedAlbum, SelectedPlaylist};
use futures::channel::mpsc;
use gpui::{prelude::*, *};
use std::rc::Rc;
use std::time::Duration;
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
        let _ = self.tx.unbounded_send(event);
    }
}

impl Global for BackgroundUiNotifier {}

pub type MenuHandler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;
pub type SubmenuAction = Rc<dyn Fn(Cuid, &mut App) + 'static>;

pub struct ContextMenuSubmenu {
    pub playlists: Vec<PlaylistListItem>,
    pub action: SubmenuAction,
}

pub struct ContextMenuItem {
    pub label: SharedString,
    pub icon: &'static str,
    pub handler: MenuHandler,
    pub disabled: bool,
    pub is_separator: bool,
    pub is_destructive: bool,
    pub submenu: Option<ContextMenuSubmenu>,
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
            submenu: None,
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
            submenu: None,
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
            submenu: None,
        }
    }

    pub fn with_submenu(
        label: impl Into<SharedString>,
        icon_path: &'static str,
        playlists: Vec<PlaylistListItem>,
        action: impl Fn(Cuid, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon: icon_path,
            handler: Rc::new(|_, _| {}),
            disabled: false,
            is_separator: false,
            is_destructive: false,
            submenu: Some(ContextMenuSubmenu {
                playlists,
                action: Rc::new(action) as SubmenuAction,
            }),
        }
    }
}

pub struct ContextMenu {
    position: Option<Point<Pixels>>,
    items: Vec<ContextMenuItem>,
    active_submenu_idx: Option<usize>,
    submenu_panel_hovered: bool,
    submenu_close_generation: usize,
}

impl ContextMenu {
    pub fn new() -> Self {
        Self {
            position: None,
            items: Vec::new(),
            active_submenu_idx: None,
            submenu_panel_hovered: false,
            submenu_close_generation: 0,
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
        self.active_submenu_idx = None;
        self.submenu_panel_hovered = false;
        self.submenu_close_generation = 0;
        cx.notify();
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.position.is_some() {
            self.position = None;
            self.items.clear();
            self.active_submenu_idx = None;
            self.submenu_panel_hovered = false;
            self.submenu_close_generation = 0;
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
        let active_sub_idx = self.active_submenu_idx;

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
                let has_submenu = item.submenu.is_some();
                let entity_item = entity.clone();
                let entity_hover = entity.clone();

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
                    .items_center()
                    .text_color(text_color)
                    .when(active_sub_idx == Some(idx), |this| {
                        this.bg(variables.element_hover)
                    })
                    .when(!is_disabled, |this| {
                        this.cursor_pointer()
                            .hover(|s| s.bg(variables.element_hover))
                            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                if !has_submenu {
                                    (handler)(window, cx);
                                    entity_item
                                        .update(cx, |this, cx| {
                                            this.hide(cx);
                                        })
                                        .ok();
                                }
                            })
                    })
                    .on_hover(move |is_hovering: &bool, _, cx| {
                        let is_hovering = *is_hovering;
                        entity_hover
                            .update(cx, |this, cx| {
                                if is_hovering {
                                    this.submenu_close_generation =
                                        this.submenu_close_generation.wrapping_add(1);
                                    if has_submenu {
                                        this.active_submenu_idx = Some(idx);
                                    } else if !this.submenu_panel_hovered {
                                        this.active_submenu_idx = None;
                                    }
                                    cx.notify();
                                } else if has_submenu && this.active_submenu_idx == Some(idx) {
                                    let close_gen = this.submenu_close_generation.wrapping_add(1);
                                    this.submenu_close_generation = close_gen;
                                    let weak = cx.entity().downgrade();
                                    cx.spawn(async move |_this, cx: &mut AsyncApp| {
                                        let bg = cx.background_executor();
                                        bg.timer(Duration::from_millis(500)).await;
                                        cx.update(|cx| {
                                            weak.update(cx, |this, cx| {
                                                if this.submenu_close_generation == close_gen {
                                                    this.active_submenu_idx = None;
                                                    cx.notify();
                                                }
                                            })
                                            .ok();
                                        });
                                    })
                                    .detach();
                                }
                            })
                            .ok();
                    })
                    .child(icon(icon_path).text_color(if is_destructive {
                        variables.destructive
                    } else {
                        variables.text_secondary
                    }))
                    .child(div().flex_1().child(label))
                    .when(has_submenu, |this| {
                        this.child(
                            icon(icons::ARROW_RIGHT)
                                .text_color(variables.text_secondary)
                                .text_sm(),
                        )
                    })
                    .into_any_element()
            })
            .collect();

        let submenu_element = if let Some(sub_idx) = self.active_submenu_idx {
            let sub_y = position.y
                + px(4.0)
                + px(self
                    .items
                    .iter()
                    .take(sub_idx)
                    .map(|i| if i.is_separator { 1.0f32 } else { 30.0f32 })
                    .sum::<f32>());
            if let Some(item) = self.items.get(sub_idx) {
                if let Some(submenu) = &item.submenu {
                    let sub_pos = point(position.x + px(249.0), sub_y);
                    let action = submenu.action.clone();
                    let entity_panel_hover = entity.clone();

                    let playlist_items: Vec<AnyElement> = submenu
                        .playlists
                        .iter()
                        .enumerate()
                        .map(|(pi, pl)| {
                            let pl_id = pl.id.clone();
                            let action = action.clone();
                            let entity_close = entity.clone();

                            let cover: AnyElement = if let Some(image_id) = &pl.image_id {
                                div()
                                    .size(px(32.0))
                                    .flex_shrink_0()
                                    .child(
                                        img(format!("!image://{}", image_id))
                                            .size_full()
                                            .object_fit(ObjectFit::Cover),
                                    )
                                    .into_any_element()
                            } else {
                                div()
                                    .size(px(32.0))
                                    .flex_shrink_0()
                                    .bg(variables.border)
                                    .into_any_element()
                            };

                            flex_row()
                                .id(ElementId::Name(format!("ctx-pl-item-{}", pi).into()))
                                .w_full()
                                .gap(px(variables.padding_8))
                                .items_center()
                                .cursor_pointer()
                                .text_color(variables.text)
                                .hover(|s| s.bg(variables.element_hover))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    (action)(pl_id.clone(), cx);
                                    entity_close
                                        .update(cx, |this, cx| {
                                            this.hide(cx);
                                        })
                                        .ok();
                                })
                                .child(cover)
                                .child(pl.name.clone())
                                .into_any_element()
                        })
                        .collect();

                    let create_btn = {
                        let entity_close = entity.clone();
                        flex_row()
                            .id("ctx-pl-create")
                            .w_full()
                            .p(px(variables.padding_8))
                            .gap(px(variables.padding_8))
                            .items_center()
                            .cursor_pointer()
                            .text_color(variables.text)
                            .hover(|s| s.bg(variables.element_hover))
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                let new_id = Cuid::new();
                                let db = cx.global::<Database>().clone();
                                if let Err(e) = db.upsert_playlist(&new_id, "", None, None, false) {
                                    tracing::error!("Failed to create playlist: {}", e);
                                    return;
                                }
                                cx.update_global::<SelectedPlaylist, _>(|sel, _| {
                                    sel.id = Some(new_id);
                                    sel.focus_title = true;
                                });
                                cx.set_global(LibraryDataChanged);
                                if let Some(Some(root)) = window.root::<MainWindow>() {
                                    root.update(cx, |view, cx| {
                                        view.set_current_view(AppView::Playlist, window, cx);
                                    });
                                }
                                entity_close
                                    .update(cx, |this, cx| {
                                        this.hide(cx);
                                    })
                                    .ok();
                            })
                            .child(
                                icon(icons::PLUS)
                                    .size(px(32.0))
                                    .text_color(variables.text_secondary),
                            )
                            .child("Create Playlist")
                    };

                    Some(
                        deferred(
                            anchored().position(sub_pos).child(
                                div()
                                    .image_cache(vleer_cache("ctx-submenu-image-cache", 50))
                                    .id("context-submenu-container")
                                    .occlude()
                                    .on_hover(move |is_hovering: &bool, _, cx| {
                                        entity_panel_hover
                                            .update(cx, |this, cx| {
                                                this.submenu_panel_hovered = *is_hovering;
                                                if *is_hovering {
                                                    this.submenu_close_generation = this
                                                        .submenu_close_generation
                                                        .wrapping_add(1);
                                                } else {
                                                    this.active_submenu_idx = None;
                                                }
                                                cx.notify();
                                            })
                                            .ok();
                                    })
                                    .w(px(250.0))
                                    .max_h(px(320.0))
                                    .overflow_y_scroll()
                                    .bg(variables.element)
                                    .border_1()
                                    .border_color(variables.border)
                                    .child(
                                        flex_col()
                                            .w_full()
                                            .children(playlist_items)
                                            .child(create_btn),
                                    ),
                            ),
                        )
                        .with_priority(2),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let main_menu = deferred(
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
        .with_priority(1);

        flex_row()
            .child(main_menu)
            .when_some(submenu_element, |d, sub| d.child(sub))
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
    let song = db.get_song(&song_id).ok().flatten();
    let (favorite, pinned, album_id) = song
        .map(|s| (s.favorite, s.pinned, s.album_id))
        .unwrap_or((false, false, None));

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite {
        icons::UNFAVORITE
    } else {
        icons::FAVORITE
    };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { icons::UNPIN } else { icons::PIN };

    let playlists = db.get_playlists("", 0, 1000).unwrap_or_default();

    vec![
        ContextMenuItem::entry("Play next", icons::PLAY_NEXT, {
            let id = song_id.clone();
            move |_, cx| {
                cx.update_global::<Queue, _>(|queue, _| {
                    queue.add_song_next(id.clone());
                });
                cx.set_global(QueueChanged);
            }
        }),
        ContextMenuItem::entry("Play last", icons::PLAY_LAST, {
            let id = song_id.clone();
            move |_, cx| {
                cx.update_global::<Queue, _>(|queue, _| {
                    queue.add_song(id.clone());
                });
                cx.set_global(QueueChanged);
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::with_submenu("Add to Playlist", icons::PLAYLIST, playlists, {
            let song_id = song_id.clone();
            move |playlist_id, cx| {
                let db = cx.global::<Database>().clone();
                if let Err(e) = db.upsert_playlist_song(&playlist_id, &song_id) {
                    error!("append_song_to_playlist failed: {e}");
                }
                cx.set_global(LibraryDataChanged);
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry(fav_label, fav_icon, {
            let id = song_id.clone();
            move |_, cx| {
                write_and_notify(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.set_favorite::<Song>(&id, !favorite) {
                            error!("set_favorite song failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::entry(pin_label, pin_icon, {
            let id = song_id.clone();
            move |_, cx| {
                write_and_notify_pinned(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.set_pinned::<Song>(&id, !pinned) {
                            error!("set_pinned song failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Go to artist", icons::ARTIST, move |_, _| {}),
        ContextMenuItem::entry("Go to album", icons::ALBUM, {
            let id = album_id.clone();
            move |window, cx| {
                if let Some(album_id) = &id {
                    cx.set_global(SelectedAlbum(Some(album_id.clone())));
                    if let Some(Some(root)) = window.root::<MainWindow>() {
                        root.update(cx, |view, cx| {
                            view.set_current_view(AppView::Album, window, cx);
                        });
                    }
                }
            }
        }),
        ContextMenuItem::entry("Properties", icons::PROPERTIES, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Remove from library", icons::TRASH, {
            let id = song_id.clone();
            move |_, cx| {
                write_and_notify(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.delete_song(&id) {
                            error!("remove_song failed: {e}");
                        }
                    }
                });
            }
        }),
    ]
}

pub fn album_context_menu_items(album_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let album = db.get_album(&album_id).ok().flatten();
    let (favorite, pinned) = album
        .map(|a| (a.favorite, a.pinned))
        .unwrap_or((false, false));

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite {
        icons::UNFAVORITE
    } else {
        icons::FAVORITE
    };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { icons::UNPIN } else { icons::PIN };

    let playlists = db.get_playlists("", 0, 1000).unwrap_or_default();

    vec![
        ContextMenuItem::entry("Play next", icons::PLAY_NEXT, {
            let id = album_id.clone();
            move |_, cx| play_album_next(id.clone(), cx)
        }),
        ContextMenuItem::entry("Play last", icons::PLAY_LAST, {
            let id = album_id.clone();
            move |_, cx| play_album_last(id.clone(), cx)
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::with_submenu("Add to Playlist", icons::PLAYLIST, playlists, {
            let album_id = album_id.clone();
            move |playlist_id, cx| {
                let db = cx.global::<Database>().clone();
                if let Ok(songs) = db.get_album_songs(&album_id) {
                    for song in &songs {
                        if let Err(e) = db.upsert_playlist_song(&playlist_id, &song.id) {
                            error!("upsert_playlist_song failed: {e}");
                        }
                    }
                }
                cx.set_global(LibraryDataChanged);
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry(fav_label, fav_icon, {
            let id = album_id.clone();
            move |_, cx| {
                write_and_notify(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.set_favorite::<Album>(&id, !favorite) {
                            error!("set_favorite album failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::entry(pin_label, pin_icon, {
            let id = album_id.clone();
            move |_, cx| {
                write_and_notify_pinned(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.set_pinned::<Album>(&id, !pinned) {
                            error!("set_pinned album failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Go to artist", icons::ARTIST, move |_, _| {}),
        ContextMenuItem::entry("Properties", icons::PROPERTIES, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Remove from library", icons::TRASH, {
            move |_, cx| {
                write_and_notify(cx, {
                    let id = album_id.clone();
                    move |db| {
                        if let Err(e) = db.delete_album(&id) {
                            error!("remove album failed: {e}");
                        }
                    }
                });
            }
        }),
    ]
}

pub fn artist_context_menu_items(artist_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let artist = db.get_artist(&artist_id).ok().flatten();
    let favorite = artist.as_ref().map(|a| a.favorite).unwrap_or(false);
    let pinned = artist.as_ref().map(|a| a.pinned).unwrap_or(false);

    let fav_label = if favorite { "Unfavorite" } else { "Favorite" };
    let fav_icon = if favorite {
        icons::UNFAVORITE
    } else {
        icons::FAVORITE
    };
    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { icons::UNPIN } else { icons::PIN };

    vec![
        ContextMenuItem::entry("Play all songs", icons::PLAY, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::entry(fav_label, fav_icon, {
            let id = artist_id.clone();
            move |_, cx| {
                write_and_notify(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.set_favorite::<Artist>(&id, !favorite) {
                            error!("set_favorite artist failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::entry(pin_label, pin_icon, {
            move |_, cx| {
                write_and_notify_pinned(cx, {
                    let id = artist_id.clone();
                    move |db| {
                        if let Err(e) = db.set_pinned::<Artist>(&id, !pinned) {
                            error!("set_pinned artist failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Go to albums", icons::ALBUM, move |_, _| {}),
        ContextMenuItem::entry("Properties", icons::PROPERTIES, move |_, _| {}),
    ]
}

pub fn playlist_context_menu_items(playlist_id: Cuid, cx: &App) -> Vec<ContextMenuItem> {
    let db = cx.global::<Database>().clone();
    let playlist = db.get_playlist(&playlist_id).ok().flatten();
    let pinned = playlist.map(|p| p.pinned).unwrap_or(false);

    let pin_label = if pinned { "Unpin" } else { "Pin" };
    let pin_icon = if pinned { icons::UNPIN } else { icons::PIN };

    vec![
        ContextMenuItem::entry("Play next", icons::PLAY_NEXT, {
            let id = playlist_id.clone();
            move |_, cx| play_playlist_next(id.clone(), cx)
        }),
        ContextMenuItem::entry("Play last", icons::PLAY_LAST, {
            let id = playlist_id.clone();
            move |_, cx| play_playlist_last(id.clone(), cx)
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry(pin_label, pin_icon, {
            let id = playlist_id.clone();
            move |_, cx| {
                write_and_notify_pinned(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.set_pinned::<Playlist>(&id, !pinned) {
                            error!("set_pinned playlist failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::separator(),
        ContextMenuItem::entry("Clear playlist", icons::X, {
            let id = playlist_id.clone();
            move |_, cx| {
                write_and_notify(cx, {
                    let id = id.clone();
                    move |db| {
                        if let Err(e) = db.clear_playlist(&id) {
                            error!("clear_playlist failed: {e}");
                        }
                    }
                });
            }
        }),
        ContextMenuItem::entry("Properties", icons::PROPERTIES, move |_, _| {}),
        ContextMenuItem::separator(),
        ContextMenuItem::destructive("Delete playlist", icons::TRASH, {
            move |window, cx| {
                write_and_notify(cx, {
                    let id = playlist_id.clone();
                    move |db| {
                        if let Err(e) = db.delete_playlist(&id) {
                            error!("delete_playlist failed: {e}");
                        }
                    }
                });
                let is_viewing = cx.global::<SelectedPlaylist>().id.as_ref() == Some(&playlist_id);
                if is_viewing {
                    cx.update_global::<SelectedPlaylist, _>(|sel, _| sel.id = None);
                    if let Some(Some(root)) = window.root::<MainWindow>() {
                        root.update(cx, |view, cx| {
                            view.set_current_view(AppView::Playlists, window, cx);
                        });
                    }
                }
            }
        }),
    ]
}
