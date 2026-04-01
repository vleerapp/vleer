use gpui::{prelude::FluentBuilder as _, *};
use std::rc::Rc;

use crate::ui::{
    components::{
        div::flex_col,
        icons::{icon::icon, icons::PLAY},
    },
    variables::Variables,
};

pub const CARD_MIN_IMAGE_SIZE: f32 = 180.0;
pub const CARD_MAX_IMAGE_SIZE: f32 = 400.0;
pub const CARD_GRID_GAP: f32 = 16.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardImageShape {
    #[default]
    Square,
    Circle,
}

pub fn calculate_card_layout(container_width: Option<f32>) -> (f32, usize) {
    let width = container_width.unwrap_or(1000.0);

    let item_count =
        ((width + CARD_GRID_GAP) / (CARD_MIN_IMAGE_SIZE + CARD_GRID_GAP)).floor() as usize;
    let item_count = item_count.max(1);

    let image_size = ((width - (item_count - 1) as f32 * CARD_GRID_GAP) / item_count as f32)
        .clamp(CARD_MIN_IMAGE_SIZE, CARD_MAX_IMAGE_SIZE);

    (image_size, item_count)
}

#[derive(IntoElement)]
pub struct Card {
    id: SharedString,
    base: Stateful<Div>,
    title: SharedString,
    subtitle: Option<SharedString>,
    image_uri: Option<String>,
    image_size: f32,
    image_shape: CardImageShape,
    on_play: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl Card {
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        image_size: f32,
    ) -> Self {
        let id = id.into();

        Self {
            base: flex_col().id(id.clone()),
            id,
            title: title.into(),
            subtitle: None,
            image_uri: None,
            image_size,
            image_shape: CardImageShape::Square,
            on_play: None,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn image_uri(mut self, image_uri: Option<String>) -> Self {
        self.image_uri = image_uri;
        self
    }

    pub fn image_shape(mut self, image_shape: CardImageShape) -> Self {
        self.image_shape = image_shape;
        self
    }

    pub fn on_play(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(Rc::new(handler));
        self
    }
}

impl Styled for Card {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Card {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Card {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Card {
            id,
            base,
            title,
            subtitle,
            image_uri,
            image_size,
            image_shape,
            on_play,
        } = self;
        let variables = cx.global::<Variables>();
        let tile_id = id.to_string();
        let image_hover_group: SharedString = format!("{tile_id}-image-hover").into();

        let image = match image_uri {
            Some(uri) => match image_shape {
                CardImageShape::Square => img(format!("!image://{}", uri))
                    .id(ElementId::Name(format!("{tile_id}-image").into()))
                    .size(px(image_size))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                CardImageShape::Circle => img(format!("!image://{}", uri))
                    .id(ElementId::Name(format!("{tile_id}-image").into()))
                    .size(px(image_size))
                    .object_fit(ObjectFit::Cover)
                    .rounded_full()
                    .into_any_element(),
            },
            None => match image_shape {
                CardImageShape::Square => div()
                    .id(ElementId::Name(
                        format!("{tile_id}-image-placeholder").into(),
                    ))
                    .size(px(image_size))
                    .bg(variables.border)
                    .into_any_element(),
                CardImageShape::Circle => div()
                    .id(ElementId::Name(
                        format!("{tile_id}-image-placeholder").into(),
                    ))
                    .size(px(image_size))
                    .bg(variables.border)
                    .rounded_full()
                    .into_any_element(),
            },
        };

        let mut image_container = div()
            .id(ElementId::Name(format!("{tile_id}-image-container").into()))
            .size(px(image_size))
            .relative()
            .group(image_hover_group.clone())
            .child(image);

        if let Some(on_play) = on_play {
            image_container = image_container.child(
                div()
                    .id(ElementId::Name(
                        format!("{tile_id}-play-button-container").into(),
                    ))
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_end()
                    .justify_end()
                    .p(px(variables.padding_16))
                    .invisible()
                    .group_hover(image_hover_group, |s| s.visible())
                    .child(
                        div()
                            .id(ElementId::Name(format!("{tile_id}-play-button").into()))
                            .size(px(variables.padding_32))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(variables.accent)
                            .hover(|s| s.bg(variables.accent_background))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                (on_play)(window, cx);
                            })
                            .child(
                                icon(PLAY)
                                    .size(px(variables.padding_16))
                                    .text_color(variables.background),
                            ),
                    ),
            );
        }

        base.id(id)
            .w(px(image_size))
            .gap(px(8.0))
            .child(image_container)
            .child(
                flex_col()
                    .id(ElementId::Name(format!("{tile_id}-info").into()))
                    .gap(px(4.0))
                    .child(
                        div()
                            .id(ElementId::Name(format!("{tile_id}-title").into()))
                            .text_ellipsis()
                            .font_weight(FontWeight(500.0))
                            .overflow_x_hidden()
                            .max_w(px(image_size))
                            .child(title),
                    )
                    .when_some(subtitle, |this, subtitle| {
                        this.child(
                            div()
                                .id(ElementId::Name(format!("{tile_id}-subtitle").into()))
                                .text_ellipsis()
                                .overflow_x_hidden()
                                .max_w(px(image_size))
                                .text_color(variables.text_secondary)
                                .child(subtitle),
                        )
                    }),
            )
    }
}
