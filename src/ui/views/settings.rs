use gpui::{Context, Entity, IntoElement, Render, prelude::FluentBuilder as _, *};

use crate::data::config::{Config, FftSize, UpdateChannel};
use crate::data::db::repo::Database;
use crate::data::scanner::{Scanner, expand_tilde};
use crate::media::playback::Playback;
use crate::services::lastfm::{LastfmAuthStatus, LastfmClient};
use crate::ui::components::button::{Button, ButtonVariant};
use crate::ui::components::context_menu::{LibraryDataChanged, QueueChanged};
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::{self, LINK, icon};
use crate::ui::components::input::{InputEvent, TextInput};
use crate::ui::components::scrollbar::ScrollableElement;
use crate::ui::components::slider::slider;
use crate::ui::components::switch::Switch;
use crate::ui::variables::Variables;
use crate::updater::{UpdateStatus, Updater, is_managed_externally, run_check_in_background};
use tracing::error;

const CONTENT_WIDTH: f32 = 704.0;
const APP_ICON: &str = "!bundled:images/icon-512.png";
const TELEMETRY_DASHBOARD: &str =
    "https://graf.wireway.ch/public-dashboards/c518e42c7bc14c5ba95040671fb9e467";

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Appearance,
    Playback,
    Privacy,
    Integrations,
    About,
}

impl SettingsTab {
    const ALL: [Self; 6] = [
        Self::General,
        Self::Appearance,
        Self::Playback,
        Self::Privacy,
        Self::Integrations,
        Self::About,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Playback => "playback",
            Self::Privacy => "privacy",
            Self::Integrations => "integrations",
            Self::About => "about",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Playback => "Playback",
            Self::Privacy => "Privacy",
            Self::Integrations => "Integrations",
            Self::About => "About",
        }
    }
}

fn setting(
    variables: &Variables,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> Div {
    flex_row()
        .justify_between()
        .gap(px(variables.padding_16))
        .w_full()
        .child(
            flex_col()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap(px(variables.padding_8))
                .child(div().text_color(variables.text).child(title.into()))
                .child(
                    div()
                        .text_sm()
                        .line_height(px(16.0))
                        .text_color(variables.text_secondary)
                        .child(description.into()),
                ),
        )
        .child(div().flex_shrink_0().child(control))
}

#[derive(IntoElement)]
struct Group {
    variables: Variables,
    title: &'static str,
    rows: Div,
    count: usize,
}

fn group(variables: &Variables, title: &'static str) -> Group {
    Group {
        variables: *variables,
        title,
        rows: flex_col().gap(px(variables.padding_16)).w_full(),
        count: 0,
    }
}

impl ParentElement for Group {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        for element in elements {
            if self.count > 0 {
                self.rows.extend([div()
                    .h(px(1.0))
                    .w_full()
                    .bg(self.variables.element_hover)
                    .into_any_element()]);
            }
            self.rows.extend([element]);
            self.count += 1;
        }
    }
}

impl RenderOnce for Group {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        flex_col()
            .gap(px(self.variables.padding_16))
            .w_full()
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_sm()
                    .text_color(self.variables.text_secondary)
                    .child(self.title),
            )
            .child(self.rows)
    }
}

fn related(variables: &Variables) -> Div {
    flex_col().gap(px(variables.padding_16)).w_full()
}

fn page(variables: &Variables) -> Div {
    flex_col()
        .gap(px(variables.padding_32))
        .w_full()
        .max_w(px(CONTENT_WIDTH))
}

fn add_scan_path(window: &mut Window, cx: &mut App) {
    let options = PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    };
    let receiver = cx.prompt_for_paths(options);
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = receiver.await
            && let Some(path) = paths.into_iter().next()
            && let Some(path_str) = path.to_str()
        {
            let path_str = path_str.to_string();
            cx.update_global::<Config, _>(|config: &mut Config, _cx| {
                config.set(|s| {
                    if !s.library.paths.contains(&path_str) {
                        s.library.paths.push(path_str);
                    }
                });
            });
            let _ = window_handle.update(cx, |_, window, _cx| {
                window.refresh();
            });
        }
    })
    .detach();
}

