use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ContentMask, Context, Corner, Corners, CursorStyle, DispatchPhase, Div, Edges,
    Element, ElementId, Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Position,
    Render, ScrollHandle, ScrollWheelEvent, Stateful, StatefulInteractiveElement, Style, Styled,
    Task, Window, ease_in_out, prelude::*, px, quad, relative, size,
};

const SCROLLBAR_HIDE_DELAY: Duration = Duration::from_secs(1);
const SCROLLBAR_HIDE_DURATION: Duration = Duration::from_millis(400);
const SCROLLBAR_SHOW_DURATION: Duration = Duration::from_millis(50);
const SCROLLBAR_PADDING: Pixels = px(4.);
const MINIMUM_THUMB_SIZE: Pixels = px(25.);

pub trait WithScrollbar: Sized {
    type Output;

    fn vertical_scroll_with_custom_bar(
        self,
        width: Pixels,
        thumb_color: Hsla,
        track_color: Option<Hsla>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Output;
}

impl WithScrollbar for Stateful<Div> {
    type Output = Self;

    #[track_caller]
    fn vertical_scroll_with_custom_bar(
        self,
        width: Pixels,
        thumb_color: Hsla,
        track_color: Option<Hsla>,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::Output {
        let element_id = std::panic::Location::caller().into();
        let scrollbar =
            get_scrollbar_state(element_id, width, thumb_color, track_color, window, cx);

        render_scrollbar(scrollbar, self, cx)
    }
}

fn get_scrollbar_state(
    element_id: ElementId,
    width: Pixels,
    thumb_color: Hsla,
    track_color: Option<Hsla>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ScrollbarState> {
    window.use_keyed_state(element_id, cx, |_window, _cx| {
        ScrollbarState::new(width, thumb_color, track_color)
    })
}

fn render_scrollbar(
    scrollbar: Entity<ScrollbarState>,
    div: Stateful<Div>,
    cx: &App,
) -> Stateful<Div> {
    let state = scrollbar.read(cx);
    let space = state.width + 2. * SCROLLBAR_PADDING;

    div.track_scroll(&state.scroll_handle)
        .overflow_y_scroll()
        .pr(space)
        .child(scrollbar)
}

struct ScrollbarState {
    scroll_handle: ScrollHandle,
    width: Pixels,
    thumb_color: Hsla,
    track_color: Option<Hsla>,
    thumb_state: ThumbState,
    mouse_in_parent: bool,
    show_state: VisibilityState,
    last_layout: Option<ScrollbarLayout>,
    _hide_task: Option<Task<()>>,
}

impl ScrollbarState {
    fn new(width: Pixels, thumb_color: Hsla, track_color: Option<Hsla>) -> Self {
        Self {
            scroll_handle: ScrollHandle::new(),
            width,
            thumb_color,
            track_color,
            thumb_state: ThumbState::Inactive,
            mouse_in_parent: false,
            show_state: VisibilityState::Visible,
            last_layout: None,
            _hide_task: None,
        }
    }

    fn show(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.show_state != VisibilityState::Visible {
            self.show_state = VisibilityState::Visible;
            cx.notify();
        }
    }

    fn compute_thumb_bounds(&self, viewport_bounds: Bounds<Pixels>) -> Option<ScrollbarLayout> {
        let max_offset = self.scroll_handle.max_offset();
        let viewport_size = self.scroll_handle.bounds().size;
        let current_offset = self.scroll_handle.offset();

        let max_offset_y = max_offset.height;
        let viewport_height = viewport_size.height;

        if max_offset_y == px(0.) || viewport_height == px(0.) {
            return None;
        }

        let content_height = viewport_height + max_offset_y;
        let visible_percentage = viewport_height / content_height;
        let thumb_height = MINIMUM_THUMB_SIZE.max(viewport_height * visible_percentage);

        if thumb_height > viewport_height {
            return None;
        }

        let current_offset_y = current_offset.y.clamp(-max_offset_y, Pixels::ZERO).abs();
        let start_offset = (current_offset_y / max_offset_y) * (viewport_height - thumb_height);

        let track_bounds = Bounds::from_corner_and_size(
            Corner::TopRight,
            viewport_bounds.corner(Corner::TopRight) - Point::new(SCROLLBAR_PADDING, px(0.)),
            size(self.width, viewport_bounds.size.height),
        );

        let padded_track = Bounds::new(
            track_bounds.origin - Point::new(px(0.), SCROLLBAR_PADDING),
            track_bounds.size + size(px(0.), 2. * SCROLLBAR_PADDING),
        );

        let thumb_bounds = Bounds::new(
            padded_track.origin + Point::new(px(0.), start_offset),
            size(self.width, thumb_height),
        );

        Some(ScrollbarLayout {
            thumb_bounds,
            track_bounds: padded_track,
        })
    }

    fn set_offset(&mut self, offset: Pixels, cx: &mut Context<Self>) {
        self.scroll_handle.set_offset(Point::new(px(0.), offset));
        cx.notify();
    }
}

impl Render for ScrollbarState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ScrollbarElement {
            state: cx.entity(),
            origin: Point::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum ThumbState {
    #[default]
    Inactive,
    Hover,
    Dragging(Pixels),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisibilityState {
    Visible,
    Animating { showing: bool },
    Hidden,
}

impl VisibilityState {
    fn for_show() -> Self {
        Self::Animating { showing: true }
    }

    fn for_hide() -> Self {
        Self::Animating { showing: false }
    }

    fn is_visible(&self) -> bool {
        !matches!(self, Self::Hidden)
    }

    fn opacity(&self, progress: f32) -> f32 {
        match self {
            Self::Visible => 1.0,
            Self::Hidden => 0.0,
            Self::Animating { showing: true } => ease_in_out(progress),
            Self::Animating { showing: false } => 1.0 - ease_in_out(progress),
        }
    }
}

#[derive(Clone)]
struct ScrollbarLayout {
    thumb_bounds: Bounds<Pixels>,
    track_bounds: Bounds<Pixels>,
}

impl ScrollbarLayout {
    fn compute_offset_for_position(&self, position: Point<Pixels>, max_offset: Pixels) -> Pixels {
        let viewport_height = self.track_bounds.size.height - 2. * SCROLLBAR_PADDING;
        let thumb_height = self.thumb_bounds.size.height;
        let click_y = position.y - self.track_bounds.origin.y - SCROLLBAR_PADDING;

        let thumb_start = click_y.clamp(px(0.), viewport_height - thumb_height);
        let percentage = if viewport_height > thumb_height {
            thumb_start / (viewport_height - thumb_height)
        } else {
            0.
        };

        max_offset * -percentage
    }

    fn compute_drag_offset(
        &self,
        position: Point<Pixels>,
        drag_start_offset: Pixels,
        max_offset: Pixels,
    ) -> Pixels {
        let click_y = position.y - drag_start_offset;
        let viewport_height = self.track_bounds.size.height - 2. * SCROLLBAR_PADDING;
        let thumb_height = self.thumb_bounds.size.height;

        let thumb_start = (click_y - self.track_bounds.origin.y - SCROLLBAR_PADDING)
            .clamp(px(0.), viewport_height - thumb_height);

        let percentage = if viewport_height > thumb_height {
            thumb_start / (viewport_height - thumb_height)
        } else {
            0.
        };

        max_offset * -percentage
    }
}

struct ScrollbarElement {
    state: Entity<ScrollbarState>,
    origin: Point<Pixels>,
}

struct PrepaintState {
    layout: Option<ScrollbarLayout>,
    parent_hitbox: Hitbox,
    thumb_hitbox: Option<Hitbox>,
    animation_start: Option<Instant>,
}

impl Element for ScrollbarElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<PrepaintState>;

    fn id(&self) -> Option<ElementId> {
        Some(("scrollbar", self.state.entity_id()).into())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let style = Style {
            position: Position::Absolute,
            inset: Edges::default(),
            size: size(relative(1.), relative(1.)).map(Into::into),
            ..Default::default()
        };

        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let layout = self.state.read(cx).compute_thumb_bounds(bounds);

        let thumb_hitbox = layout
            .as_ref()
            .map(|l| window.insert_hitbox(l.thumb_bounds, HitboxBehavior::BlockMouseExceptScroll));

        let animation_start = matches!(
            self.state.read(cx).show_state,
            VisibilityState::Animating { .. }
        )
        .then(|| {
            window.request_animation_frame();
            Instant::now()
        });

        Some(PrepaintState {
            layout,
            parent_hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            thumb_hitbox,
            animation_start,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepaint) = prepaint.take() else {
            return;
        };

        let (opacity, thumb_color, track_color, width, thumb_state, capture) = {
            let state = self.state.read(cx);
            if !state.show_state.is_visible() {
                return;
            }

            let opacity = if let VisibilityState::Animating { .. } = state.show_state {
                let elapsed = prepaint
                    .animation_start
                    .map(|start| start.elapsed())
                    .unwrap_or_default();
                let progress = elapsed.as_secs_f32() / SCROLLBAR_SHOW_DURATION.as_secs_f32();
                state.show_state.opacity(progress.min(1.0))
            } else {
                1.0
            };

            let capture = matches!(state.thumb_state, ThumbState::Dragging(_));

            (
                opacity,
                state.thumb_color,
                state.track_color,
                state.width,
                state.thumb_state,
                capture,
            )
        };

        let Some(layout) = &prepaint.layout else {
            return;
        };

        let parent_hitbox = prepaint.parent_hitbox.clone();
        let thumb_hitbox_for_closures = prepaint.thumb_hitbox.clone();

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(track_color) = track_color {
                let mut color = track_color;
                color.a *= opacity;
                window.paint_quad(quad(
                    layout.track_bounds,
                    Corners::default(),
                    color,
                    Edges::default(),
                    Hsla::transparent_black(),
                    gpui::BorderStyle::default(),
                ));
            }

            let mut thumb_color = thumb_color;
            if matches!(thumb_state, ThumbState::Dragging(_)) {
                thumb_color = Hsla {
                    h: thumb_color.h,
                    s: thumb_color.s * 0.9,
                    l: thumb_color.l + (1.0 - thumb_color.l) * 0.1,
                    a: thumb_color.a,
                };
            } else if matches!(thumb_state, ThumbState::Hover) {
                thumb_color = Hsla {
                    h: thumb_color.h,
                    s: thumb_color.s * 0.95,
                    l: thumb_color.l + (1.0 - thumb_color.l) * 0.05,
                    a: thumb_color.a,
                };
            }
            thumb_color.a *= opacity;

            window.paint_quad(quad(
                layout.thumb_bounds,
                Corners::all(width / 2.),
                thumb_color,
                Edges::default(),
                Hsla::transparent_black(),
                gpui::BorderStyle::default(),
            ));

            if let Some(ref hitbox) = prepaint.thumb_hitbox {
                window.set_cursor_style(CursorStyle::Arrow, hitbox);
            }
        });

        self.state.update(cx, |state, _| {
            state.last_layout = prepaint.layout.clone();
        });

        let phase = if capture {
            DispatchPhase::Capture
        } else {
            DispatchPhase::Bubble
        };

        window.on_mouse_event({
            let state = self.state.clone();
            move |event: &MouseDownEvent, event_phase, window, cx| {
                if event_phase != phase || event.button != MouseButton::Left {
                    return;
                }

                state.update(cx, |state, cx| {
                    let Some(layout) = &state.last_layout else {
                        return;
                    };

                    if layout.thumb_bounds.contains(&event.position) {
                        let offset = event.position.y - layout.thumb_bounds.origin.y;
                        state.thumb_state = ThumbState::Dragging(offset);
                        state.show(window, cx);
                        cx.stop_propagation();
                    } else if layout.track_bounds.contains(&event.position) {
                        let max_offset = state.scroll_handle.max_offset().height;
                        let thumb_center = layout.thumb_bounds.size.height / 2.;
                        let target_offset = layout.compute_offset_for_position(
                            event.position - Point::new(px(0.), thumb_center),
                            max_offset,
                        );
                        state.set_offset(target_offset, cx);
                        cx.stop_propagation();
                    }
                });
            }
        });

        window.on_mouse_event({
            let state = self.state.clone();
            let parent_hitbox = parent_hitbox.clone();
            let thumb_hitbox = thumb_hitbox_for_closures.clone();
            move |event: &MouseMoveEvent, event_phase, window, cx| {
                if event_phase != phase {
                    return;
                }

                state.update(cx, |state, cx| match state.thumb_state {
                    ThumbState::Dragging(drag_offset) if event.dragging() => {
                        if let Some(layout) = &state.last_layout {
                            let max_offset = state.scroll_handle.max_offset().height;
                            let new_offset =
                                layout.compute_drag_offset(event.position, drag_offset, max_offset);
                            state.set_offset(new_offset, cx);
                            cx.stop_propagation();
                        }
                    }
                    _ => {
                        let mouse_in_parent = parent_hitbox.is_hovered(window);
                        let was_in_parent = state.mouse_in_parent;
                        state.mouse_in_parent = mouse_in_parent;

                        if mouse_in_parent {
                            if !was_in_parent {
                                state.show(window, cx);
                            }

                            let new_thumb_state = thumb_hitbox
                                .as_ref()
                                .filter(|h| h.is_hovered(window))
                                .map(|_| ThumbState::Hover)
                                .unwrap_or(ThumbState::Inactive);

                            if state.thumb_state != new_thumb_state {
                                state.thumb_state = new_thumb_state;
                                cx.notify();
                            }
                        } else if was_in_parent {
                            state.thumb_state = ThumbState::Inactive;
                        }
                    }
                });
            }
        });

        window.on_mouse_event({
            let state = self.state.clone();
            move |_event: &MouseUpEvent, event_phase, window, cx| {
                if event_phase != phase {
                    return;
                }

                state.update(cx, |state, cx| {
                    if matches!(state.thumb_state, ThumbState::Dragging(_)) {
                        state.thumb_state = ThumbState::Inactive;
                        cx.notify();
                    }
                });
            }
        });

        window.on_mouse_event({
            let state = self.state.clone();
            move |_event: &ScrollWheelEvent, _phase, window, cx| {
                state.update(cx, |state, cx| {
                    if parent_hitbox.is_hovered(window) {
                        state.show(window, cx);
                    }
                });
            }
        });
    }
}

impl IntoElement for ScrollbarElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
