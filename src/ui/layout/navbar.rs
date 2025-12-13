use gpui::*;

use crate::ui::{
    components::{
        div::flex_row,
        icons::icons::{HOME, SETTINGS},
        nav_button::NavButton,
        title::Title,
    },
    variables::Variables,
    views::AppView,
};

pub struct Navbar {
    pub hovered: bool,
}

impl Navbar {
    pub fn new() -> Self {
        Self { hovered: false }
    }
}

impl Render for Navbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let border_color = if self.hovered {
            variables.accent
        } else {
            variables.border
        };

        div()
            .relative()
            .size_full()
            .child(
                flex_row()
                    .border(px(1.0))
                    .border_color(border_color)
                    .h_full()
                    .justify_between()
                    .child(flex_row().p(px(variables.padding_16)).child(NavButton::new(
                        HOME,
                        Some("Home"),
                        AppView::Home,
                    )))
                    .child(div().p(px(variables.padding_16)).child(NavButton::new(
                        SETTINGS,
                        None,
                        AppView::Settings,
                    ))),
            )
            .child(Title::new("NavBar", self.hovered))
    }
}
