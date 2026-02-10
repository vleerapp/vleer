use gpui::*;

use crate::ui::{
    components::{
        div::flex_row,
        icons::{
            icon::icon,
            icons::{MAXIMIZE, MINIMIZE, UNMAXIMIZE, X},
        },
    },
    variables::Variables,
};

#[derive(IntoElement)]
pub struct WindowControls {
    titlebar_height: Pixels,
}

impl WindowControls {
    pub fn new(titlebar_height: Pixels) -> Self {
        Self { titlebar_height }
    }
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let supported = window.window_controls();
        let close_hover: Hsla = Rgba {
            r: 232.0 / 255.0,
            g: 17.0 / 255.0,
            b: 32.0 / 255.0,
            a: 1.0,
        }
        .into();
        let close_active = close_hover.opacity(0.85);
        let hover_bg = Hsla::from(variables.element_hover);
        let use_window_control_area = cfg!(target_os = "windows");

        let mut controls = flex_row().id("window-controls").h(self.titlebar_height);

        if !use_window_control_area {
            controls = controls.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        }

        if supported.minimize {
            controls = controls.child(window_control_button(
                "window-minimize",
                MINIMIZE,
                variables,
                hover_bg,
                hover_bg,
                WindowControlArea::Min,
                use_window_control_area,
                |window, _| window.minimize_window(),
            ));
        }

        if supported.maximize {
            let icon_path = if window.is_maximized() {
                UNMAXIMIZE
            } else {
                MAXIMIZE
            };
            controls = controls.child(window_control_button(
                "window-maximize",
                icon_path,
                variables,
                hover_bg,
                hover_bg,
                WindowControlArea::Max,
                use_window_control_area,
                |window, _| window.zoom_window(),
            ));
        }

        controls.child(window_control_button(
            "window-close",
            X,
            variables,
            close_hover,
            close_active,
            WindowControlArea::Close,
            use_window_control_area,
            |window, _| window.remove_window(),
        ))
    }
}

fn window_control_button(
    id: &'static str,
    icon_path: &'static str,
    variables: &Variables,
    hover_bg: Hsla,
    active_bg: Hsla,
    area: WindowControlArea,
    use_window_control_area: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let button = flex_row()
        .id(id)
        .w(px(48.0))
        .h_full()
        .cursor_pointer()
        .justify_center()
        .content_center()
        .text_color(variables.text_secondary)
        .hover(|s| s.bg(hover_bg))
        .active(|s| s.bg(active_bg))
        .child(icon(icon_path));

    if use_window_control_area {
        button.window_control_area(area)
    } else {
        button
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_click(window, cx);
            })
    }
}
