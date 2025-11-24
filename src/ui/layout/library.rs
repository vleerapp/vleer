use gpui::*;

use crate::ui::{
    components::{
        div::{flex_col, flex_row},
        icons::icons,
        navbutton::NavButton,
        title::Title,
    },
    variables::Variables,
    views::AppView,
};

pub struct Library {
    pub hovered: bool,
}

impl Library {
    pub fn new() -> Self {
        Self { hovered: false }
    }
}

impl Render for Library {
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
                flex_col()
                    .id("library")
                    .border(px(1.0))
                    .border_color(border_color)
                    .h_full()
                    .overflow_y_scroll()
                    .p(px(variables.padding_16))
                    .gap(px(8.0))
                    .child(NavButton::new(icons::SONGS, "Songs", AppView::Songs)),
            )
            .child(Title::new("Library", self.hovered))
    }
}
