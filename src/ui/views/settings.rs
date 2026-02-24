use gpui::{Context, IntoElement, Render, *};

use crate::data::config::Config;
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::switch::Switch;
use crate::ui::variables::Variables;

pub struct SettingsView {}

impl SettingsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Config>(|_this, cx| cx.notify())
            .detach();
        Self {}
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let telemetry = cx.global::<Config>().get().telemetry;
        let discord_rpc = cx.global::<Config>().get().discord_rpc;
        let normalization = cx.global::<Config>().get().audio.normalization;

        flex_col()
            .size_full()
            .p(px(variables.padding_24))
            .gap(px(variables.padding_24))
            .child(
                flex_col()
                    .gap(px(variables.padding_16))
                    .child(
                        div()
                            .text_color(variables.text)
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child("General"),
                    )
                    .child(
                        flex_row()
                            .gap(px(variables.padding_8))
                            .child(Switch::new("telemetry-switch", telemetry).on_change(
                                move |value, _window, cx| {
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        config.set(|s| s.telemetry = value);
                                    });
                                },
                            ))
                            .child(div().text_color(variables.text_secondary).child("Telemetry")),
                    )
                    .child(
                        flex_row()
                            .gap(px(variables.padding_8))
                            .child(Switch::new("discord-rpc-switch", discord_rpc).on_change(
                                move |value, _window, cx| {
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        config.set(|s| s.discord_rpc = value);
                                    });
                                },
                            ))
                            .child(div().text_color(variables.text_secondary).child("Discord RPC")),
                    ),
            )
            .child(
                flex_col()
                    .gap(px(variables.padding_16))
                    .child(
                        div()
                            .text_color(variables.text)
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Audio"),
                    )
                    .child(
                        flex_row()
                            .gap(px(variables.padding_8))
                            .child(
                                Switch::new("normalization-switch", normalization).on_change(
                                    move |value, _window, cx| {
                                        cx.update_global::<Config, _>(|config, _cx| {
                                            config.set(|s| s.audio.normalization = value);
                                        });
                                    },
                                ),
                            )
                            .child(div().text_color(variables.text_secondary).child("Normalizer")),
                    ),
            )
    }
}
