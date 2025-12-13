use gpui::{Context, IntoElement, Render, *};

use crate::ui::{
    components::{div::flex_col, title::Title},
    variables::Variables,
    views::HoverableView,
};

pub struct SettingsView {
    pub hovered: bool,
}

impl SettingsView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let view = Self { hovered: false };
        view
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        div()
            .id("settings-view")
            .relative()
            .size_full()
            .child(
                flex_col()
                    .border(px(1.0))
                    .border_color(border_color)
                    .size_full()
                    .p(px(variables.padding_24)),
            )
            .child(Title::new("Settings", self.hovered))
    }
}

impl HoverableView for SettingsView {
    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.hovered = hovered;
        cx.notify();
    }
}
