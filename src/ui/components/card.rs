use gpui::{prelude::FluentBuilder as _, *};
use std::ops::Range;
use std::rc::Rc;

use crate::ui::{
    assets::{ImageRequest, cover_uri},
    components::{
        context_menu::{ContextMenu, album_context_menu_items},
        div::flex_col,
        icons::{self, icon},
        nav::{ArtistLine, open_album},
        song_table::format_artist_line,
    },
    layout::SidePanel,
    variables::Variables,
};
use crate::{data::models::AlbumListItem, media::playback::play_album_now};

pub type PlayHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub const CARD_MIN_IMAGE_SIZE: f32 = 180.0;
pub const CARD_MAX_IMAGE_SIZE: f32 = 400.0;
pub const CARD_GRID_GAP: f32 = 16.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CardImageShape {
    #[default]
    Square,
    Circle,
}

#[derive(Clone, Default)]
pub struct ViewportWidth(Rc<std::cell::Cell<Option<f32>>>);

impl Global for ViewportWidth {}

impl ViewportWidth {
    pub fn get(&self) -> Option<f32> {
        self.0.get()
    }

    pub fn probe(&self) -> impl IntoElement {
        let width = self.0.clone();
        canvas(
            move |bounds, window, _| {
                let measured: f32 = bounds.size.width.into();
                if width
                    .get()
                    .is_none_or(|last| (last - measured).abs() > 0.01)
                {
                    width.set(Some(measured));
                    window.request_animation_frame();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

pub fn content_width(window: &Window, cx: &App) -> f32 {
    let variables = cx.global::<Variables>();
    if let Some(width) = cx.global::<ViewportWidth>().get() {
        return (width - variables.padding_24 * 2.0).max(1.0);
    }
    let mut estimated: f32 = window.bounds().size.width.into();
    estimated -= 300.0 + 98.0;
    if cx.global::<SidePanel>().is_open() {
        estimated -= 316.0;
    }
    estimated.max(1.0)
}

pub const CARD_TEXT_HEIGHT: f32 = 14.0;
const CARD_BODY_GAP: f32 = 8.0;
const CARD_INFO_GAP: f32 = 4.0;

pub fn card_row_height(container_width: f32, columns: usize, subtitle: bool) -> f32 {
    let gaps = columns.saturating_sub(1) as f32 * CARD_GRID_GAP;
    let width = ((container_width - gaps) / columns.max(1) as f32).max(1.0);
    let info = if subtitle {
        CARD_TEXT_HEIGHT * 2.0 + CARD_INFO_GAP
    } else {
        CARD_TEXT_HEIGHT
    };
    width + CARD_BODY_GAP + info + CARD_GRID_GAP
}

pub fn card_spacer() -> Div {
    div()
        .flex_grow(1.0)
        .flex_shrink(1.0)
        .flex_basis(px(0.0))
        .min_w_0()
}

pub fn card_columns(container_width: Option<f32>) -> usize {
    let width = container_width.unwrap_or(1000.0);
    (((width + CARD_GRID_GAP) / (CARD_MIN_IMAGE_SIZE + CARD_GRID_GAP)).floor() as usize).max(1)
}

#[derive(IntoElement)]
pub struct Card {
    id: SharedString,
    base: Stateful<Div>,
    title: SharedString,
    subtitle: Option<SharedString>,
    subtitle_artist_ranges: Option<Vec<Range<usize>>>,
    image_uri: Option<String>,
    image_shape: CardImageShape,
    on_play: Option<PlayHandler>,
}

impl Card {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        let id = id.into();

        Self {
            base: flex_col().id(id.clone()),
            id,
            title: title.into(),
            subtitle: None,
            subtitle_artist_ranges: None,
            image_uri: None,
            image_shape: CardImageShape::Square,
            on_play: None,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn subtitle_artist_ranges(mut self, ranges: Vec<Range<usize>>) -> Self {
        self.subtitle_artist_ranges = Some(ranges);
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
            subtitle_artist_ranges,
            image_uri,
            image_shape,
            on_play,
        } = self;
        let variables = *cx.global::<Variables>();
        let tile_id = id.to_string();
        let image_hover_group: SharedString = format!("{tile_id}-image-hover").into();

        let image = image_uri.map(|uri| {
            let element = img(format!(
                "{}?size={}",
                uri,
                crate::ui::assets::bucket_size(CARD_MAX_IMAGE_SIZE)
            ))
            .id(ElementId::Name(format!("{tile_id}-image").into()))
            .size_full()
            .object_fit(ObjectFit::Cover);
            match image_shape {
                CardImageShape::Square => element.into_any_element(),
                CardImageShape::Circle => element.rounded_full().into_any_element(),
            }
        });

        let mut image_container = div()
            .id(ElementId::Name(format!("{tile_id}-image-container").into()))
            .w_full()
            .aspect_square()
            .relative()
            .bg(variables.border)
            .group(image_hover_group.clone())
            .children(image);

        if matches!(image_shape, CardImageShape::Circle) {
            image_container = image_container.rounded_full();
        }

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
                    .group_hover_style(image_hover_group, |s| s.visible())
                    .child(
                        div()
                            .id(ElementId::Name(format!("{tile_id}-play-button").into()))
                            .size(px(variables.padding_32))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(variables.accent)
                            .hover_style(|s| s.bg(variables.accent_hover))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
                                cx.stop_propagation();
                                on_play(window, cx);
                            })
                            .child(
                                icon(icons::PLAY)
                                    .size(px(variables.padding_16))
                                    .text_color(variables.background),
                            ),
                    ),
            );
        }

        base.id(id)
            .flex_grow(1.0)
            .flex_shrink(1.0)
            .flex_basis(px(0.0))
            .min_w_0()
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
                            .whitespace_nowrap()
                            .font_weight(FontWeight(500.0))
                            .h(px(CARD_TEXT_HEIGHT))
                            .line_height(px(CARD_TEXT_HEIGHT))
                            .w_full()
                            .min_w_0()
                            .child(title),
                    )
                    .when_some(subtitle, |this, subtitle| {
                        if let Some(ranges) = subtitle_artist_ranges {
                            this.child(
                                div()
                                    .id(ElementId::Name(format!("{tile_id}-subtitle").into()))
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .h(px(CARD_TEXT_HEIGHT))
                                    .line_height(px(CARD_TEXT_HEIGHT))
                                    .w_full()
                                    .min_w_0()
                                    .text_color(variables.text_secondary)
                                    .child(ArtistLine::new(
                                        format!("{tile_id}-artist-line"),
                                        subtitle,
                                        ranges,
                                    )),
                            )
                        } else {
                            this.child(
                                div()
                                    .id(ElementId::Name(format!("{tile_id}-subtitle").into()))
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .h(px(CARD_TEXT_HEIGHT))
                                    .line_height(px(CARD_TEXT_HEIGHT))
                                    .w_full()
                                    .min_w_0()
                                    .text_color(variables.text_secondary)
                                    .child(subtitle),
                            )
                        }
                    }),
            )
    }
}

