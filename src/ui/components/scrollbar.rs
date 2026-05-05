use gpui::prelude::FluentBuilder;
use gpui::*;
use std::{cell::Cell, ops::Deref, panic::Location, rc::Rc, time::Instant};

use crate::ui::variables::Variables;

const DEFAULT_WIDTH: Pixels = px(16.);
const THUMB_WIDTH: Pixels = px(4.);
const MIN_THUMB_SIZE: f32 = 48.;
const FADE_OUT_DURATION: f32 = 3.0;

pub trait AxisExt {
    fn is_vertical(&self) -> bool;
}
impl AxisExt for Axis {
    fn is_vertical(&self) -> bool {
        matches!(self, Axis::Vertical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
    Both,
}
impl From<Axis> for ScrollbarAxis {
    fn from(axis: Axis) -> Self {
        match axis {
            Axis::Vertical => Self::Vertical,
            Axis::Horizontal => Self::Horizontal,
        }
    }
}
impl ScrollbarAxis {
    fn all(&self) -> Vec<Axis> {
        match self {
            Self::Vertical => vec![Axis::Vertical],
            Self::Horizontal => vec![Axis::Horizontal],
            Self::Both => vec![Axis::Horizontal, Axis::Vertical],
        }
    }
}

pub trait ScrollbarHandle: 'static {
    fn offset(&self) -> Point<Pixels>;
    fn set_offset(&self, offset: Point<Pixels>);
    fn content_size(&self) -> Size<Pixels>;
    fn start_drag(&self) {}
    fn end_drag(&self) {}
}
impl ScrollbarHandle for ScrollHandle {
    fn offset(&self) -> Point<Pixels> {
        self.offset()
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset(offset);
    }
    fn content_size(&self) -> Size<Pixels> {
        (self.max_offset() + self.bounds().size.into()).into()
    }
}
impl ScrollbarHandle for UniformListScrollHandle {
    fn offset(&self) -> Point<Pixels> {
        self.0.borrow().base_handle.offset()
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.0.borrow_mut().base_handle.set_offset(offset)
    }
    fn content_size(&self) -> Size<Pixels> {
        let base_handle = &self.0.borrow().base_handle;
        (base_handle.max_offset() + base_handle.bounds().size.into()).into()
    }
}
impl ScrollbarHandle for ListState {
    fn offset(&self) -> Point<Pixels> {
        self.scroll_px_offset_for_scrollbar()
    }
    fn set_offset(&self, offset: Point<Pixels>) {
        self.set_offset_from_scrollbar(offset);
    }
    fn content_size(&self) -> Size<Pixels> {
        self.viewport_bounds().size + self.max_offset_for_scrollbar().into()
    }
    fn start_drag(&self) {
        self.scrollbar_drag_started();
    }
    fn end_drag(&self) {
        self.scrollbar_drag_ended();
    }
}

#[derive(Debug, Clone)]
struct ScrollbarState(Rc<Cell<ScrollbarStateInner>>);
#[derive(Debug, Clone, Copy)]
struct ScrollbarStateInner {
    hovered_axis: Option<Axis>,
    hovered_on_thumb: Option<Axis>,
    dragged_axis: Option<Axis>,
    drag_pos: Point<Pixels>,
    last_scroll_offset: Point<Pixels>,
    last_scroll_time: Option<Instant>,
    last_update: Instant,
}
impl Default for ScrollbarState {
    fn default() -> Self {
        Self(Rc::new(Cell::new(ScrollbarStateInner {
            hovered_axis: None,
            hovered_on_thumb: None,
            dragged_axis: None,
            drag_pos: point(px(0.), px(0.)),
            last_scroll_offset: point(px(0.), px(0.)),
            last_scroll_time: None,
            last_update: Instant::now(),
        })))
    }
}
impl Deref for ScrollbarState {
    type Target = Rc<Cell<ScrollbarStateInner>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl ScrollbarStateInner {
    fn with_drag_pos(&self, axis: Axis, pos: Point<Pixels>) -> Self {
        let mut state = *self;
        if axis.is_vertical() {
            state.drag_pos.y = pos.y;
        } else {
            state.drag_pos.x = pos.x;
        }
        state.dragged_axis = Some(axis);
        state
    }
    fn with_unset_drag_pos(&self) -> Self {
        let mut state = *self;
        state.dragged_axis = None;
        state
    }
    fn with_hovered(&self, axis: Option<Axis>) -> Self {
        let mut state = *self;
        state.hovered_axis = axis;
        if axis.is_some() {
            state.last_scroll_time = Some(Instant::now());
        }
        state
    }
    fn with_hovered_on_thumb(&self, axis: Option<Axis>) -> Self {
        let mut state = *self;
        state.hovered_on_thumb = axis;
        if self.is_scrollbar_visible() && axis.is_some() {
            state.last_scroll_time = Some(Instant::now());
        }
        state
    }
    fn with_last_scroll(&self, offset: Point<Pixels>, time: Option<Instant>) -> Self {
        let mut state = *self;
        state.last_scroll_offset = offset;
        state.last_scroll_time = time;
        state
    }
    fn with_last_update(&self, t: Instant) -> Self {
        let mut state = *self;
        state.last_update = t;
        state
    }
    fn is_scrollbar_visible(&self) -> bool {
        if self.dragged_axis.is_some() {
            return true;
        }
        if let Some(last_time) = self.last_scroll_time {
            let elapsed = Instant::now().duration_since(last_time).as_secs_f32();
            elapsed < FADE_OUT_DURATION
        } else {
            false
        }
    }
}

pub struct Scrollbar {
    id: ElementId,
    axis: ScrollbarAxis,
    scroll_handle: Rc<dyn ScrollbarHandle>,
    scroll_size: Option<Size<Pixels>>,
    max_fps: usize,
    width: Pixels,
}

impl Scrollbar {
    #[track_caller]
    pub fn new<H: ScrollbarHandle + Clone>(scroll_handle: &H) -> Self {
        let caller = Location::caller();
        Self {
            id: ElementId::CodeLocation(*caller),
            axis: ScrollbarAxis::Both,
            scroll_handle: Rc::new(scroll_handle.clone()),
            max_fps: 60,
            scroll_size: None,
            width: DEFAULT_WIDTH,
        }
    }
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }
    pub fn axis(mut self, axis: impl Into<ScrollbarAxis>) -> Self {
        self.axis = axis.into();
        self
    }
    pub fn scroll_size(mut self, size: Size<Pixels>) -> Self {
        self.scroll_size = Some(size);
        self
    }
    fn get_colors(
        &self,
        cx: &App,
        state: &ScrollbarStateInner,
        axis: Axis,
    ) -> (Hsla, Hsla, Pixels) {
        let variables = cx.global::<Variables>();
        let default_thumb = variables.accent;
        let hover_thumb = variables.accent_hover;
        let default_track = transparent_black();
        let is_dragged = state.dragged_axis == Some(axis);
        let is_hovered_thumb = state.hovered_on_thumb == Some(axis);
        let is_hovered_bar = state.hovered_axis == Some(axis);
        if is_dragged || is_hovered_thumb {
            (hover_thumb.into(), default_track, THUMB_WIDTH)
        } else if is_hovered_bar || state.is_scrollbar_visible() {
            (default_thumb.into(), default_track, THUMB_WIDTH)
        } else {
            (default_track, default_track, THUMB_WIDTH)
        }
    }
}
impl IntoElement for Scrollbar {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct PrepaintState {
    hitbox: Hitbox,
    scrollbar_state: ScrollbarState,
    states: Vec<AxisPrepaintState>,
}
struct AxisPrepaintState {
    axis: Axis,
    bar_hitbox: Hitbox,
    bounds: Bounds<Pixels>,
    track_color: Hsla,
    thumb_bounds: Bounds<Pixels>,
    thumb_fill_bounds: Bounds<Pixels>,
    thumb_color: Hsla,
    scroll_size: Pixels,
    container_size: Pixels,
    thumb_size: Pixels,
    margin_end: Pixels,
}
impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.position = Position::Absolute;
        style.flex_grow = 1.0;
        style.flex_shrink = 1.0;
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, None, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.insert_hitbox(bounds, HitboxBehavior::Normal)
        });
        let state = window
            .use_state(cx, |_, _| ScrollbarState::default())
            .read(cx)
            .clone();
        let mut states = vec![];
        let mut has_both = matches!(self.axis, ScrollbarAxis::Both);
        let scroll_size = self
            .scroll_size
            .unwrap_or(self.scroll_handle.content_size());
        for axis in self.axis.all() {
            let is_vertical = axis.is_vertical();
            let (scroll_area_size, container_size, scroll_position) = if is_vertical {
                (
                    scroll_size.height,
                    hitbox.size.height,
                    self.scroll_handle.offset().y,
                )
            } else {
                (
                    scroll_size.width,
                    hitbox.size.width,
                    self.scroll_handle.offset().x,
                )
            };
            let margin_end = if has_both && !is_vertical {
                self.width
            } else {
                px(0.)
            };
            if scroll_area_size <= container_size {
                has_both = false;
                continue;
            }
            let thumb_length =
                (container_size / scroll_area_size * container_size).max(px(MIN_THUMB_SIZE));
            let thumb_start = -(scroll_position / (scroll_area_size - container_size)
                * (container_size - margin_end - thumb_length));
            let thumb_end = (thumb_start + thumb_length).min(container_size - margin_end);
            let bounds = Bounds {
                origin: if is_vertical {
                    point(
                        hitbox.origin.x + hitbox.size.width - self.width,
                        hitbox.origin.y,
                    )
                } else {
                    point(
                        hitbox.origin.x,
                        hitbox.origin.y + hitbox.size.height - self.width,
                    )
                },
                size: if is_vertical {
                    size(self.width, hitbox.size.height)
                } else {
                    size(hitbox.size.width, self.width)
                },
            };
            let (thumb_color, track_color, thumb_width) = self.get_colors(&cx, &state.get(), axis);
            let thumb_length = thumb_end - thumb_start;
            let thumb_bounds = if is_vertical {
                Bounds {
                    origin: point(bounds.origin.x, bounds.origin.y + thumb_start),
                    size: size(self.width, thumb_length),
                }
            } else {
                Bounds {
                    origin: point(bounds.origin.x + thumb_start, bounds.origin.y),
                    size: size(thumb_length, self.width),
                }
            };
            let thumb_fill_bounds = if is_vertical {
                Bounds {
                    origin: point(
                        bounds.origin.x + self.width - thumb_width,
                        bounds.origin.y + thumb_start,
                    ),
                    size: size(thumb_width, thumb_length),
                }
            } else {
                Bounds {
                    origin: point(
                        bounds.origin.x + thumb_start,
                        bounds.origin.y + self.width - thumb_width,
                    ),
                    size: size(thumb_length, thumb_width),
                }
            };
            let bar_hitbox = window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            });
            states.push(AxisPrepaintState {
                axis,
                bar_hitbox,
                bounds,
                track_color,
                thumb_bounds,
                thumb_fill_bounds,
                thumb_color,
                scroll_size: scroll_area_size,
                container_size,
                thumb_size: thumb_length,
                margin_end,
            });
        }
        PrepaintState {
            hitbox,
            states,
            scrollbar_state: state,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let state = &prepaint.scrollbar_state;
        let view_id = window.current_view();
        let hitbox_bounds = prepaint.hitbox.bounds;
        if self.scroll_handle.offset() != state.get().last_scroll_offset {
            let was_dragging = state.get().dragged_axis.is_some();
            state.set(
                state
                    .get()
                    .with_last_scroll(self.scroll_handle.offset(), Some(Instant::now())),
            );
            if !was_dragging {
                cx.notify(view_id);
            }
        }
        window.with_content_mask(
            Some(ContentMask {
                bounds: hitbox_bounds,
            }),
            |window| {
                for axis_state in &prepaint.states {
                    let axis = axis_state.axis;
                    let bounds = axis_state.bounds;
                    let thumb_bounds = axis_state.thumb_bounds;
                    let scroll_area_size = axis_state.scroll_size;
                    let container_size = axis_state.container_size;
                    let thumb_size = axis_state.thumb_size;
                    let margin_end = axis_state.margin_end;
                    window.set_cursor_style(CursorStyle::default(), &axis_state.bar_hitbox);
                    window.paint_layer(hitbox_bounds, |cx| {
                        cx.paint_quad(fill(bounds, axis_state.track_color));
                        cx.paint_quad(fill(axis_state.thumb_fill_bounds, axis_state.thumb_color));
                    });
                    window.on_mouse_event({
                        let state = state.clone();
                        let scroll_handle = self.scroll_handle.clone();
                        move |event: &ScrollWheelEvent, phase, _, cx| {
                            if phase.bubble() && hitbox_bounds.contains(&event.position) {
                                if scroll_handle.offset() != state.get().last_scroll_offset {
                                    state.set(state.get().with_last_scroll(
                                        scroll_handle.offset(),
                                        Some(Instant::now()),
                                    ));
                                    cx.notify(view_id);
                                }
                            }
                        }
                    });
                    let safe_range = (-scroll_area_size + container_size)..px(0.);

                    window.on_mouse_event({
                        let state = state.clone();
                        let scroll_handle = self.scroll_handle.clone();
                        let safe_range = safe_range.clone();
                        move |event: &MouseDownEvent, phase, _, cx| {
                            if phase.bubble() && bounds.contains(&event.position) {
                                cx.stop_propagation();
                                if thumb_bounds.contains(&event.position) {
                                    let pos = event.position - thumb_bounds.origin;
                                    scroll_handle.start_drag();
                                    state.set(state.get().with_drag_pos(axis, pos));
                                    cx.notify(view_id);
                                } else {
                                    let offset = scroll_handle.offset();
                                    let percentage = if axis.is_vertical() {
                                        (event.position.y - thumb_size / 2. - bounds.origin.y)
                                            / (bounds.size.height - thumb_size)
                                    } else {
                                        (event.position.x - thumb_size / 2. - bounds.origin.x)
                                            / (bounds.size.width - thumb_size)
                                    }
                                    .min(1.);
                                    if axis.is_vertical() {
                                        scroll_handle.set_offset(point(
                                            offset.x,
                                            (-scroll_area_size * percentage)
                                                .clamp(safe_range.start, safe_range.end),
                                        ));
                                    } else {
                                        scroll_handle.set_offset(point(
                                            (-scroll_area_size * percentage)
                                                .clamp(safe_range.start, safe_range.end),
                                            offset.y,
                                        ));
                                    }
                                }
                            }
                        }
                    });
                    window.on_mouse_event({
                        let state = state.clone();
                        let scroll_handle = self.scroll_handle.clone();
                        let safe_range = safe_range.clone();
                        let max_fps_duration =
                            std::time::Duration::from_millis((1000 / self.max_fps) as u64);
                        move |event: &MouseMoveEvent, _, _, cx| {
                            let is_dragging =
                                state.get().dragged_axis == Some(axis) && event.dragging();

                            let mut notify = false;
                            if !is_dragging {
                                if bounds.contains(&event.position) {
                                    if state.get().hovered_axis != Some(axis) {
                                        state.set(state.get().with_hovered(Some(axis)));
                                        notify = true;
                                    }
                                } else if state.get().hovered_axis == Some(axis) {
                                    state.set(state.get().with_hovered(None));
                                    notify = true;
                                }
                                if thumb_bounds.contains(&event.position) {
                                    if state.get().hovered_on_thumb != Some(axis) {
                                        state.set(state.get().with_hovered_on_thumb(Some(axis)));
                                        notify = true;
                                    }
                                } else if state.get().hovered_on_thumb == Some(axis) {
                                    state.set(state.get().with_hovered_on_thumb(None));
                                    notify = true;
                                }
                            }
                            if is_dragging {
                                cx.stop_propagation();
                                if state.get().last_update.elapsed() > max_fps_duration {
                                    let drag_pos = state.get().drag_pos;
                                    let percentage = (if axis.is_vertical() {
                                        (event.position.y - drag_pos.y - bounds.origin.y)
                                            / (bounds.size.height - thumb_size)
                                    } else {
                                        (event.position.x - drag_pos.x - bounds.origin.x)
                                            / (bounds.size.width - thumb_size - margin_end)
                                    })
                                    .clamp(0., 1.);
                                    let offset = if axis.is_vertical() {
                                        point(
                                            scroll_handle.offset().x,
                                            (-(scroll_area_size - container_size) * percentage)
                                                .clamp(safe_range.start, safe_range.end),
                                        )
                                    } else {
                                        point(
                                            (-(scroll_area_size - container_size) * percentage)
                                                .clamp(safe_range.start, safe_range.end),
                                            scroll_handle.offset().y,
                                        )
                                    };
                                    scroll_handle.set_offset(offset);
                                    state.set(state.get().with_last_update(Instant::now()));
                                    notify = true;
                                }
                            }
                            if notify {
                                cx.notify(view_id);
                            }
                        }
                    });
                    window.on_mouse_event({
                        let state = state.clone();
                        let scroll_handle = self.scroll_handle.clone();
                        move |_: &MouseUpEvent, phase, _, cx| {
                            if phase.bubble() {
                                scroll_handle.end_drag();
                                state.set(state.get().with_unset_drag_pos());
                                cx.notify(view_id);
                            }
                        }
                    });
                }
            },
        );
    }
}

