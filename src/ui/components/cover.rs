use gpui::{prelude::FluentBuilder, *};

use crate::ui::{
    components::{
        div::{flex_col, flex_row},
        icons::{self, icon},
    },
    variables::Variables,
};

const COVER_HOVER_GROUP: &str = "cover-hover";

pub fn cover(
    size: f32,
    uri: Option<String>,
    round: bool,
    variables: &Variables,
    on_play: Option<impl Fn(&mut Window, &mut App) + 'static>,
) -> Div {
    let image = uri.map(|uri| {
        let element = img(format!("{uri}?size={}", size as u32))
            .size_full()
            .object_fit(ObjectFit::Cover);
        if round {
            element.rounded_full().into_any_element()
        } else {
            element.into_any_element()
        }
    });
    let container = div().size_full().bg(variables.border).children(image);

    div()
        .group(COVER_HOVER_GROUP)
        .size(px(size))
        .flex_shrink_0()
        .relative()
        .child(if round {
            container.rounded_full().into_any_element()
        } else {
            container.into_any_element()
        })
        .when_some(on_play, |this, on_play| {
            this.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().opacity(0.5))
                    .invisible()
                    .group_hover_style(COVER_HOVER_GROUP, |s| s.visible())
                    .child(icon(icons::PLAY).size(px(16.0)).text_color(variables.text))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                        cx.stop_propagation();
                        on_play(window, cx);
                    }),
            )
        })
}

pub fn avatar(image_id: Option<&str>, size: f32, variables: &Variables) -> Div {
    cover(
        size,
        image_id.map(|id| format!("!image://{id}")),
        true,
        variables,
        None::<fn(&mut Window, &mut App)>,
    )
}

const COVER_SIZE: f32 = 64.0;

#[allow(clippy::too_many_arguments)]
pub fn release_row(
    id: impl Into<SharedString>,
    cover_uri: Option<String>,
    title: String,
    subtitle: String,
    expanded: bool,
    variables: &Variables,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_play: impl Fn(&mut Window, &mut App) + 'static,
    on_menu: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_open: Option<impl Fn(&mut Window, &mut App) + 'static>,
    tracks: Vec<AnyElement>,
) -> Stateful<Div> {
    let id: SharedString = id.into();

    flex_col()
        .id(ElementId::Name(id.clone()))
        .w_full()
        .child(
            flex_row()
                .id(ElementId::Name(format!("{id}-header").into()))
                .w_full()
                .gap(px(variables.padding_8))
                .pr(px(variables.padding_8))
                .bg(variables.element)
                .cursor_pointer()
                .hover_style(|s| s.bg(variables.element_hover))
                .on_click(on_toggle)
                .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    on_menu(event, window, cx)
                })
                .child(cover(
                    COVER_SIZE,
                    cover_uri,
                    false,
                    variables,
                    Some(on_play),
                ))
                .child(
                    flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .id(ElementId::Name(format!("{id}-title").into()))
                                .self_start()
                                .max_w_full()
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .font_weight(FontWeight(500.0))
                                .when_some(on_open, |this, on_open| {
                                    this.cursor_pointer()
                                        .hover_style(|s| s.underline())
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            move |_event, window, cx| {
                                                cx.stop_propagation();
                                                on_open(window, cx);
                                            },
                                        )
                                })
                                .child(title),
                        )
                        .child(
                            div()
                                .overflow_x_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_color(variables.text_secondary)
                                .child(subtitle),
                        ),
                )
                .child(icon(if expanded {
                    icons::ARROW_DOWN
                } else {
                    icons::ARROW_RIGHT
                })),
        )
        .when(expanded, |this| {
            this.child(
                flex_col()
                    .w_full()
                    .px(px(variables.padding_8))
                    .pt(px(variables.padding_16))
                    .children(tracks),
            )
        })
}