pub fn album_card(id: String, album: &AlbumListItem, context_menu: Entity<ContextMenu>) -> Card {
    let (artist_str, artist_ranges) = format_artist_line(&album.artist_name);
    let (subtitle, subtitle_ranges) = if let Some(year) = &album.year {
        let prefix = format!("{} \u{00B7} ", year);
        let prefix_len = prefix.len();
        let subtitle = format!("{}{}", prefix, artist_str);
        let ranges: Vec<std::ops::Range<usize>> = artist_ranges
            .into_iter()
            .map(|r| (r.start + prefix_len)..(r.end + prefix_len))
            .collect();
        (subtitle, ranges)
    } else {
        (artist_str, artist_ranges)
    };

    let album_id = album.id.clone();
    let play_album_id = album_id.clone();
    let nav_album_id = album_id.clone();

    Card::new(id, album.title.clone())
        .subtitle(subtitle)
        .subtitle_artist_ranges(subtitle_ranges)
        .image_uri(Some(cover_uri(
            album.image_id.as_deref(),
            ImageRequest::Album(album.id.clone()),
        )))
        .on_play(move |_window, cx| {
            play_album_now(play_album_id.clone(), cx);
        })
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            open_album(nav_album_id.clone(), window, cx);
        })
        .on_mouse_down(MouseButton::Right, move |event, _window, cx| {
            let items = album_context_menu_items(album_id.clone(), cx);
            context_menu.update(cx, |menu, cx| {
                menu.show(event.position, items, cx);
            });
        })
}
