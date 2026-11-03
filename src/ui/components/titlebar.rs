use gpui::*;

use crate::ui::{
    components::{div::flex_row, window_controls::WindowControls},
    variables::Variables,
};

const TITLEBAR_HEIGHT: f32 = 32.0;

fn show_linux_controls(window: &Window) -> bool {
    cfg!(target_os = "linux") && matches!(window.window_decorations(), Decorations::Client { .. })
}

pub fn titlebar_visible(window: &Window) -> bool {
    cfg!(target_os = "windows") || show_linux_controls(window)
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

pub struct Titlebar {
    should_move: bool,
}

impl Titlebar {
    pub fn new() -> Self {
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
            .window_control_area(WindowControlArea::Drag)
            .child(div().flex_1().pl(px(variables.padding_16)))
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
