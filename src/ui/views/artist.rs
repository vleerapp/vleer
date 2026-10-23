use gpui::{prelude::FluentBuilder, *};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use crate::{
    data::{
        db::Database,
        models::{Artist, Cuid, Song},
    },
    media::playback::{play_artist_now, play_song_ids_now, play_song_ids_shuffled},
    ui::{
        assets::{ImageRequest, cover_uri},
        components::{
            button::Button,
            context_menu::{
                ContextMenu, LibraryDataChanged, album_context_menu_items,
                artist_context_menu_items,
            },
            cover::{avatar, release_row},
            div::{flex_col, flex_row},
            icons,
            nav::open_album,
            scrollbar::ScrollableElement,
            song_table::{
                SongEntry, SongTableHandlers, SongTableItem, SongTableLayout,
                calculate_column_widths, queue_after_handler, song_entry_from_song,
            },
        },
        variables::Variables,
        views::{ActiveView, AppView, SelectedArtist},
    },
};

const HERO_HEIGHT: f32 = 280.0;
const HERO_IMAGE_SIZE: u32 = 64;
const HERO_BLUR_SIGMA: u32 = 8;
const AVATAR_SIZE: f32 = 160.0;

struct Release {
    key: String,
    album_id: Option<Cuid>,
    title: String,
    image_id: Option<String>,
    year: Option<String>,
    songs: Vec<Song>,
}

impl Release {
    fn cover(&self) -> Option<String> {
        let fallback = match (&self.album_id, self.songs.first()) {
            (Some(album_id), _) => ImageRequest::Album(album_id.clone()),
            (None, Some(song)) => ImageRequest::Track(song.id.clone()),
            (None, None) => return None,
        };
        Some(cover_uri(self.image_id.as_deref(), fallback))
    }

    fn subtitle(&self) -> String {
        let count = format!("{} songs", self.songs.len());
        match &self.year {
            Some(year) => format!("{year} \u{00B7} {count}"),
            None => count,
        }
    }
}

struct ArtistData {
    artist: Option<Artist>,
    song_ids: Arc<Vec<Cuid>>,
    albums: Vec<Release>,
    appears_on: Vec<Release>,
}

fn load_releases(db: &Database, artist_id: &Cuid, appears_on: bool) -> Vec<Release> {
    db.get_artist_albums(artist_id, appears_on)
        .unwrap_or_default()
        .into_iter()
        .map(|album| Release {
            key: album.id.to_string(),
            songs: db.get_album_songs(&album.id).unwrap_or_default(),
            album_id: Some(album.id),
            title: album.title,
            image_id: album.image_id,
            year: album
                .year
                .map(|y| y.chars().take(4).collect::<String>())
                .filter(|y| !y.is_empty()),
        })
        .collect()
}

fn load_artist(db: &Database, artist_id: &Cuid) -> ArtistData {
    let song_ids = db.get_artist_song_ids(artist_id).unwrap_or_default();
    let mut albums = load_releases(db, artist_id, false);

    let loose: Vec<Song> = db
        .get_songs_by_ids(&song_ids)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.album_id.is_none())
        .collect();
    if !loose.is_empty() {
        albums.push(Release {
            key: "loose".to_string(),
            album_id: None,
            title: "Other Songs".to_string(),
            image_id: None,
            year: None,
            songs: loose,
        });
    }

    let appears_on = load_releases(db, artist_id, true);

    ArtistData {
        artist: db.get_artist(artist_id).ok().flatten(),
        song_ids: Arc::new(song_ids),
        albums,
        appears_on,
    }
}

pub struct ArtistView {
    artist_id: Option<Cuid>,
    data: Option<ArtistData>,
    expanded: HashSet<String>,
    tracks: HashMap<String, Vec<Entity<SongTableItem>>>,
    load_task: Option<Task<()>>,
    context_menu: Entity<ContextMenu>,
}