fn remove_scan_path(path: String, cx: &mut App) {
    cx.update_global::<Config, _>(|config, _cx| {
        config.set(|s| {
            s.library.paths.retain(|p| p != &path);
        });
    });

    let db = cx.global::<Database>().clone();
    let scanner = cx.global::<Scanner>().clone();
    let dir = expand_tilde(&path).to_string_lossy().into_owned();
    cx.spawn(async move |cx| {
        let bg = cx.background_executor().clone();
        let deleted = bg
            .spawn(async move { scanner.delete_path_exclusive(&db, &dir).await })
            .await;
        match deleted {
            Ok(n) if n > 0 => {
                cx.update(|cx| {
                    cx.set_global(LibraryDataChanged);
                    cx.set_global(QueueChanged);
                });
            }
            Ok(_) => {}
            Err(e) => {
                error!("failed to delete songs under removed scan path: {e}");
            }
        }
    })
    .detach();
}

#[derive(IntoElement)]
struct ScanPathsList;

impl RenderOnce for ScanPathsList {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let paths = cx.global::<Config>().get().library.paths.clone();

        flex_col().gap(px(variables.padding_8)).w_full().children(
            paths.into_iter().enumerate().map(move |(i, path)| {
                flex_row()
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
                                icon(icons::X)
                                    .text_color(variables.text_secondary)
                                    .hover(|s| s.text_color(variables.text)),
                            )
                            .on_click(move |_event, _window, cx| {
                                remove_scan_path(path.clone(), cx);
                            }),
                    )
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
        let eq = cx.global::<Config>().get().playback.equalizer.clone();

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
            .self_start()
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
                                                            if let Some(g) = s
                                                                .playback
                                                                .equalizer
                                                                .gains
                                                                .get_mut(i)
                                                            {
                                                                *g = new_gain;
                                                            }
                                                        });
                                                        let eq = &config.get().playback.equalizer;
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

fn start_lastfm_connect(cx: &mut App) {
    let client = cx.global::<LastfmClient>().clone();
    cx.set_global(LastfmAuthStatus::Connecting);

    cx.spawn(async move |cx| {
        let bg = cx.background_executor().clone();

        let token = {
            let client = client.clone();
            bg.spawn(async move { client.request_token() }).await
        };

        let token = match token {
            Ok(t) => t,
            Err(e) => {
                cx.update(|app| app.set_global(LastfmAuthStatus::Error(e.to_string())));
                return;
            }
        };

        cx.update(|app| {
            app.open_url(&LastfmClient::auth_url(&token));
            app.set_global(LastfmAuthStatus::WaitingForBrowser);
        });

        for _ in 0..40 {
            bg.timer(std::time::Duration::from_secs(3)).await;

            let result = {
                let client = client.clone();
                let token = token.clone();
                bg.spawn(async move { client.get_session(&token) }).await
            };

            if let Ok((session_key, username)) = result {
                cx.update(|app| {
                    app.update_global::<Config, _>(|config, _cx| {
                        config.set(|s| {
                            s.integrations.lastfm.session_key = Some(session_key);
                            s.integrations.lastfm.username = Some(username);
                        });
                    });
                    app.set_global(LastfmAuthStatus::Idle);
                });
                return;
            }
        }

        cx.update(|app| {
            app.set_global(LastfmAuthStatus::Error(
                "Authorization timed out".to_string(),
            ));
        });
    })
    .detach();
}

pub struct SettingsView {
    tab: SettingsTab,
    gain_inputs: Vec<Entity<TextInput>>,
    freq_inputs: Vec<Entity<TextInput>>,
    q_inputs: Vec<Entity<TextInput>>,
    lastfm_threshold_input: Entity<TextInput>,
}

