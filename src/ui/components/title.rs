use gpui::{
    InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled, div, px,
};

use crate::ui::variables::Variables;

#[derive(IntoElement)]
pub struct Title {
    label: SharedString,
    hover_group: SharedString,
}

impl Title {
    pub fn new(label: impl Into<SharedString>, hover_group: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            hover_group: hover_group.into(),
        }
    }
}

impl RenderOnce for Title {
    fn render(self, _window: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        div()
            .id(self.hover_group.clone())
            .absolute()
            .top(px(-6.0))
            .left(px(6.0))
            .px(px(2.0))
            .bg(variables.background)
            .text_color(variables.border)
            .group_hover(self.hover_group, |s| s.text_color(variables.accent))
            .child(self.label)
    }
}
