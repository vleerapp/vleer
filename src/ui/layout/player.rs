use gpui::{prelude::FluentBuilder, *};

use crate::{
    data::{config::Config, state::State},
    media::{
        playback::Playback,
        queue::{Queue, RepeatMode},
    },
    ui::{
        components::{
            button::Button,
            div::{flex_col, flex_row},
            icons::{icon::icon, icons::*},
            progress_bar::progress_slider,
            slider::slider,
            title::Title,
        },
        variables::Variables,
    },
};

pub struct Player {
    pub hovered: bool,
}

impl Player {
    pub fn new() -> Self {
        Self { hovered: false }
    }
}

impl Render for Player {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        let current_track = cx.global::<Queue>().current().map(|song| {
            let state = cx.global::<State>();

            let artist_name = song
                .artist_id
                .as_ref()
                .and_then(|id| {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(state.get_artist(id))
                    })
                })
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown Artist".to_string());

            let cover_uri = song.cover_uri();

            (song.title.clone(), artist_name, cover_uri)
        });

        let is_playing = cx.global::<Playback>().is_playing();
        let volume = cx.global::<Playback>().get_volume();
        let repeat_mode = cx.global::<Queue>().repeat_mode();
        let is_shuffle = cx.global::<Queue>().is_shuffle();

        let play_button = Button::new("play_pause")
            .group("playpause-button")
            .child(if is_playing {
                icon(PAUSE).group_hover("playpause-button", |s| s.text_color(variables.text))
            } else {
                icon(PLAY).group_hover("playpause-button", |s| s.text_color(variables.text))
            })
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.update_global::<Playback, _>(|playback, cx| {
                    playback.play_pause(cx);
                });
                cx.notify();
            }));

        let prev_button = Button::new("previous")
            .group("previous-button")
            .child(icon(PREVIOUS).group_hover("previous-button", |s| s.text_color(variables.text)))
            .on_click(cx.listener(|_this, _event, _window, cx| {
                if let Err(e) = Queue::previous(cx) {
                    tracing::error!("Failed to play previous: {}", e);
                }
                cx.notify();
            }));

        let next_button = Button::new("next")
            .group("next-button")
            .child(icon(NEXT).group_hover("next-button", |s| s.text_color(variables.text)))
            .on_click(cx.listener(|_this, _event, _window, cx| {
                if let Err(e) = Queue::next(cx) {
                    tracing::error!("Failed to play next: {}", e);
                }
                cx.notify();
            }));

        let shuffle_button = Button::new("shuffle")
            .group("shuffle-button")
            .child(
                icon(SHUFFLE)
                    .when(is_shuffle, |s| s.text_color(variables.accent))
                    .group_hover("shuffle-button", |s| {
                        if !is_shuffle {
                            s.text_color(variables.text)
                        } else {
                            s
                        }
                    }),
            )
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.update_global::<Queue, _>(|queue, _cx| {
                    queue.toggle_shuffle();
                });
                cx.notify();
            }));

        let repeat_icon = match repeat_mode {
            RepeatMode::Off => REPLAY,
            RepeatMode::All => REPLAY,
            RepeatMode::One => REPLAY_1,
        };

        let repeat_button = Button::new("repeat")
            .group("repeat-button")
            .child(
                icon(repeat_icon)
                    .when(
                        repeat_mode == RepeatMode::All || repeat_mode == RepeatMode::One,
                        |s| s.text_color(variables.accent),
                    )
                    .group_hover("repeat-button", |s| {
                        if repeat_mode == RepeatMode::Off {
                            s.text_color(variables.text)
                        } else {
                            s
                        }
                    }),
            )
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.update_global::<Queue, _>(|queue, _cx| {
                    queue.cycle_repeat();
                });
                cx.notify();
            }));

        let controls = flex_row()
            .gap(px(variables.padding_8))
            .items_center()
            .justify_center()
            .child(shuffle_button)
            .child(prev_button)
            .child(play_button)
            .child(next_button)
            .child(repeat_button);

        let track_info = if let Some((title, artist, cover_uri)) = current_track {
            flex_row()
                .gap(px(variables.padding_8))
                .items_center()
                .child(if let Some(uri) = cover_uri {
                    img(format!("{}?size=50", uri))
                        .size(px(36.0))
                        .object_fit(ObjectFit::Cover)
                        .into_any_element()
                } else {
                    div()
                        .size(px(36.0))
                        .bg(variables.element)
                        .into_any_element()
                })
                .child(
                    flex_col()
                        .gap(px(2.0))
                        .child(div().font_weight(FontWeight(500.0)).child(title))
                        .child(div().text_color(variables.text_secondary).child(artist)),
                )
                .into_any_element()
        } else {
            flex_row()
                .gap(px(variables.padding_8))
                .items_center()
                .child(
                    div()
                        .size(px(36.0))
                        .bg(variables.element)
                        .into_any_element(),
                )
                .child(
                    flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .font_weight(FontWeight(500.0))
                                .text_color(variables.text_secondary)
                                .child("No Song Playing"),
                        )
                        .child(div().text_color(variables.text_muted).child("")),
                )
                .into_any_element()
        };

        let volume_icon = match volume {
            v if v == 0.0 => VOLUME_MUTE,
            v if v <= 0.25 => VOLUME_1,
            v if v <= 0.50 => VOLUME_2,
            v if v <= 0.75 => VOLUME_3,
            v if v <= 1.0 => VOLUME_4,
            _ => VOLUME_1,
        };

        let volume_display = flex_row()
            .gap(px(variables.padding_8))
            .items_center()
            .justify_end()
            .child(icon(volume_icon))
            .child(
                slider()
                    .id("volume-slider")
                    .w(px(150.0))
                    .h(px(16.0))
                    .value(volume)
                    .on_change(|value, _window, cx| {
                        cx.update_global::<Playback, _>(|playback, _cx| {
                            playback.set_volume(value);
                        });

                        cx.update_global::<Config, _>(|config, _cx| {
                            config.set_volume(value);
                        });
                    }),
            );

        div()
            .relative()
            .size_full()
            .child(
                flex_col()
                    .border(px(1.0))
                    .border_color(border_color)
                    .h_full()
                    .p(px(variables.padding_16))
                    .gap(px(variables.padding_16))
                    .child(
                        flex_row()
                            .h(px(36.0))
                            .w_full()
                            .items_center()
                            .gap(px(variables.padding_16))
                            .child(div().flex_1().min_w_0().child(track_info))
                            .child(div().flex_1().child(controls))
                            .child(div().flex_1().min_w_0().child(volume_display)),
                    )
                    .child(
                        progress_slider()
                            .id("playback-progress")
                            .w_full()
                            .h(px(16.0))
                            .current_time(cx.global::<Playback>().get_position())
                            .duration(
                                cx.global::<Queue>()
                                    .current()
                                    .map(|s| s.duration as f32)
                                    .unwrap_or(0.0),
                            )
                            .on_seek(|value, window, cx| {
                                let duration = cx
                                    .global::<Queue>()
                                    .current()
                                    .map(|s| s.duration as f32)
                                    .unwrap_or(0.0);

                                if duration > 0.0 {
                                    let seek_time = value * duration;

                                    cx.update_global::<Playback, _>(|playback, _cx| {
                                        if let Err(e) = playback.seek(seek_time) {
                                            tracing::error!("Failed to seek: {}", e);
                                        }
                                    });

                                    window.refresh();
                                }
                            }),
                    ),
            )
            .child(Title::new("Player", self.hovered))
    }
}