impl SettingsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Config>(|this, cx| {
            let eq = cx.global::<Config>().get().playback.equalizer.clone();
            for (i, input) in this.gain_inputs.iter().enumerate() {
                let gain = eq.gains.get(i).copied().unwrap_or(0.0);
                input.update(cx, |inp, cx| {
                    inp.set_text(format!("{:.1}", gain), cx);
                });
            }
            for (i, input) in this.freq_inputs.iter().enumerate() {
                let freq = eq.frequencies.get(i).copied().unwrap_or(0);
                input.update(cx, |inp, cx| {
                    inp.set_text(format!("{}", freq), cx);
                });
            }
            for (i, input) in this.q_inputs.iter().enumerate() {
                let q = eq
                    .q_values
                    .get(i)
                    .copied()
                    .unwrap_or(crate::media::equalizer::Q_DEFAULT);
                input.update(cx, |inp, cx| {
                    inp.set_text(format!("{:.2}", q), cx);
                });
            }
            let threshold = cx
                .global::<Config>()
                .get()
                .integrations
                .lastfm
                .scrobble_threshold;
            this.lastfm_threshold_input.update(cx, |inp, cx| {
                inp.set_text(format!("{:.0}", threshold * 100.0), cx);
            });
            cx.notify();
        })
        .detach();

        cx.observe_global::<LastfmAuthStatus>(|_this, cx| {
            cx.notify();
        })
        .detach();

        let eq = cx.global::<Config>().get().playback.equalizer.clone();
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
                                .map(|v| (-12.0..=12.0).contains(&v))
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
                                .map(|v| (20..=20000).contains(&v))
                                .unwrap_or(false)
                        })
                })
            })
            .collect();

        let q_inputs: Vec<Entity<TextInput>> = (0..10)
            .map(|i| {
                let q = eq
                    .q_values
                    .get(i)
                    .copied()
                    .unwrap_or(crate::media::equalizer::Q_DEFAULT);
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
                                .map(|v| (0.1..=10.0).contains(&v))
                                .unwrap_or(false)
                        })
                })
            })
            .collect();

        for (i, input) in gain_inputs.iter().enumerate() {
            cx.subscribe(input, move |_this, _entity, event, cx| {
                if let InputEvent::Submit(text) = event
                    && let Ok(new_gain) = text.parse::<f32>()
                {
                    let new_gain = new_gain.clamp(-12.0, 12.0);
                    let (enabled, gains, q_values) =
                        cx.update_global::<Config, _>(|config, _cx| {
                            config.set(|s| {
                                if let Some(g) = s.playback.equalizer.gains.get_mut(i) {
                                    *g = new_gain;
                                }
                            });
                            let eq = &config.get().playback.equalizer;
                            (eq.enabled, eq.gains.clone(), eq.q_values.clone())
                        });
                    if enabled {
                        cx.update_global::<Playback, _>(|playback, _cx| {
                            playback.apply_eq_settings(&gains, &q_values);
                        });
                    }
                }
            })
            .detach();
        }

        for (i, input) in q_inputs.iter().enumerate() {
            cx.subscribe(input, move |_this, _entity, event, cx| {
                if let InputEvent::Submit(text) = event
                    && let Ok(new_q) = text.parse::<f32>()
                {
                    let new_q = new_q.clamp(
                        crate::media::equalizer::Q_MIN,
                        crate::media::equalizer::Q_MAX,
                    );
                    let (enabled, gains, q_values) =
                        cx.update_global::<Config, _>(|config, _cx| {
                            config.set(|s| {
                                if let Some(q) = s.playback.equalizer.q_values.get_mut(i) {
                                    *q = new_q;
                                }
                            });
                            let eq = &config.get().playback.equalizer;
                            (eq.enabled, eq.gains.clone(), eq.q_values.clone())
                        });
                    if enabled {
                        cx.update_global::<Playback, _>(|playback, _cx| {
                            playback.apply_eq_settings(&gains, &q_values);
                        });
                    }
                }
            })
            .detach();
        }

        for (i, input) in freq_inputs.iter().enumerate() {
            cx.subscribe(input, move |_this, _entity, event, cx| {
                if let InputEvent::Submit(text) = event
                    && let Ok(new_freq) = text.parse::<i32>()
                {
                    let new_freq = new_freq.clamp(20, 20000);
                    cx.update_global::<Config, _>(|config, _cx| {
                        config.set(|s| {
                            if let Some(f) = s.playback.equalizer.frequencies.get_mut(i) {
                                *f = new_freq;
                            }
                        });
                    });
                    cx.update_global::<Playback, _>(|playback, cx| {
                        let config = cx.global::<Config>().clone();
                        playback.apply_config(&config);
                    });
                }
            })
            .detach();
        }

        let lastfm_threshold = cx
            .global::<Config>()
            .get()
            .integrations
            .lastfm
            .scrobble_threshold;
        let lastfm_threshold_input = cx.new(|cx| {
            TextInput::new(cx, "")
                .with_text(format!("{:.0}", lastfm_threshold * 100.0))
                .with_background(element_hover)
                .with_text_color(text_secondary)
                .with_height(px(24.0))
                .centered()
                .with_validator(|s| {
                    if s.is_empty() {
                        return true;
                    }
                    s.parse::<f32>()
                        .map(|v| (5.0..=100.0).contains(&v))
                        .unwrap_or(false)
                })
        });

        cx.subscribe(&lastfm_threshold_input, move |_this, _entity, event, cx| {
            if let InputEvent::Submit(text) = event
                && let Ok(new_pct) = text.parse::<f32>()
            {
                let new_pct = new_pct.clamp(5.0, 100.0);
                cx.update_global::<Config, _>(|config, _cx| {
                    config.set(|s| s.integrations.lastfm.scrobble_threshold = new_pct / 100.0);
                });
            }
        })
        .detach();

        Self {
            tab: SettingsTab::General,
            gain_inputs,
            freq_inputs,
            q_inputs,
            lastfm_threshold_input,
        }
    }
}

