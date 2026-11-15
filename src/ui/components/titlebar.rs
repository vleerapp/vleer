use gpui::{prelude::FluentBuilder as _, *};

use crate::ui::{
    app::MainWindow,
    components::{
        div::flex_row,
        icons::{ARROW_LEFT, ARROW_RIGHT, icon},
        window_controls::WindowControls,
    },
    variables::Variables,
    views::NavState,
};

const TITLEBAR_HEIGHT: f32 = 32.0;

fn show_linux_controls(window: &Window) -> bool {
    cfg!(target_os = "linux") && matches!(window.window_decorations(), Decorations::Client { .. })
}

pub fn titlebar_visible(window: &Window) -> bool {
    cfg!(target_os = "windows") || show_linux_controls(window)
}

pub fn history_in_titlebar(window: &Window) -> bool {
    titlebar_visible(window) || (cfg!(target_os = "macos") && !window.is_fullscreen())
}

pub fn content_top_padding(window: &Window, variables: &Variables) -> Pixels {
    if titlebar_visible(window) {
        px(0.0)
    } else if cfg!(target_os = "macos") && !window.is_fullscreen() {
        px(variables.padding_16 * 2.0)
    } else {
        px(variables.padding_16)
    }
}

pub fn history_buttons(cx: &App) -> Div {
    let variables = cx.global::<Variables>();
    let state = *cx.global::<NavState>();
    let button = |id: &'static str, path: &'static str, enabled: bool, forward: bool| {
        icon(path)
            .id(id)
            .text_color(if enabled {
                variables.text_secondary
            } else {
                variables.text_muted
            })
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover_style(|s| s.text_color(variables.text))
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                if !enabled {
                    return;
                }
                if let Some(Some(root)) = window.root::<MainWindow>() {
                    root.update(cx, |main, cx| {
                        if forward {
                            main.go_forward(window, cx)
                        } else {
                            main.go_back(window, cx)
                        }
                    });
                }
            })
    };
    flex_row()
        .items_center()
        .gap(px(variables.padding_8))
        .child(button("history-back", ARROW_LEFT, state.back, false))
        .child(button("history-forward", ARROW_RIGHT, state.forward, true))
}

pub struct Titlebar {
    should_move: bool,
}

impl Titlebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<NavState>(|_, cx| cx.notify()).detach();
        Self { should_move: false }
    }
}

impl Render for Titlebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let height = px(TITLEBAR_HEIGHT);

        let mut titlebar = flex_row()
            .id("window-titlebar")
            .w_full()
            .h(height)
            .justify_between()
            .bg(variables.background)
            .child(history_buttons(cx).pl(px(variables.padding_16)))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(WindowControls::new(height).into_any_element());

        if show_linux_controls(window) {
            titlebar = titlebar
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _ev, _window, _cx| {
                        this.should_move = true;
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _ev, _window, _cx| {
                        this.should_move = false;
                    }),
                )
                .on_mouse_move(cx.listener(|this, _ev, window, _cx| {
                    if this.should_move {
                        this.should_move = false;
                        window.start_window_move();
                    }
                }));
        }

        titlebar
    }
}
