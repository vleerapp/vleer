use gpui::{Context, Entity, IntoElement, Render, *};

use crate::data::config::Config;
use crate::media::playback::Playback;
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::icon::icon;
use crate::ui::components::icons::icons::{PLUS, X};
use crate::ui::components::input::{InputEvent, TextInput};
use crate::ui::components::slider::slider;
use crate::ui::components::switch::Switch;
use crate::ui::variables::Variables;

#[derive(IntoElement)]
struct ScanPathsSection;

impl RenderOnce for ScanPathsSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let paths = cx.global::<Config>().get().scan.paths.clone();

        flex_col()
            .gap(px(variables.padding_16))
            .items_start()
            .child(
                flex_col()
                    .gap(px(variables.padding_8))
                    .max_w(px(650.0))
                    .w_full()
                    .children(paths.into_iter().enumerate().map(move |(i, path)| {
                        flex_row()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .p(px(variables.padding_16))
                            .bg(variables.element)
                            .child(
                                div()
                                    .text_color(variables.text)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(path.clone()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("remove-path-{i}")))
                                    .cursor_pointer()
                                    .child(
                                        icon(X)
                                            .text_color(variables.text_secondary)
                                            .hover(|s| s.text_color(variables.text)),
                                    )
                                    .on_click(move |_event, _window, cx| {
                                        cx.update_global::<Config, _>(|config, _cx| {
                                            config.set(|s| {
                                                s.scan.paths.retain(|p| p != &path);
                                            });
                                        });
                                    }),
                            )
                    })),
            )
            .child(
                div()
                    .id("add-scan-path")
                    .cursor_pointer()
                    .group("add-scan-path-btn")
                    .child(
                        flex_row()
                            .items_center()
                            .gap(px(variables.padding_8))
                            .child(
                                div()
                                    .id("add-scan-path-text")
                                    .text_color(variables.text_secondary)
                                    .group_hover("add-scan-path-btn", |s| {
                                        s.text_color(variables.text)
                                    })
                                    .child("Add new path"),
                            )
                            .child(icon(PLUS).group_hover("add-scan-path-btn", |s| {
                                s.text_color(variables.text)
                            })),
                    )
                    .on_click(move |_event, _window, cx| {
                        let options = PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: false,
                            prompt: None,
                        };
                        let receiver = cx.prompt_for_paths(options);
                        cx.spawn(async move |cx| {
                            if let Ok(Ok(Some(paths))) = receiver.await {
                                if let Some(path) = paths.into_iter().next() {
                                    if let Some(path_str) = path.to_str() {
                                        let path_str = path_str.to_string();
                                        let _ = cx.update_global::<Config, _>(
                                            |config: &mut Config, _cx| {
                                                config.set(|s| {
                                                    if !s.scan.paths.contains(&path_str) {
                                                        s.scan.paths.push(path_str);
                                                    }
                                                });
                                            },
                                        );
                                    }
                                }
                            }
                        })
                        .detach();
                    }),
            )
    }
}

#[derive(IntoElement)]
struct EqSection {
    gain_inputs: Vec<Entity<TextInput>>,
    freq_inputs: Vec<Entity<TextInput>>,
    q_inputs: Vec<Entity<TextInput>>,
}

