use gpui::{Context, IntoElement, Render, *};

use crate::ui::{
    components::{context_menu::ContextMenu, div::flex_col},
    variables::Variables,
};

pub struct PlaylistsView {
    context_menu: Entity<ContextMenu>,
}

impl PlaylistsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            context_menu: cx.new(|_| ContextMenu::new()),
        }
    }
}

impl Render for PlaylistsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        flex_col()
            .size_full()
            .p(px(variables.padding_24))
            .child(self.context_menu.clone())
    }
}
