use gpui::{prelude::FluentBuilder as _, *};
use std::rc::Rc;

use crate::ui::variables::Variables;

const TRACK_WIDTH: f32 = 50.0;
const TRACK_HEIGHT: f32 = 18.0;
const THUMB_WIDTH: f32 = 22.0;
const THUMB_HEIGHT: f32 = 14.0;
const THUMB_PADDING: f32 = 2.0;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    on_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            on_change: None,
        }
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let thumb_color: Hsla = if self.checked {
            variables.accent.into()
        } else {
            variables.element_hover.into()
        };
        let thumb_x = if self.checked {
            TRACK_WIDTH - THUMB_WIDTH - THUMB_PADDING
        } else {
            THUMB_PADDING
        };

        let thumb = div()
            .absolute()
            .left(px(thumb_x))
            .top(px(THUMB_PADDING))
            .w(px(THUMB_WIDTH))
            .h(px(THUMB_HEIGHT))
            .bg(thumb_color);

        let checked = self.checked;
        div()
            .id(self.id)
            .cursor_pointer()
            .w(px(TRACK_WIDTH))
            .h(px(TRACK_HEIGHT))
            .bg(variables.element)
            .relative()
            .when_some(self.on_change, |this, on_change| {
                this.on_click(move |_event, window, cx| {
                    (on_change)(!checked, window, cx);
                })
            })
            .child(thumb)
    }
}