pub trait ScrollableElement: IntoElement + Sized {
    fn overflow_y_scrollbar(self) -> Scrollable<Self>
    where
        Self: InteractiveElement + Styled + ParentElement + Element,
    {
        Scrollable::new(self, ScrollbarAxis::Vertical)
    }
}

impl<E: IntoElement> ScrollableElement for E {}

#[derive(IntoElement)]
pub struct Scrollable<E: InteractiveElement + Styled + ParentElement + Element> {
    id: ElementId,
    element: E,
    axis: ScrollbarAxis,
}

impl<E> Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    #[track_caller]
    pub fn new(element: E, axis: impl Into<ScrollbarAxis>) -> Self {
        let caller = Location::caller();
        Self {
            id: ElementId::CodeLocation(*caller),
            element,
            axis: axis.into(),
        }
    }
}

impl<E> Styled for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E> ParentElement for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element,
{
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements)
    }
}

impl InteractiveElement for Scrollable<Div> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

impl InteractiveElement for Scrollable<Stateful<Div>> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

impl<E> RenderOnce for Scrollable<E>
where
    E: InteractiveElement + Styled + ParentElement + Element + 'static,
{
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let scroll_handle = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::default())
            .read(cx)
            .clone();

        *self.element.style() = StyleRefinement::default();

        div()
            .id(self.id)
            .size_full()
            .relative()
            .child(
                div()
                    .id("scroll-area")
                    .flex()
                    .size_full()
                    .track_scroll(&scroll_handle)
                    .map(|this| match self.axis {
                        ScrollbarAxis::Vertical => this.flex_col().overflow_y_scroll(),
                        ScrollbarAxis::Horizontal => this.flex_row().overflow_x_scroll(),
                        ScrollbarAxis::Both => this.overflow_scroll(),
                    })
                    .child(self.element.flex_1()),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .child(
                        Scrollbar::new(&scroll_handle)
                            .id("scrollbar")
                            .axis(self.axis),
                    ),
            )
    }
}
