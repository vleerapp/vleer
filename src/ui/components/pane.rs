use crate::ui::variables::Variables;
use gpui::prelude::FluentBuilder;
use gpui::*;

const TITLE_BOX_HEIGHT: f32 = 13.0;
const TITLE_INSET_X: f32 = 1.0;
const TITLE_INSET_Y: f32 = 2.0;

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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let title_id = self.id.clone();
        let hover = window.use_keyed_state(title_id.clone(), cx, |_, _| false);
        let hovered = *hover.read(cx);
        let tracker = hover.clone();
        let variables = cx.global::<Variables>();
        let accent_or = |normal: Hsla| {
            if hovered {
                Hsla::from(variables.accent)
            } else {
                normal
            }
        };
        let scale = window.scale_factor();
        let snap = |logical: f32| px((logical * scale).ceil() / scale);

        div()
            .id(self.id.clone())
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        let tracker = tracker.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            let inside = bounds.contains(&event.position);
                            tracker.update(cx, |state, cx| {
                                if *state != inside {
                                    *state = inside;
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .size_full()
                    .min_h_0()
                    .border(px(1.0))
                    .border_color(accent_or(Hsla::from(variables.border)))
                    .child(self.content),
            )
            .when_some(self.title, |this, title| {
                this.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(6.0))
                        .h(px(1.0))
                        .px(px(3.0))
                        .flex()
                        .items_center()
                        .bg(variables.background)
                        .child(
                            div()
                                .relative()
                                .flex_shrink_0()
                                .line_height(px(TITLE_BOX_HEIGHT))
                                .text_color(accent_or(Hsla::from(variables.border)))
                                .child(
                                    div()
                                        .absolute()
                                        .top(snap(TITLE_INSET_Y))
                                        .bottom(snap(TITLE_INSET_Y))
                                        .left(snap(TITLE_INSET_X))
                                        .right(snap(TITLE_INSET_X))
                                        .bg(variables.background),
                                )
                                .child(title),
                        ),
                )
            })
    }
}

pub fn pane(id: impl Into<SharedString>) -> Pane {
    Pane::new(id)
}
