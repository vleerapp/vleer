use crate::{
    data::state::State,
    ui::{
        components::{div::flex_row, icons::icon::icon},
        variables::Variables,
        views::AppView,
    },
};
use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(IntoElement)]
pub struct NavButton {
    icon: SharedString,
    label: Option<SharedString>,
    target_view: AppView,
}

impl NavButton {
    pub fn new(icon: impl Into<SharedString>, label: Option<&str>, target_view: AppView) -> Self {
        Self {
            icon: icon.into(),
            label: label.map(|s| SharedString::from(s.to_string())),
            target_view,
        }
    }
}

impl RenderOnce for NavButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let state = cx.global::<State>();
        let current_view = state.get_current_view_sync();
        let is_active = current_view == self.target_view;
        let target_view = self.target_view;
        let icon_path = self.icon;
        let label = self.label;

        let text_color = if is_active {
            variables.text
        } else {
            variables.text_secondary
        };

        flex_row()
            .items_center()
            .gap(px(variables.padding_8))
            .text_color(text_color)
            .cursor_pointer()
            .child(icon(icon_path).text_color(text_color))
            .when_some(label, |this, label_text| {
                this.child(div().child(label_text).hover(|s| s.underline()))
            })
            .hover(|s| s.text_color(variables.text))
            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                let state = cx.global::<State>().clone();
                state.set_current_view_sync(target_view);
                window.refresh();
            })
    }
}
