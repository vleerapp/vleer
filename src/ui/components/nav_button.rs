use gpui::*;

use crate::ui::{
    app::ViewState, components::icons::icon::icon, variables::Variables, views::AppView,
};

#[derive(IntoElement)]
pub struct NavButton {
    icon: SharedString,
    label: SharedString,
    target_view: AppView,
}

impl NavButton {
    pub fn new(
        icon: impl Into<SharedString>,
        label: impl Into<SharedString>,
        target_view: AppView,
    ) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
            target_view,
        }
    }
}

impl RenderOnce for NavButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let current_view = cx.global::<ViewState>().current();
        let is_active = current_view == self.target_view;
        let target_view = self.target_view;
        let icon_path = self.icon;
        let label = self.label;

        let text_color = if is_active {
            variables.text
        } else {
            variables.text_secondary
        };

        div()
            .flex()
            .items_center()
            .gap(px(variables.padding_8))
            .text_color(text_color)
            .child(icon(icon_path).text_color(text_color))
            .child(label)
            .hover(|s| s.text_color(variables.text).cursor_pointer())
            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                cx.update_global::<ViewState, _>(|state, _cx| {
                    state.set(target_view);
                });
                window.refresh();
            })
    }
}
