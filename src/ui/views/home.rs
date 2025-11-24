use std::path::PathBuf;

use gpui::{prelude::FluentBuilder, *};
use gpui_component::*;

use std::collections::HashMap;

use crate::{
    data::{db::Database, types::Song},
    ui::{
        components::{
            icons::{
                icon::icon,
                icons::{ARROW_LEFT, ARROW_RIGHT},
            },
            title::Title,
        },
        variables::Variables,
    },
};

#[derive(Clone)]
enum RecentItem {
    Song {
        title: String,
        artist_name: Option<String>,
        cover_uri: Option<String>,
    },
    Album {
        title: String,
        artist_name: Option<String>,
        cover_uri: Option<String>,
        year: Option<String>,
    },
}

pub struct HomeView {
    pub hovered: bool,
    recently_added: Vec<RecentItem>,
    recently_added_offset: usize,
    covers_dir: PathBuf,
    container_width: Option<f32>,
}

const MIN_COVER_SIZE: f32 = 180.0;
const MAX_COVER_SIZE: f32 = 220.0;
const GAP_SIZE: f32 = 16.0;

impl HomeView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let covers_dir = dirs::data_dir()
            .expect("couldn't get data directory")
            .join("vleer")
            .join("covers");

        let mut view = Self {
            hovered: false,
            recently_added: Vec::new(),
            recently_added_offset: 0,
            covers_dir,
            container_width: None,
        };

        view.load_recently_added(window, cx);
        view
    }

    fn load_recently_added(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let db = cx.global::<Database>().clone();
        let covers_dir = self.covers_dir.clone();

        cx.spawn_in(
            window,
            |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
                let mut cx = cx.clone();
                async move {
                    match db.get_recently_added_songs(100).await {
                        Ok(songs) => {
                            let mut cover_groups: HashMap<String, Vec<Song>> = HashMap::new();
                            let mut no_cover_songs: Vec<Song> = Vec::new();

                            for song in songs {
                                if let Some(cover) = &song.cover {
                                    cover_groups.entry(cover.clone()).or_default().push(song);
                                } else {
                                    no_cover_songs.push(song);
                                }
                            }

                            let mut recent_items: Vec<(String, RecentItem)> = Vec::new();

                            for (_cover_hash, group_songs) in cover_groups {
                                let first_song = &group_songs[0];
                                let most_recent = group_songs
                                    .iter()
                                    .map(|s| s.date_added.clone())
                                    .max()
                                    .unwrap_or_default();

                                let artist_name = if let Some(artist_id) = &first_song.artist_id {
                                    db.get_artist_name(artist_id).await.ok().flatten()
                                } else {
                                    None
                                };

                                let cover_uri = first_song.cover.as_ref().and_then(|cover_hash| {
                                    let cover_path = covers_dir.join(cover_hash);
                                    if cover_path.exists() {
                                        Some(format!("!file://{}", cover_path.to_string_lossy()))
                                    } else {
                                        None
                                    }
                                });

                                let year = first_song.date.clone();

                                if group_songs.len() > 1 {
                                    let album_title = if let Some(album_id) = &first_song.album_id {
                                        db.get_album(album_id).await.ok().map(|a| a.title)
                                    } else {
                                        None
                                    };

                                    recent_items.push((
                                        most_recent,
                                        RecentItem::Album {
                                            title: album_title
                                                .unwrap_or_else(|| "Unknown Album".to_string()),
                                            artist_name,
                                            cover_uri,
                                            year,
                                        },
                                    ));
                                } else {
                                    recent_items.push((
                                        most_recent,
                                        RecentItem::Song {
                                            title: first_song.title.clone(),
                                            artist_name,
                                            cover_uri,
                                        },
                                    ));
                                }
                            }

                            for song in no_cover_songs {
                                let date_added = song.date_added.clone();

                                let artist_name = if let Some(artist_id) = &song.artist_id {
                                    db.get_artist_name(artist_id).await.ok().flatten()
                                } else {
                                    None
                                };

                                recent_items.push((
                                    date_added,
                                    RecentItem::Song {
                                        title: song.title.clone(),
                                        artist_name,
                                        cover_uri: None,
                                    },
                                ));
                            }

                            recent_items.sort_by(|a, b| b.0.cmp(&a.0));

                            let items: Vec<RecentItem> =
                                recent_items.into_iter().map(|(_, item)| item).collect();

                            this.update(&mut cx, |this, cx| {
                                this.recently_added = items;
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(e) => {
                            tracing::error!("Failed to load recently added songs: {}", e);
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn calculate_layout(&self) -> (f32, usize) {
        let width = self.container_width.unwrap_or(1000.0);

        let num_items = ((width + GAP_SIZE) / (MIN_COVER_SIZE + GAP_SIZE)).floor() as usize;
        let num_items = num_items.max(1);

        let cover_size = if num_items > 0 {
            ((width - (num_items - 1) as f32 * GAP_SIZE) / num_items as f32)
                .clamp(MIN_COVER_SIZE, MAX_COVER_SIZE)
        } else {
            MIN_COVER_SIZE
        };

        (cover_size, num_items)
    }

    fn scroll_recently_added_left(&mut self, cx: &mut Context<Self>) {
        let (_, items_per_page) = self.calculate_layout();
        if self.recently_added_offset >= items_per_page {
            self.recently_added_offset -= items_per_page;
        } else {
            self.recently_added_offset = 0;
        }
        cx.notify();
    }

    fn scroll_recently_added_right(&mut self, cx: &mut Context<Self>) {
        let (_, items_per_page) = self.calculate_layout();
        let max_offset = self.recently_added.len().saturating_sub(items_per_page);
        if self.recently_added_offset + items_per_page <= max_offset {
            self.recently_added_offset += items_per_page;
        } else {
            self.recently_added_offset = max_offset;
        }
        cx.notify();
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        let bounds = window.bounds();
        let window_width: f32 = bounds.size.width.into();
        // 300 (sidebar) + 16 (gap) + 32 (main padding) + 48 (home padding)
        let estimated_width = window_width - 300.0 - 96.0;
        if estimated_width > 0.0 {
            self.container_width = Some(estimated_width);
        }

        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        let (cover_size, items_per_page) = self.calculate_layout();

        let can_scroll_left = self.recently_added_offset > 0;
        let can_scroll_right =
            self.recently_added_offset + items_per_page < self.recently_added.len();

        let recently_added_content = if self.recently_added.is_empty() {
            h_flex()
                .w_full()
                .child("No Data")
                .text_color(variables.text_secondary)
                .into_any_element()
        } else {
            let visible_items: Vec<_> = self
                .recently_added
                .iter()
                .skip(self.recently_added_offset)
                .take(items_per_page)
                .collect();

            h_flex()
                .w_full()
                .gap(px(GAP_SIZE))
                .children(visible_items.into_iter().map(|item| {
                    let (title, subtitle, cover_uri) = match item {
                        RecentItem::Song {
                            title,
                            artist_name,
                            cover_uri,
                        } => {
                            let artist = artist_name
                                .clone()
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            (title.clone(), artist, cover_uri.clone())
                        }
                        RecentItem::Album {
                            title,
                            artist_name,
                            cover_uri,
                            year,
                        } => {
                            let artist = artist_name
                                .clone()
                                .unwrap_or_else(|| "Unknown Artist".to_string());
                            let subtitle = if let Some(y) = year {
                                format!("{} · {}", y, artist)
                            } else {
                                artist
                            };
                            (title.clone(), subtitle, cover_uri.clone())
                        }
                    };

                    let cover_element = if let Some(uri) = cover_uri {
                        img(uri)
                            .size(px(cover_size))
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    } else {
                        div()
                            .size(px(cover_size))
                            .bg(variables.border)
                            .into_any_element()
                    };

                    v_flex()
                        .w(px(cover_size))
                        .gap(px(8.0))
                        .child(cover_element)
                        .child(
                            v_flex()
                                .gap(px(4.0))
                                .child(
                                    div()
                                        .text_ellipsis()
                                        .overflow_x_hidden()
                                        .max_w(px(cover_size))
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_ellipsis()
                                        .overflow_x_hidden()
                                        .max_w(px(cover_size))
                                        .text_color(variables.text_secondary)
                                        .text_size(px(12.0))
                                        .child(subtitle),
                                ),
                        )
                }))
                .into_any_element()
        };

        let recently_played = v_flex()
            .child(
                h_flex()
                    .child(
                        div()
                            .gap(px(variables.padding_16))
                            .child("Recently Played")
                            .font_bold()
                            .text_size(px(18.0)),
                    )
                    .child(
                        h_flex()
                            .gap(px(variables.padding_8))
                            .child(icon(ARROW_LEFT))
                            .child(icon(ARROW_RIGHT)),
                    )
                    .items_center()
                    .justify_between(),
            )
            .child(
                h_flex()
                    .child("No Data")
                    .text_color(variables.text_secondary),
            )
            .gap(px(variables.padding_16));

        let left_arrow_color = if can_scroll_left {
            variables.text_secondary
        } else {
            variables.text_muted
        };

        let right_arrow_color = if can_scroll_right {
            variables.text_secondary
        } else {
            variables.text_muted
        };

        let recently_added = v_flex()
            .child(
                h_flex()
                    .child(
                        div()
                            .gap(px(variables.padding_16))
                            .child("Recently Added")
                            .font_bold()
                            .text_size(px(18.0)),
                    )
                    .child(
                        h_flex()
                            .gap(px(variables.padding_8))
                            .child(
                                icon(ARROW_LEFT)
                                    .when(can_scroll_left, |this| this.cursor_pointer())
                                    .when(!can_scroll_left, |this| this.cursor_not_allowed())
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            this.scroll_recently_added_left(cx);
                                        }),
                                    )
                                    .text_color(left_arrow_color)
                                    .hover(|this| {
                                        if can_scroll_left {
                                            this.text_color(variables.text)
                                        } else {
                                            this
                                        }
                                    }),
                            )
                            .child(
                                icon(ARROW_RIGHT)
                                    .cursor_pointer()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _event, _window, cx| {
                                            this.scroll_recently_added_right(cx);
                                        }),
                                    )
                                    .text_color(right_arrow_color)
                                    .hover(|this| {
                                        if can_scroll_right {
                                            this.text_color(variables.text)
                                        } else {
                                            this
                                        }
                                    }),
                            ),
                    )
                    .items_center()
                    .justify_between(),
            )
            .child(recently_added_content)
            .gap(px(variables.padding_16));

        div()
            .relative()
            .size_full()
            .child(
                v_flex()
                    .border(px(1.0))
                    .border_color(border_color)
                    .size_full()
                    .paddings(px(variables.padding_24))
                    .gap(px(variables.padding_24))
                    .child(h_flex().text_color(variables.accent).child(
                        r"
                __
 _      _____  / /________  ____ ___  ___
| | /| / / _ \/ / ___/ __ \/ __ `__ \/ _ \
| |/ |/ /  __/ / /__/ /_/ / / / / / /  __/
|__/|__/\___/_/\___/\____/_/ /_/ /_/\___/ ",
                    ))
                    .child(recently_played)
                    .child(recently_added),
            )
            .child(Title::new("Home", self.hovered))
    }
}
