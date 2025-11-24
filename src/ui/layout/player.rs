use gpui::*;

use crate::{
    data::config::Config,
    media::{
        playback::PlaybackContext,
        queue::{Queue, RepeatMode},
    },
    ui::{
        components::{
            button::Button,
            div::{flex_col, flex_row},
            icons::{icon::icon, icons::*},
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

    fn format_time(seconds: f32) -> String {
        let mins = (seconds / 60.0).floor() as i32;
        let secs = (seconds % 60.0).floor() as i32;
        format!("{:02}:{:02}", mins, secs)
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

        let current_track = cx.global::<Queue>().current().map(|item| {
            (
                item.title
                    .clone()
                    .unwrap_or_else(|| "Unknown Track".to_string()),
                item.artist_name
                    .clone()
                    .unwrap_or_else(|| "Unknown Artist".to_string()),
            )
        });

        let is_playing = cx.global::<PlaybackContext>().is_playing();
        let volume = cx.global::<PlaybackContext>().get_volume();
        let repeat_mode = cx.global::<Queue>().repeat_mode();
        let _is_shuffle = cx.global::<Queue>().is_shuffle();

        let play_button = Button::new("play_pause")
            .child(if is_playing { icon(PAUSE) } else { icon(PLAY) })
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.update_global::<PlaybackContext, _>(|playback, _cx| {
                    playback.toggle_play_pause();
                });
                cx.notify();
            }));

        let prev_button = Button::new("previous")
            .child(icon(PREVIOUS))
            .on_click(cx.listener(|_this, _event, _window, cx| {
                let next_item = cx.update_global::<Queue, _>(|queue, _cx| {
                    queue.previous().map(|item| {
                        (
                            item.path.clone(),
                            item.replaygain_track_gain,
                            item.replaygain_track_peak,
                        )
                    })
                });

                if let Some((path, rg_gain, rg_peak)) = next_item {
                    let config = cx.global::<Config>().clone();
                    cx.update_global::<PlaybackContext, _>(|playback, _cx| {
                        if let Err(e) =
                            playback.load_file_with_replaygain(&path, &config, rg_gain, rg_peak)
                        {
                            tracing::error!("Failed to load previous track: {}", e);
                        } else {
                            playback.play();
                        }
                    });
                }
                cx.notify();
            }));

        let next_button = Button::new("next").child(icon(NEXT)).on_click(cx.listener(
            |_this, _event, _window, cx| {
                let next_item = cx.update_global::<Queue, _>(|queue, _cx| {
                    queue.next().map(|item| {
                        (
                            item.path.clone(),
                            item.replaygain_track_gain,
                            item.replaygain_track_peak,
                        )
                    })
                });

                if let Some((path, rg_gain, rg_peak)) = next_item {
                    let config = cx.global::<Config>().clone();
                    cx.update_global::<PlaybackContext, _>(|playback, _cx| {
                        if let Err(e) =
                            playback.load_file_with_replaygain(&path, &config, rg_gain, rg_peak)
                        {
                            tracing::error!("Failed to load next track: {}", e);
                        } else {
                            playback.play();
                        }
                    });
                }
                cx.notify();
            },
        ));

        let shuffle_button = Button::new("shuffle")
            .child(icon(SHUFFLE))
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
            .child(icon(repeat_icon))
            .on_click(cx.listener(|_this, _event, _window, cx| {
                cx.update_global::<Queue, _>(|queue, _cx| {
                    queue.cycle_repeat();
                });
                cx.notify();
            }));

        let controls = flex_row()
            .gap(px(variables.padding_8))
            .items_center()
            .child(shuffle_button)
            .child(prev_button)
            .child(play_button)
            .child(next_button)
            .child(repeat_button);

        let track_info = if let Some((title, artist)) = current_track {
            flex_col()
                .gap(px(4.0))
                .child(div().text_sm().font_weight(FontWeight::BOLD).child(title))
                .child(div().text_xs().text_color(rgb(0x999999)).child(artist))
        } else {
            flex_col().gap(px(4.0)).child(
                div()
                    .text_sm()
                    .text_color(rgb(0x666666))
                    .child("No track loaded"),
            )
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
            .child(icon(volume_icon))
            .child(
                slider()
                    .id("volume-slider")
                    .w(px(150.0))
                    .h(px(16.0))
                    .value(volume)
                    .on_change(|value, _window, cx| {
                        let mut config = cx.global::<Config>().clone();
                        cx.update_global::<PlaybackContext, _>(|playback, _cx| {
                            playback.set_volume_and_save(value, &mut config);
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
                    .gap(px(variables.padding_8))
                    .child(
                        flex_row()
                            .h(px(36.0))
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(track_info)
                            .child(controls)
                            .child(volume_display),
                    ),
            )
            .child(Title::new("Player", self.hovered))
    }
}