impl RenderOnce for EqSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let eq = cx.global::<Config>().get().equalizer.clone();

        let label_w = px(52.0);
        let cell_w = px(60.0);
        let cell_h = px(24.0);
        let slider_h = px(150.0);
        let gap_small = px(2.0);
        let gap_large = px(8.0);

        let label_cell = |text: &'static str| {
            div()
                .w(label_w)
                .h(cell_h)
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_end()
                .pr(px(14.0))
                .text_color(variables.text_secondary)
                .child(text)
        };

        let gain_inputs = self.gain_inputs.clone();

        div()
            .bg(variables.element)
            .p(px(variables.padding_16))
            .flex_shrink_0()
            .child(
                flex_col()
                    .gap(gap_small)
                    .child(
                        flex_col()
                            .gap(gap_large)
                            .child(
                                flex_row()
                                    .items_start()
                                    .gap(gap_small)
                                    .child(label_cell("Gain"))
                                    .children(self.gain_inputs.into_iter().map(|entity| {
                                        div()
                                            .w(cell_w)
                                            .h(cell_h)
                                            .flex_shrink_0()
                                            .overflow_hidden()
                                            .child(entity)
                                    })),
                            )
                            .child(
                                flex_row()
                                    .items_start()
                                    .gap(gap_small)
                                    .child(div().w(label_w).h(slider_h).flex_shrink_0())
                                    .children((0..10usize).map(|i| {
                                        let gain_db = eq.gains.get(i).copied().unwrap_or(0.0);
                                        let slider_val = (gain_db + 12.0) / 24.0;
                                        let gain_inputs = gain_inputs.clone();
                                        slider()
                                            .id(SharedString::from(format!("eq-slider-{i}")))
                                            .w(cell_w)
                                            .h(slider_h)
                                            .vertical()
                                            .render_full(true)
                                            .value(slider_val.clamp(0.0, 1.0))
                                            .on_change(move |val, _win, cx| {
                                                let new_gain = val * 24.0 - 12.0;
                                                let (enabled, gains, q_values) = cx
                                                    .update_global::<Config, _>(|config, _cx| {
                                                        config.set(|s| {
                                                            if let Some(g) =
                                                                s.equalizer.gains.get_mut(i)
                                                            {
                                                                *g = new_gain;
                                                            }
                                                        });
                                                        let eq = &config.get().equalizer;
                                                        (
                                                            eq.enabled,
                                                            eq.gains.clone(),
                                                            eq.q_values.clone(),
                                                        )
                                                    });
                                                if enabled {
                                                    cx.update_global::<Playback, _>(
                                                        |playback, _cx| {
                                                            playback.apply_eq_settings(
                                                                &gains, &q_values,
                                                            );
                                                        },
                                                    );
                                                }
                                                if let Some(input) = gain_inputs.get(i) {
                                                    input.update(cx, |inp, cx| {
                                                        inp.set_text(
                                                            format!("{:.1}", new_gain),
                                                            cx,
                                                        );
                                                    });
                                                }
                                            })
                                    })),
                            )
                            .child(
                                flex_row()
                                    .items_start()
                                    .gap(gap_small)
                                    .child(label_cell("Freq"))
                                    .children(self.freq_inputs.into_iter().map(|entity| {
                                        div()
                                            .w(cell_w)
                                            .h(cell_h)
                                            .flex_shrink_0()
                                            .overflow_hidden()
                                            .child(entity)
                                    })),
                            ),
                    )
                    .child(
                        flex_row()
                            .items_start()
                            .gap(gap_small)
                            .child(label_cell("Q"))
                            .children(self.q_inputs.into_iter().map(|entity| {
                                div()
                                    .w(cell_w)
                                    .h(cell_h)
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .child(entity)
                            })),
                    ),
            )
    }
}

pub struct SettingsView {
    gain_inputs: Vec<Entity<TextInput>>,
    freq_inputs: Vec<Entity<TextInput>>,
    q_inputs: Vec<Entity<TextInput>>,
}

