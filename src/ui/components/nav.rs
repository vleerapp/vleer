use gpui::{prelude::FluentBuilder, *};
use std::ops::Range;

use crate::{
    data::{db::Database, models::Cuid},
    ui::{
        app::MainWindow,
        views::{AppView, SelectedAlbum, SelectedArtist},
    },
};

fn navigate(view: AppView, window: &mut Window, cx: &mut App) {
    if let Some(Some(root)) = window.root::<MainWindow>() {
        root.update(cx, |main, cx| main.set_current_view(view, window, cx));
    }
}

pub fn open_album(id: Cuid, window: &mut Window, cx: &mut App) {
    cx.set_global(SelectedAlbum(Some(id)));
    navigate(AppView::Album, window, cx);
}

pub fn open_artist(id: Cuid, window: &mut Window, cx: &mut App) {
    cx.set_global(SelectedArtist(Some(id)));
    navigate(AppView::Artist, window, cx);
}

pub fn open_artist_named(name: &str, window: &mut Window, cx: &mut App) {
    let artist = cx
        .global::<Database>()
        .get_artist_by_name(name)
        .ok()
        .flatten();
    if let Some(artist) = artist {
        open_artist(artist.id, window, cx);
    }
}

fn artist_click_handler(
    text: &str,
    ranges: &[Range<usize>],
) -> impl Fn(usize, &mut Window, &mut App) + 'static {
    let names: Vec<String> = ranges.iter().map(|r| text[r.clone()].to_string()).collect();
    move |ix, window, cx| {
        if let Some(name) = names.get(ix) {
            open_artist_named(name, window, cx);
        }
    }
}

#[derive(IntoElement)]
pub struct ArtistLine {
    id: SharedString,
    text: SharedString,
    ranges: Vec<Range<usize>>,
}

impl ArtistLine {
    pub fn new(
        id: impl Into<SharedString>,
        text: impl Into<SharedString>,
        ranges: Vec<Range<usize>>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            ranges,
        }
    }
}

impl RenderOnce for ArtistLine {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ArtistLine { id, text, ranges } = self;

        let hovered =
            window.use_keyed_state(ElementId::Name(format!("{id}-state").into()), cx, |_, _| {
                None::<usize>
            });
        let current = *hovered.read(cx);

        let mut styled = StyledText::new(text.clone());
        if let Some(range) = current.and_then(|ix| ranges.get(ix)) {
            styled = styled.with_highlights([(
                range.clone(),
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.0),
                        color: None,
                        wavy: false,
                    }),
                    ..Default::default()
                },
            )]);
        }

        let on_click = artist_click_handler(&text, &ranges);
        let click_ranges = ranges.clone();
        let hovered_for_text = hovered.clone();
        let hovered_for_leave = hovered.clone();
        let hovered_for_down = hovered;

        div()
            .id(ElementId::Name(id.clone()))
            .min_w_0()
            .when_some(current, |this, _| this.cursor_pointer())
            .on_hover(move |is_hovered, _window, cx| {
                if !is_hovered {
                    hovered_for_leave.update(cx, |state, cx| {
                        if state.take().is_some() {
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                if hovered_for_down.read(cx).is_some() {
                    cx.stop_propagation();
                }
            })
            .child(
                InteractiveText::new(ElementId::Name(format!("{id}-text").into()), styled)
                    .on_click(click_ranges, on_click)
                    .on_hover(move |ix, _event, _window, cx| {
                        let next = ix.and_then(|ix| ranges.iter().position(|r| r.contains(&ix)));
                        hovered_for_text.update(cx, |state, cx| {
                            if *state != next {
                                *state = next;
                                cx.notify();
                            }
                        });
                    }),
            )
    }
}