impl SettingsView {
    fn general(&self, variables: &Variables, cx: &App) -> Div {
        let general = cx.global::<Config>().get().general.clone();

        page(variables)
            .child(
                group(variables, "System tray")
                    .child(setting(
                        variables,
                        "Show tray icon",
                        "Control playback from your system tray",
                        Switch::new("tray-icon-switch", general.tray_icon).on_change(
                            move |value, _window, cx| {
                                cx.update_global::<Config, _>(|config, _cx| {
                                    config.set(|s| s.general.tray_icon = value);
                                });
                            },
                        ),
                    ))
                    .when(general.tray_icon, |group| {
                        group.child(setting(
                            variables,
                            "Close to tray",
                            "Keep playing in the tray when the window is closed",
                            Switch::new("close-to-tray-switch", general.close_to_tray).on_change(
                                move |value, _window, cx| {
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        config.set(|s| s.general.close_to_tray = value);
                                    });
                                },
                            ),
                        ))
                    }),
            )
            .child(
                group(variables, "Library").child(
                    related(variables)
                        .child(setting(
                            variables,
                            "Add music folder",
                            "Vleer scans these folders for songs",
                            Button::new("add-scan-path")
                                .variant(ButtonVariant::Default)
                                .icon(icons::PLUS)
                                .child("Add folder")
                                .on_click(|_event, window, cx| add_scan_path(window, cx)),
                        ))
                        .child(ScanPathsList),
                ),
            )
    }

    fn appearance(&self, variables: &Variables, cx: &App) -> Div {
        let appearance = cx.global::<Config>().get().appearance.clone();
        let spectrum = appearance.spectrum;

        page(variables)
            .child(group(variables, "Visualizer").child(setting(
                variables,
                "Show playing animation",
                "Animate bars next to the current song",
                Switch::new("visualizer-enabled-switch", appearance.visualizer).on_change(
                    move |value, _window, cx| {
                        cx.update_global::<Config, _>(|config, _cx| {
                            config.set(|s| s.appearance.visualizer = value);
                        });
                        cx.update_global::<Playback, _>(|playback, _cx| {
                            playback.set_visualizer_enabled(value);
                        });
                    },
                ),
            )))
            .child(
                group(variables, "Spectrum")
                    .child(setting(
                        variables,
                        "Show spectrum",
                        "Display live frequency bars in the player",
                        Switch::new("spectrum-enabled-switch", spectrum.enabled).on_change(
                            move |value, _window, cx| {
                                cx.update_global::<Config, _>(|config, _cx| {
                                    config.set(|s| s.appearance.spectrum.enabled = value);
                                });
                                cx.update_global::<Playback, _>(|playback, _cx| {
                                    playback.set_spectrum_enabled(value);
                                });
                            },
                        ),
                    ))
                    .when(spectrum.enabled, |group| {
                        group
                            .child(setting(
                                variables,
                                "Show peak caps",
                                "Keep a marker at the highest point of each bar",
                                Switch::new("spectrum-peak-caps-switch", spectrum.peak_caps)
                                    .on_change(move |value, _window, cx| {
                                        cx.update_global::<Config, _>(|config, _cx| {
                                            config.set(|s| s.appearance.spectrum.peak_caps = value);
                                        });
                                    }),
                            ))
                            .child(setting(
                                variables,
                                "Spectrum detail",
                                "FFT size. Larger shows more bass detail but reacts slower",
                                flex_row().gap(px(variables.padding_8)).children(
                                    FftSize::ALL.into_iter().map(|size| {
                                        Button::new(format!("fft-size-{}", size.samples()))
                                            .variant(if size == spectrum.fft_size {
                                                ButtonVariant::Active
                                            } else {
                                                ButtonVariant::Default
                                            })
                                            .child(size.samples().to_string())
                                            .on_click(move |_event, _window, cx| {
                                                cx.update_global::<Config, _>(|config, _cx| {
                                                    config.set(|s| {
                                                        s.appearance.spectrum.fft_size = size
                                                    });
                                                });
                                            })
                                    }),
                                ),
                            ))
                    }),
            )
    }

    fn playback(&self, variables: &Variables, cx: &App) -> Div {
        let eq_enabled = cx.global::<Config>().get().playback.equalizer.enabled;

        page(variables).child(
            group(variables, "Equalizer").child(
                related(variables)
                    .child(setting(
                        variables,
                        "Enable equalizer",
                        "Shape the sound with 10 adjustable bands",
                        Switch::new("eq-enabled-switch", eq_enabled).on_change(
                            move |value, _window, cx| {
                                let (gains, q_values) =
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        config.set(|s| s.playback.equalizer.enabled = value);
                                        let eq = &config.get().playback.equalizer;
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
                        ),
                    ))
                    .child(EqSection {
                        gain_inputs: self.gain_inputs.clone(),
                        freq_inputs: self.freq_inputs.clone(),
                        q_inputs: self.q_inputs.clone(),
                    }),
            ),
        )
    }

    fn privacy(&self, variables: &Variables, cx: &App) -> Div {
        let telemetry = cx.global::<Config>().get().privacy.telemetry;

        page(variables).child(
            group(variables, "Telemetry")
                .child(setting(
                    variables,
                    "Share usage statistics",
                    "Sends only OS, app version and song count",
                    Switch::new("telemetry-switch", telemetry).on_change(
                        move |value, _window, cx| {
                            cx.update_global::<Config, _>(|config, _cx| {
                                config.set(|s| s.privacy.telemetry = value);
                            });
                        },
                    ),
                ))
                .child(setting(
                    variables,
                    "View public dashboard",
                    "See exactly what is shared",
                    Button::new("telemetry-dashboard-link")
                        .variant(ButtonVariant::Default)
                        .icon(LINK)
                        .child("Open")
                        .on_click(|_event, _window, cx| cx.open_url(TELEMETRY_DASHBOARD)),
                )),
        )
    }

    fn integrations(&self, variables: &Variables, cx: &App) -> Div {
        let integrations = cx.global::<Config>().get().integrations.clone();
        let lastfm = integrations.lastfm;
        let auth_status = cx.global::<LastfmAuthStatus>().clone();
        let connected = lastfm.session_key.is_some();
        let busy = matches!(
            auth_status,
            LastfmAuthStatus::Connecting | LastfmAuthStatus::WaitingForBrowser
        );
        let status_text = match &auth_status {
            LastfmAuthStatus::Connecting => "Requesting authorization…".to_string(),
            LastfmAuthStatus::WaitingForBrowser => {
                "Waiting for authorization in your browser…".to_string()
            }
            LastfmAuthStatus::Error(e) => format!("Error: {e}"),
            LastfmAuthStatus::Idle => match &lastfm.username {
                Some(name) if connected => format!("Connected as {name}"),
                _ => "Not connected".to_string(),
            },
        };
        let threshold_input = self.lastfm_threshold_input.clone();

        let discord = group(variables, "Discord").child(setting(
            variables,
            "Show rich presence",
            "Display what you are playing on Discord",
            Switch::new("discord-rpc-switch", integrations.discord.enabled).on_change(
                move |value, _window, cx| {
                    cx.update_global::<Config, _>(|config, _cx| {
                        config.set(|s| s.integrations.discord.enabled = value);
                    });
                },
            ),
        ));

        let lastfm_group = group(variables, "Last.fm")
            .when(!LastfmClient::is_configured(), |group| {
                group.child(setting(
                    variables,
                    "Last.fm unavailable",
                    "Last.fm is not configured for this build",
                    div(),
                ))
            })
            .when(LastfmClient::is_configured(), |group| {
                group
                    .child(setting(
                        variables,
                        if connected {
                            "Disconnect account"
                        } else {
                            "Connect account"
                        },
                        status_text,
                        Button::new("lastfm-connect-btn")
                            .variant(if connected || busy {
                                ButtonVariant::Default
                            } else {
                                ButtonVariant::Accent
                            })
                            .child(if connected {
                                "Log out"
                            } else if busy {
                                "Connecting…"
                            } else {
                                "Connect"
                            })
                            .on_click(move |_event, _window, cx| {
                                if busy {
                                    return;
                                }
                                if connected {
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        config.set(|s| {
                                            s.integrations.lastfm.session_key = None;
                                            s.integrations.lastfm.username = None;
                                        });
                                    });
                                    return;
                                }
                                start_lastfm_connect(cx);
                            }),
                    ))
                    .child(setting(
                        variables,
                        "Scrobble point",
                        "How much of a song must play before it scrobbles",
                        flex_row()
                            .gap(px(variables.padding_16))
                            .child(
                                slider()
                                    .id("lastfm-scrobble-threshold-slider")
                                    .w(px(160.0))
                                    .h(px(16.0))
                                    .render_full(true)
                                    .value(lastfm.scrobble_threshold)
                                    .on_change({
                                        let threshold_input = threshold_input.clone();
                                        move |val, _win, cx| {
                                            let val = val.clamp(0.05, 1.0);
                                            cx.update_global::<Config, _>(|config, _cx| {
                                                config.set(|s| {
                                                    s.integrations.lastfm.scrobble_threshold = val
                                                });
                                            });
                                            threshold_input.update(cx, |inp, cx| {
                                                inp.set_text(format!("{:.0}", val * 100.0), cx);
                                            });
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .w(px(50.0))
                                    .h(px(24.0))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .child(threshold_input),
                            )
                            .child(div().text_color(variables.text_secondary).child("%")),
                    ))
            });

        page(variables).child(discord).child(lastfm_group)
    }

    fn about(&self, variables: &Variables, cx: &App) -> Div {
        let updates = cx.global::<Config>().get().updates.clone();
        let updater = cx.global::<Updater>().clone();
        let managed_externally = is_managed_externally();
        let status = updater.status();
        let status_text = match &status {
            UpdateStatus::Idle => "Idle".to_string(),
            UpdateStatus::Checking => "Checking…".to_string(),
            UpdateStatus::UpToDate => "Up to date".to_string(),
            UpdateStatus::Available(info) => format!("Update available: {}", info.version),
            UpdateStatus::Downloading => "Downloading…".to_string(),
            UpdateStatus::Installing => "Installing…".to_string(),
            UpdateStatus::Failed(e) => format!("Failed: {e}"),
        };
        let available_info = match status {
            UpdateStatus::Available(info) => Some(info),
            _ => None,
        };

        let header = flex_row()
            .gap(px(variables.padding_24))
            .child(
                img(APP_ICON)
                    .size(px(96.0))
                    .flex_shrink_0()
                    .object_fit(ObjectFit::Contain),
            )
            .child(
                flex_col()
                    .items_start()
                    .gap(px(variables.padding_8))
                    .child(
                        div()
                            .text_color(variables.text)
                            .text_size(px(24.0))
                            .font_weight(FontWeight::BOLD)
                            .child("Vleer"),
                    )
                    .child(
                        flex_col()
                            .items_start()
                            .line_height(px(16.0))
                            .text_color(variables.text_secondary)
                            .child(env!("CARGO_PKG_DESCRIPTION"))
                            .child(env!("CARGO_PKG_LICENSE")),
                    ),
            );

        let link = |id: &'static str, label: &'static str, url: &'static str| {
            Button::new(id)
                .variant(ButtonVariant::Default)
                .icon(LINK)
                .child(label)
                .on_click(move |_event, _window, cx| cx.open_url(url))
        };

        let app = group(variables, "About")
            .child(setting(
                variables,
                "Version",
                format!("Current version: {}", env!("CARGO_PKG_VERSION")),
                div(),
            ))
            .child(setting(
                variables,
                "Source code",
                "Vleer is free and open source on GitHub",
                link("about-github", "GitHub", env!("CARGO_PKG_REPOSITORY")),
            ))
            .child(setting(
                variables,
                "Website",
                "News, downloads and documentation",
                link("about-website", "vleer.app", env!("CARGO_PKG_HOMEPAGE")),
            ));

        let updates_group = group(variables, "Updates")
            .when(managed_externally, |group| {
                group.child(setting(
                    variables,
                    "Updates managed externally",
                    "Use your package manager, or reinstall to update",
                    div(),
                ))
            })
            .when(!managed_externally, |group| {
                group
                    .child(setting(
                        variables,
                        "Check automatically",
                        "Look for new versions when Vleer starts",
                        Switch::new("auto-check-switch", updates.auto_check).on_change(
                            move |value, _window, cx| {
                                cx.update_global::<Config, _>(|config, _cx| {
                                    config.set(|s| s.updates.auto_check = value);
                                });
                            },
                        ),
                    ))
                    .child(setting(
                        variables,
                        "Use nightly builds",
                        "Get early builds that may be unstable",
                        Switch::new("nightly-channel-switch", updates.channel.is_nightly())
                            .on_change({
                                let updater = updater.clone();
                                move |value, _window, cx| {
                                    let channel = if value {
                                        UpdateChannel::Nightly
                                    } else {
                                        UpdateChannel::Stable
                                    };
                                    cx.update_global::<Config, _>(|config, _cx| {
                                        config.set(|s| s.updates.channel = channel);
                                    });
                                    run_check_in_background(
                                        updater.clone(),
                                        channel,
                                        cx.background_executor(),
                                    );
                                }
                            }),
                    ))
                    .child(setting(
                        variables,
                        "Check for updates",
                        status_text,
                        flex_row()
                            .gap(px(variables.padding_8))
                            .child(
                                Button::new("check-updates-btn")
                                    .variant(ButtonVariant::Default)
                                    .child("Check now")
                                    .on_click({
                                        let updater = updater.clone();
                                        move |_event, _window, cx| {
                                            let channel =
                                                cx.global::<Config>().get().updates.channel;
                                            run_check_in_background(
                                                updater.clone(),
                                                channel,
                                                cx.background_executor(),
                                            );
                                        }
                                    }),
                            )
                            .when_some(available_info, |row, info| {
                                let updater = updater.clone();
                                row.child(
                                    Button::new("install-update-btn")
                                        .variant(ButtonVariant::Accent)
                                        .child("Install & restart")
                                        .on_click(move |_event, _window, _cx| {
                                            let info = info.clone();
                                            let updater = updater.clone();
                                            std::thread::spawn(move || {
                                                let path = match updater.download(&info) {
                                                    Ok(p) => p,
                                                    Err(e) => {
                                                        tracing::error!("download failed: {e:#}");
                                                        return;
                                                    }
                                                };
                                                if let Err(e) = updater.install_and_exit(&path) {
                                                    tracing::error!("install failed: {e}");
                                                }
                                            });
                                        }),
                                )
                            }),
                    ))
            });

        page(variables)
            .child(header)
            .child(app)
            .child(updates_group)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let current = self.tab;

        let tabs: Div = flex_row()
            .gap(px(variables.padding_8))
            .items_stretch()
            .children(SettingsTab::ALL.map(|tab| {
                Button::new(format!("settings-tab-{}", tab.id()))
                    .variant(if tab == current {
                        ButtonVariant::Active
                    } else {
                        ButtonVariant::Default
                    })
                    .child(tab.label())
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.tab = tab;
                        cx.notify();
                    }))
            }));

        let content = match current {
            SettingsTab::General => self.general(&variables, cx),
            SettingsTab::Appearance => self.appearance(&variables, cx),
            SettingsTab::Playback => self.playback(&variables, cx),
            SettingsTab::Privacy => self.privacy(&variables, cx),
            SettingsTab::Integrations => self.integrations(&variables, cx),
            SettingsTab::About => self.about(&variables, cx),
        };

        div()
            .flex_1()
            .size_full()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                flex_col()
                    .p(px(variables.padding_24))
                    .gap(px(variables.padding_24))
                    .child(tabs)
                    .child(content),
            )
    }
}