impl SettingsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Config>(|_this, cx| cx.notify())
            .detach();

        let eq = cx.global::<Config>().get().equalizer.clone();
        let element_hover = cx.global::<Variables>().element_hover;
        let text_secondary = cx.global::<Variables>().text_secondary;

        let gain_inputs: Vec<Entity<TextInput>> = (0..10)
            .map(|i| {
                let gain = eq.gains.get(i).copied().unwrap_or(0.0);
                cx.new(|cx| {
                    TextInput::new(cx, "")
                        .with_text(format!("{:.1}", gain))
                        .with_background(element_hover)
                        .with_text_color(text_secondary)
                        .with_height(px(24.0))
                        .centered()
                        .with_validator(|s| {
                            if s.is_empty() {
                                return true;
                            }
                            s.parse::<f32>()
                                .map(|v| v >= -12.0 && v <= 12.0)
                                .unwrap_or(false)
                        })
                })
            })
            .collect();

        let freq_inputs: Vec<Entity<TextInput>> = (0..10)
            .map(|i| {
                let freq = eq.frequencies.get(i).copied().unwrap_or(0);
                cx.new(|cx| {
                    TextInput::new(cx, "")
                        .with_text(format!("{}", freq))
                        .with_background(element_hover)
                        .with_text_color(text_secondary)
                        .with_height(px(24.0))
                        .centered()
                        .with_validator(|s| {
                            if s.is_empty() {
                                return true;
                            }
                            s.parse::<u32>()
                                .map(|v| v >= 20 && v <= 20000)
                                .unwrap_or(false)
                        })
                })
            })
            .collect();

        let q_inputs: Vec<Entity<TextInput>> = (0..10)
            .map(|i| {
                let q = eq.q_values.get(i).copied().unwrap_or(1.461);
                cx.new(|cx| {
                    TextInput::new(cx, "")
                        .with_text(format!("{:.2}", q))
                        .with_background(element_hover)
                        .with_text_color(text_secondary)
                        .with_height(px(24.0))
                        .centered()
                        .with_validator(|s| {
                            if s.is_empty() {
                                return true;
                            }
                            s.parse::<f32>()
                                .map(|v| v >= 0.1 && v <= 10.0)
                                .unwrap_or(false)
                        })
                })
            })
            .collect();

        for (i, input) in gain_inputs.iter().enumerate() {
            cx.subscribe(input, move |_this, _entity, event, cx| {
                if let InputEvent::Submit(text) = event {
                    if let Ok(new_gain) = text.parse::<f32>() {
                        let new_gain = new_gain.clamp(-12.0, 12.0);
                        let (enabled, gains, q_values) =
                            cx.update_global::<Config, _>(|config, _cx| {
                                config.set(|s| {
                                    if let Some(g) = s.equalizer.gains.get_mut(i) {
                                        *g = new_gain;
                                    }
                                });
                                let eq = &config.get().equalizer;
                                (eq.enabled, eq.gains.clone(), eq.q_values.clone())
                            });
                        if enabled {
                            cx.update_global::<Playback, _>(|playback, _cx| {
                                playback.apply_eq_settings(&gains, &q_values);
                            });
                        }
                    }
                }
            })
            .detach();
        }

        for (i, input) in q_inputs.iter().enumerate() {
            cx.subscribe(input, move |_this, _entity, event, cx| {
                if let InputEvent::Submit(text) = event {
                    if let Ok(new_q) = text.parse::<f32>() {
                        let new_q = new_q.max(0.1);
                        let (enabled, gains, q_values) =
                            cx.update_global::<Config, _>(|config, _cx| {
                                config.set(|s| {
                                    if let Some(q) = s.equalizer.q_values.get_mut(i) {
                                        *q = new_q;
                                    }
                                });
                                let eq = &config.get().equalizer;
                                (eq.enabled, eq.gains.clone(), eq.q_values.clone())
                            });
                        if enabled {
                            cx.update_global::<Playback, _>(|playback, _cx| {
                                playback.apply_eq_settings(&gains, &q_values);
                            });
                        }
                    }
                }
            })
            .detach();
        }

        Self {
            gain_inputs,
            freq_inputs,
            q_inputs,
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let telemetry = cx.global::<Config>().get().telemetry;
        let discord_rpc = cx.global::<Config>().get().discord_rpc;
        let normalization = cx.global::<Config>().get().audio.normalization;
        let eq_enabled = cx.global::<Config>().get().equalizer.enabled;

        flex_col()
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
                            .child(
                                div()
                                    .text_color(variables.text_secondary)
                                    .child("Telemetry"),
                            ),
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
                            .child(
                                div()
                                    .text_color(variables.text_secondary)
                                    .child("Discord RPC"),
                            ),
                    ),
            )
            .child(
                flex_col()
                    .items_start()
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
                            .child(
                                div()
                                    .text_color(variables.text_secondary)
                                    .child("Normalization"),
                            ),
                    )
                    .child(
                        flex_row()
                            .gap(px(variables.padding_8))
                            .child(Switch::new("eq-enabled-switch", eq_enabled).on_change(
                                move |value, _window, cx| {
                                    let (gains, q_values) =
                                        cx.update_global::<Config, _>(|config, _cx| {
                                            config.set(|s| s.equalizer.enabled = value);
                                            let eq = &config.get().equalizer;
                                            (eq.gains.clone(), eq.q_values.clone())
                                        });
                                    cx.update_global::<Playback, _>(|playback, _cx| {
                                        if value {
                                            playback.apply_eq_settings(&gains, &q_values);
                                        } else {
                                            playback.set_eq_enabled(false);
                                        }
                                    });
                                },
                            ))
                            .child(
                                div()
                                    .text_color(variables.text_secondary)
                                    .child("Equalizer"),
                            ),
                    )
                    .child(EqSection {
                        gain_inputs: self.gain_inputs.clone(),
                        freq_inputs: self.freq_inputs.clone(),
                        q_inputs: self.q_inputs.clone(),
                    }),
            )
            .child(
                flex_col()
                    .gap(px(variables.padding_16))
                    .child(
                        div()
                            .text_color(variables.text)
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Scan Paths"),
                    )
                    .child(ScanPathsSection),
            )
    }
}
