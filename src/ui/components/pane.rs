use crate::ui::components::title::Title;
use crate::ui::variables::Variables;
use gpui::prelude::FluentBuilder;
use gpui::*;

#[derive(IntoElement)]
pub struct Pane {
    id: SharedString,
    title: Option<SharedString>,
    content: AnyElement,
}

impl Pane {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: None,
            content: div().into_any_element(),
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn child(mut self, content: impl IntoElement) -> Self {
        self.content = content.into_any_element();
        self
    }
}

impl RenderOnce for Pane {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let title_id = self.id.clone();
        let variables = cx.global::<Variables>();

        div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .group(self.id.clone())
            .child(
                div()
                    .id(self.id.clone())
                    .size_full()
                    .min_h_0()
                    .border(px(1.0))
                    .border_color(Hsla::from(variables.border))
                    .group_hover(title_id.clone(), |s| s.border_color(variables.accent))
                    .child(self.content),
            )
            .when_some(self.title, |this, title| {
                this.child(Title::new(title, title_id.clone()))
            })
    }
}

pub fn pane(id: impl Into<SharedString>) -> Pane {
    Pane::new(id)
}
