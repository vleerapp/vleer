use gpui::*;

use crate::ui::{
    components::{
        div::flex_row,
        icons::icons::{HOME, SETTINGS},
        nav_button::NavButton,
    },
    variables::Variables,
    views::AppView,
};

pub struct Navbar {}

impl Navbar {
    pub fn new() -> Self {
        Self {}
    }
}

impl Render for Navbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();

        flex_row()
            .h_full()
            .justify_between()
            .child(flex_row().p(px(variables.padding_16)).child(NavButton::new(
                HOME,
                Some("Home"),
                None,
                AppView::Home,
            )))
            .child(div().p(px(variables.padding_16)).child(NavButton::new(
                SETTINGS,
                None,
                None,
                AppView::Settings,
            )))
    }
}