impl ArtistView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            artist_id: cx.global::<SelectedArtist>().0.clone(),
            data: None,
            expanded: HashSet::new(),
            tracks: HashMap::new(),
            load_task: None,
            context_menu: cx.new(|_| ContextMenu::new()),
        };

        if cx.global::<ActiveView>().0 == AppView::Artist {
            view.reload(cx);
        }

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 != AppView::Artist {
                return;
            }
            let new_id = cx.global::<SelectedArtist>().0.clone();
            if new_id != this.artist_id || this.data.is_none() {
                this.artist_id = new_id;
                this.expanded.clear();
                this.tracks.clear();
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
        let Some(artist_id) = self.artist_id.clone() else {
            self.data = None;
            cx.notify();
            return;
        };

        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();

        self.load_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let id = artist_id.clone();
            let data = bg.spawn(async move { load_artist(&db, &id) }).await;

            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    if this.artist_id.as_ref() != Some(&artist_id) {
                        return;
                    }
                    this.data = Some(data);
                    this.tracks.clear();
                    for key in this.expanded.clone() {
                        this.build_tracks(&key, cx);
                    }
                    cx.notify();
                })
            })
            .ok();
        }));
    }

    fn build_tracks(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(songs) = self.data.as_ref().and_then(|data| {
            data.albums
                .iter()
                .chain(&data.appears_on)
                .find(|release| release.key == key)
                .map(|release| release.songs.clone())
        }) else {
            return;
        };

        let rows: Rc<Vec<Arc<SongEntry>>> =
            Rc::new(songs.iter().map(song_entry_from_song).collect());
        let (number_width, duration_width) = calculate_column_widths(rows.len());
        let items = (0..rows.len())
            .map(|index| {
                let get_row_rows = rows.clone();
                SongTableItem::new(
                    cx,
                    index,
                    SongTableLayout {
                        number_width,
                        duration_width,
                        show_numbers: true,
                        show_album: false,
                        show_cover: false,
                        show_genre: false,
                        sort_method: None,
                    },
                    SongTableHandlers {
                        get_row: Rc::new(move |_cx, idx, _sort| get_row_rows.get(idx).cloned()),
                        on_select: None,
                        get_queue: Some(queue_after_handler(rows.clone())),
                        playlist: None,
                        reorder: None,
                    },
                )
            })
            .collect();
        self.tracks.insert(key.to_string(), items);
    }

    fn releases_section(
        &self,
        id: &str,
        title: &str,
        releases: &[Release],
        variables: &Variables,
        cx: &mut Context<Self>,
    ) -> Div {
        let rows = releases.iter().map(|release| {
            let expanded = self.expanded.contains(&release.key);
            let release_ids: Rc<Vec<Cuid>> =
                Rc::new(release.songs.iter().map(|s| s.id.clone()).collect());

            let tracks = if expanded {
                self.tracks
                    .get(&release.key)
                    .map(|items| {
                        items
                            .iter()
                            .cloned()
                            .map(IntoElement::into_any_element)
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let key = release.key.clone();
            let menu = self.context_menu.clone();
            let album_id = release.album_id.clone();
            let open_id = release.album_id.clone();

            release_row(
                format!("{id}-{}", release.key),
                release.cover(),
                release.title.clone(),
                release.subtitle(),
                expanded,
                variables,
                cx.listener(move |this, _event, _window, cx| {
                    if !this.expanded.remove(&key) {
                        this.expanded.insert(key.clone());
                        this.build_tracks(&key, cx);
                    }
                    cx.notify();
                }),
                move |_window, cx| play_song_ids_now(release_ids.to_vec(), cx),
                move |event, _window, cx| {
                    if let Some(album_id) = &album_id {
                        let items = album_context_menu_items(album_id.clone(), cx);
                        menu.update(cx, |menu, cx| menu.show(event.position, items, cx));
                    }
                },
                open_id.map(|id| {
                    move |window: &mut Window, cx: &mut App| open_album(id.clone(), window, cx)
                }),
                tracks,
            )
        });

        flex_col()
            .w_full()
            .gap(px(variables.padding_16))
            .child(
                div()
                    .font_weight(FontWeight(600.0))
                    .text_size(px(18.0))
                    .child(title.to_string()),
            )
            .child(
                flex_col()
                    .w_full()
                    .gap(px(variables.padding_16))
                    .children(rows),
            )
    }
}

fn hero(
    artist: &Artist,
    stats: String,
    buttons: impl IntoElement,
    variables: &Variables,
) -> impl IntoElement {
    let solid = rgb_to_hsla(variables.background);
    let image = artist.image_id.as_deref();

    div()
        .id("artist-hero")
        .relative()
        .w_full()
        .h(px(HERO_HEIGHT))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(variables.element)
        .when_some(image, |this, image_id| {
            this.child(
                img(format!(
                    "!image://{image_id}?size={HERO_IMAGE_SIZE}&blur={HERO_BLUR_SIGMA}"
                ))
                .absolute()
                .size_full()
                .object_fit(ObjectFit::Cover),
            )
        })
        .child(div().absolute().size_full().bg(linear_gradient(
            180.0,
            linear_color_stop(solid.opacity(0.35), 0.0),
            linear_color_stop(solid, 1.0),
        )))
        .child(
            flex_row()
                .absolute()
                .size_full()
                .items_end()
                .p(px(variables.padding_24))
                .gap(px(variables.padding_24))
                .child(avatar(image, AVATAR_SIZE, variables))
                .child(
                    flex_col()
                        .min_w_0()
                        .gap(px(variables.padding_8))
                        .child(
                            div()
                                .text_size(px(40.0))
                                .line_height(px(44.0))
                                .font_weight(FontWeight::BOLD)
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(artist.name.clone()),
                        )
                        .child(
                            div()
                                .text_color(variables.text_secondary)
                                .whitespace_nowrap()
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .child(stats),
                        )
                        .child(buttons),
                ),
        )
}

impl Render for ArtistView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let context_menu = self.context_menu.clone();

        let status = |text: &'static str| {
            flex_row()
                .id("artist-status")
                .w_full()
                .p(px(variables.padding_24))
                .text_color(variables.text_secondary)
                .child(text)
                .into_any_element()
        };

        let body = match self.data.as_ref().map(|d| (d, d.artist.as_ref())) {
            None => status("Loading..."),
            Some((_, None)) => status("Artist not found"),
            Some((data, Some(artist))) => {
                let stats = format!(
                    "{} songs \u{00B7} {} albums",
                    data.song_ids.len(),
                    data.albums.iter().filter(|r| r.album_id.is_some()).count()
                );

                let artist_id_play = artist.id.clone();
                let artist_id_menu = artist.id.clone();
                let ids_shuffle = data.song_ids.clone();
                let menu_for_button = context_menu.clone();

                let buttons = flex_row()
                    .gap(px(variables.padding_8))
                    .pt(px(variables.padding_8))
                    .child(
                        Button::new("artist-play-button")
                            .icon(icons::PLAY)
                            .items_center()
                            .gap(px(variables.padding_8))
                            .bg_color(variables.accent)
                            .color(variables.background)
                            .hover_color(variables.background)
                            .hover_style(|s| s.bg(variables.accent_hover))
                            .on_click(move |_event, _window, cx| {
                                play_artist_now(artist_id_play.clone(), cx);
                            }),
                    )
                    .child(
                        Button::new("artist-shuffle-button")
                            .icon(icons::SHUFFLE)
                            .items_center()
                            .gap(px(variables.padding_8))
                            .on_click(move |_event, _window, cx| {
                                play_song_ids_shuffled(ids_shuffle.to_vec(), cx);
                            }),
                    )
                    .child(
                        Button::new("artist-more-button")
                            .icon(icons::DOTS)
                            .items_center()
                            .gap(px(variables.padding_8))
                            .on_click(move |event, _window, cx| {
                                let items = artist_context_menu_items(artist_id_menu.clone(), cx);
                                menu_for_button.update(cx, |menu, cx| {
                                    menu.show(event.position(), items, cx);
                                });
                            }),
                    );

                flex_col()
                    .w_full()
                    .child(hero(artist, stats, buttons, &variables))
                    .child(
                        flex_col()
                            .w_full()
                            .p(px(variables.padding_24))
                            .gap(px(variables.padding_24))
                            .when(!data.albums.is_empty(), |this| {
                                this.child(self.releases_section(
                                    "artist-albums",
                                    "Discography",
                                    &data.albums,
                                    &variables,
                                    cx,
                                ))
                            })
                            .when(!data.appears_on.is_empty(), |this| {
                                this.child(self.releases_section(
                                    "artist-appears-on",
                                    "Appears On",
                                    &data.appears_on,
                                    &variables,
                                    cx,
                                ))
                            }),
                    )
                    .into_any_element()
            }
        };

        div()
            .id("artist-container")
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .id("artist-scroll-container")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(body)
                    .child(self.context_menu.clone()),
            )
    }
}
