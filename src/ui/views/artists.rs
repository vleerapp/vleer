use gpui::{Context, IntoElement, Render, *};

use crate::ui::{components::div::flex_col, variables::Variables};

pub struct ArtistsView {}

impl ArtistsView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let view = Self {};
        view
    }
}

impl Render for ArtistsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        flex_col()
            .border(px(1.0))
            .border_color(variables.border)
            .group_hover("artists-view", |s| s.border_color(variables.accent))
            .size_full()
            .p(px(variables.padding_24))
    }
}
