use gpui::{Context, IntoElement, Render, *};

use crate::ui::{
    components::{div::flex_col},
    variables::Variables,
};

pub struct SettingsView {}

impl SettingsView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let view = Self {};
        view
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        flex_col()
            .size_full()
            .p(px(variables.padding_24))
    }
}
