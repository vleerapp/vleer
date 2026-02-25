use std::{cell::RefCell, rc::Rc};

use gpui::*;

use crate::ui::variables::Variables;

type ClickHandler = dyn FnMut(f32, &mut Window, &mut App);

#[derive(Clone, Copy, PartialEq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

pub struct Slider {
    pub(self) id: Option<ElementId>,
    pub(self) style: StyleRefinement,
    pub(self) value: f32,
    pub(self) orientation: Orientation,
    pub(self) on_change: Option<Rc<RefCell<ClickHandler>>>,
    pub(self) hitbox: Option<Hitbox>,
    pub(self) render_full: bool,
}

impl Slider {
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn value(mut self, value: f32) -> Self {
        self.value = value;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.orientation = Orientation::Vertical;
        self
    }

    pub fn render_full(mut self, render_full: bool) -> Self {
        self.render_full = render_full;
        self
    }

    pub fn on_change(mut self, func: impl FnMut(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(RefCell::new(func)));
        self
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for Slider {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Slider {
    type RequestLayoutState = ();

    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.id.clone()
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        (window.request_layout(style, [], cx), ())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        self.hitbox = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let variables = cx.global::<Variables>();

        window.set_cursor_style(CursorStyle::PointingHand, self.hitbox.as_ref().unwrap());

        let track_color = variables.border;
        let fill_color = variables.accent;
        let thumb_color = variables.text;

        match self.orientation {
            Orientation::Horizontal => {
                self.paint_horizontal(
                    bounds,
                    track_color,
                    fill_color,
                    thumb_color,
                    self.render_full,
                    window,
                );
            }
            Orientation::Vertical => {
                self.paint_vertical(
                    bounds,
                    track_color,
                    fill_color,
                    thumb_color,
                    self.render_full,
                    window,
                );
            }
        }

        if let Some(func) = self.on_change.as_ref() {
            let orientation = self.orientation;
            window.with_optional_element_state(
                id,
                move |v: Option<Option<Rc<RefCell<bool>>>>, cx| {
                    let mouse_in = v.flatten().unwrap_or_else(|| Rc::new(RefCell::new(false)));
                    let func = func.clone();
                    let func_copy = func.clone();

                    let mouse_in_1 = mouse_in.clone();

                    cx.on_mouse_event(move |ev: &MouseDownEvent, _, window, cx| {
                        if !bounds.contains(&ev.position) {
                            return;
                        }

                        window.prevent_default();
                        cx.stop_propagation();

                        let value = compute_value(ev.position, bounds, orientation);
                        (func.borrow_mut())(value, window, cx);
                        (*mouse_in_1.borrow_mut()) = true;
                    });

                    let mouse_in_2 = mouse_in.clone();

                    cx.on_mouse_event(move |ev: &MouseMoveEvent, _, window, cx| {
                        if *mouse_in_2.borrow() {
                            let value = compute_value(ev.position, bounds, orientation);
                            (func_copy.borrow_mut())(value, window, cx);
                        }
                    });

                    let mouse_in_3 = mouse_in.clone();

                    cx.on_mouse_event(move |_: &MouseUpEvent, _, _, _| {
                        (*mouse_in_3.borrow_mut()) = false;
                    });

                    ((), Some(mouse_in))
                },
            )
        }
    }
}

fn compute_value(position: Point<Pixels>, bounds: Bounds<Pixels>, orientation: Orientation) -> f32 {
    match orientation {
        Orientation::Horizontal => {
            let relative_x: f32 = (position.x - bounds.origin.x).into();
            let width: f32 = bounds.size.width.into();
            (relative_x / width).clamp(0.0, 1.0)
        }
        Orientation::Vertical => {
            let relative_y: f32 = (position.y - bounds.origin.y).into();
            let height: f32 = bounds.size.height.into();
            (1.0 - relative_y / height).clamp(0.0, 1.0)
        }
    }
}

impl Slider {
    fn paint_horizontal(
        &self,
        bounds: Bounds<Pixels>,
        track_color: Rgba,
        fill_color: Rgba,
        thumb_color: Rgba,
        render_full: bool,
        window: &mut Window,
    ) {
        let thumb_width = px(10.0);
        let thumb_height = px(16.0);

        let thumb_x = bounds.origin.x + (bounds.size.width - thumb_width) * self.value;
        let thumb_y = bounds.origin.y + (bounds.size.height - thumb_height) / 2.0;

        let dash_size = px(4.0);
        let gap_size = px(4.0);
        let line_height = px(2.0);
        let line_y = bounds.origin.y + (bounds.size.height - line_height) / 2.0;

        let track_start = bounds.origin.x;
        let track_end = bounds.origin.x + bounds.size.width;

        let thumb_right = thumb_x + thumb_width / 2.0;

        let mut x = track_start;
        while x < track_end {
            let dash_end = (x + dash_size).min(track_end);
            let dash_width = dash_end - x;

            if dash_width > px(0.0) {
                if !render_full && x >= thumb_right {
                    break;
                }

                let color = if render_full {
                    if x < thumb_right {
                        track_color
                    } else {
                        fill_color
                    }
                } else {
                    fill_color
                };
                window.paint_quad(quad(
                    Bounds {
                        origin: Point { x, y: line_y },
                        size: Size {
                            width: dash_width,
                            height: line_height,
                        },
                    },
                    Corners::default(),
                    color,
                    Edges::all(px(0.0)),
                    rgb(0x000000),
                    BorderStyle::Solid,
                ));
            }

            x = x + dash_size + gap_size;
        }

        window.paint_quad(quad(
            Bounds {
                origin: Point {
                    x: thumb_x,
                    y: thumb_y,
                },
                size: Size {
                    width: thumb_width,
                    height: thumb_height,
                },
            },
            Corners::default(),
            thumb_color,
            Edges::all(px(0.0)),
            rgb(0x000000),
            BorderStyle::Solid,
        ));
    }

    fn paint_vertical(
        &self,
        bounds: Bounds<Pixels>,
        track_color: Rgba,
        fill_color: Rgba,
        thumb_color: Rgba,
        render_full: bool,
        window: &mut Window,
    ) {
        let thumb_width = px(16.0);
        let thumb_height = px(10.0);

        let available = bounds.size.height - thumb_height;
        let thumb_y = bounds.origin.y + (1.0 - self.value) * available;
        let thumb_x = bounds.origin.x + (bounds.size.width - thumb_width) / 2.0;
        let thumb_center_y = thumb_y + thumb_height / 2.0;

        let dash_size = px(4.0);
        let gap_size = px(4.0);
        let line_width = px(2.0);
        let line_x = bounds.origin.x + (bounds.size.width - line_width) / 2.0;

        let track_top = bounds.origin.y;
        let track_bottom = bounds.origin.y + bounds.size.height;

        let mut y = track_top;
        while y < track_bottom {
            let dash_end = (y + dash_size).min(track_bottom);
            let dash_height = dash_end - y;
            if dash_height > px(0.0) {
                let color = if render_full {
                    if y < thumb_center_y {
                        track_color
                    } else {
                        fill_color
                    }
                } else {
                    fill_color
                };
                window.paint_quad(quad(
                    Bounds {
                        origin: Point { x: line_x, y },
                        size: Size {
                            width: line_width,
                            height: dash_height,
                        },
                    },
                    Corners::default(),
                    color,
                    Edges::all(px(0.0)),
                    rgb(0x000000),
                    BorderStyle::Solid,
                ));
            }
            y = y + dash_size + gap_size;
        }

        window.paint_quad(quad(
            Bounds {
                origin: Point {
                    x: thumb_x,
                    y: thumb_y,
                },
                size: Size {
                    width: thumb_width,
                    height: thumb_height,
                },
            },
            Corners::default(),
            thumb_color,
            Edges::all(px(0.0)),
            rgb(0x000000),
            BorderStyle::Solid,
        ));
    }
}

pub fn slider() -> Slider {
    Slider {
        id: None,
        style: StyleRefinement::default(),
        value: 0.0,
        orientation: Orientation::Horizontal,
        on_change: None,
        hitbox: None,
        render_full: false,
    }
}
