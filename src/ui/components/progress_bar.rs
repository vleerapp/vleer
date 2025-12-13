use std::{cell::RefCell, rc::Rc};

use gpui::*;

use crate::ui::variables::Variables;

type ChangeHandler = dyn FnMut(f32, &mut Window, &mut App);

pub struct ProgressSlider {
    pub(self) id: Option<ElementId>,
    pub(self) style: StyleRefinement,
    pub(self) current_time: f32,
    pub(self) duration: f32,
    pub(self) on_seek: Option<Rc<RefCell<ChangeHandler>>>,
    pub(self) hitbox: Option<Hitbox>,
}

impl ProgressSlider {
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn current_time(mut self, time: f32) -> Self {
        self.current_time = time;
        self
    }

    pub fn duration(mut self, duration: f32) -> Self {
        self.duration = duration;
        self
    }

    pub fn on_seek(mut self, func: impl FnMut(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_seek = Some(Rc::new(RefCell::new(func)));
        self
    }

    fn format_time(seconds: f32) -> String {
        let mins = (seconds / 60.0).floor() as i32;
        let secs = (seconds % 60.0).floor() as i32;
        format!("{:01}:{:02}", mins, secs)
    }
}

impl Styled for ProgressSlider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for ProgressSlider {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ProgressSlider {
    type RequestLayoutState = ();

    type PrepaintState = (Rc<RefCell<bool>>, Rc<RefCell<bool>>, Rc<RefCell<f32>>);

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
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        self.hitbox = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));

        window.with_optional_element_state(
            id,
            |v: Option<Option<(Rc<RefCell<bool>>, Rc<RefCell<bool>>, Rc<RefCell<f32>>)>>, _| {
                let (hovered, dragging, drag_value) = v.flatten().unwrap_or_else(|| {
                    (
                        Rc::new(RefCell::new(false)),
                        Rc::new(RefCell::new(false)),
                        Rc::new(RefCell::new(0.0)),
                    )
                });
                (
                    (hovered.clone(), dragging.clone(), drag_value.clone()),
                    Some((hovered, dragging, drag_value)),
                )
            },
        )
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        (hovered, dragging, drag_value): &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let variables = cx.global::<Variables>();

        let live_progress = if self.duration > 0.0 {
            (self.current_time / self.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let progress = if *dragging.borrow() {
            (*drag_value.borrow()).clamp(0.0, 1.0)
        } else {
            live_progress
        };

        let is_hovered = self
            .hitbox
            .as_ref()
            .map(|h| h.is_hovered(window))
            .unwrap_or(false);
        *hovered.borrow_mut() = is_hovered;

        let bar_height = px(16.0);
        let bar_y = bounds.origin.y + (bounds.size.height - bar_height) / 2.0;

        let filled_width = bounds.size.width * progress;
        if filled_width > px(0.0) {
            let filled_bounds = Bounds {
                origin: Point {
                    x: bounds.origin.x,
                    y: bar_y,
                },
                size: Size {
                    width: filled_width,
                    height: bar_height,
                },
            };

            window.paint_quad(quad(
                filled_bounds,
                Corners::default(),
                variables.accent,
                Edges::all(px(0.0)),
                rgb(0x000000),
                BorderStyle::Solid,
            ));
        }

        let is_dragging = *dragging.borrow();
        if is_hovered || is_dragging {
            if is_dragging {
                window.set_cursor_style(CursorStyle::PointingHand, self.hitbox.as_ref().unwrap());
            } else if is_hovered {
                window.set_cursor_style(CursorStyle::PointingHand, self.hitbox.as_ref().unwrap());
            }

            let thumb_width = px(10.0);
            let thumb_height = px(16.0);

            let thumb_x = bounds.origin.x + (bounds.size.width - thumb_width) * progress;
            let thumb_y = bounds.origin.y + (bounds.size.height - thumb_height) / 2.0;

            let thumb_bounds = Bounds {
                origin: Point {
                    x: thumb_x,
                    y: thumb_y,
                },
                size: Size {
                    width: thumb_width,
                    height: thumb_height,
                },
            };

            window.paint_quad(quad(
                thumb_bounds,
                Corners::default(),
                variables.text,
                Edges::all(px(0.0)),
                rgb(0x000000),
                BorderStyle::Solid,
            ));
        }

        let shown_time = if *dragging.borrow() {
            *drag_value.borrow() * self.duration
        } else {
            self.current_time
        };

        let current = Self::format_time(shown_time);
        let total = Self::format_time(self.duration);
        let time_text = format!("{} / {}", current, total);

        let filled_layout = if filled_width > px(0.0) {
            let filled_runs = [TextRun {
                len: time_text.len(),
                font: Font {
                    family: "Feature Mono".into(),
                    features: Default::default(),
                    fallbacks: None,
                    weight: FontWeight(500.0),
                    style: Default::default(),
                },
                color: variables.background.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            Some(window.text_system().shape_line(
                time_text.clone().into(),
                px(14.0),
                &filled_runs,
                None,
            ))
        } else {
            None
        };

        let unfilled_layout = if filled_width < bounds.size.width {
            let unfilled_runs = [TextRun {
                len: time_text.len(),
                font: Font {
                    family: "Feature Mono".into(),
                    features: Default::default(),
                    fallbacks: None,
                    weight: Default::default(),
                    style: Default::default(),
                },
                color: variables.accent.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            Some(window.text_system().shape_line(
                time_text.clone().into(),
                px(14.0),
                &unfilled_runs,
                None,
            ))
        } else {
            None
        };

        let text_width = unfilled_layout
            .as_ref()
            .map_or(px(0.0), |layout| layout.width);
        let text_x = bounds.origin.x + (bounds.size.width - text_width) / 2.0;
        let text_y = bounds.origin.y + (bounds.size.height - px(14.0)) / 2.0;
        let text_origin = Point {
            x: text_x,
            y: text_y,
        };

        if let Some(filled_layout) = filled_layout {
            let filled_clip = Bounds {
                origin: Point {
                    x: bounds.origin.x,
                    y: bounds.origin.y,
                },
                size: Size {
                    width: filled_width.max(px(1.0)),
                    height: bounds.size.height,
                },
            };
            window.with_content_mask(
                Some(ContentMask {
                    bounds: filled_clip,
                }),
                |window| {
                    filled_layout.paint(text_origin, px(14.0), window, cx).ok();
                },
            );
        }

        if let Some(unfilled_layout) = unfilled_layout {
            let unfilled_clip = Bounds {
                origin: Point {
                    x: bounds.origin.x + filled_width,
                    y: bounds.origin.y,
                },
                size: Size {
                    width: bounds.size.width - filled_width,
                    height: bounds.size.height,
                },
            };
            window.with_content_mask(
                Some(ContentMask {
                    bounds: unfilled_clip,
                }),
                |window| {
                    unfilled_layout
                        .paint(text_origin, px(14.0), window, cx)
                        .ok();
                },
            );
        }

        fn compute_value(pos: Point<Pixels>, bounds: Bounds<Pixels>) -> f32 {
            let relative = pos - bounds.origin;
            let relative_x: f32 = relative.x.into();
            let width: f32 = bounds.size.width.into();
            (relative_x / width).clamp(0.0, 1.0)
        }

        if let Some(on_seek) = self.on_seek.as_ref() {
            let func = on_seek.clone();

            let bounds_down = bounds;
            let dragging_down = dragging.clone();
            let drag_value_down = drag_value.clone();
            let func_down = func.clone();

            window.on_mouse_event(move |ev: &MouseDownEvent, _, window, cx| {
                if !bounds_down.contains(&ev.position) {
                    return;
                }

                window.prevent_default();
                cx.stop_propagation();

                let v = compute_value(ev.position, bounds_down);
                *dragging_down.borrow_mut() = true;
                *drag_value_down.borrow_mut() = v;

                (func_down.borrow_mut())(v, window, cx);
            });

            let bounds_move = bounds;
            let dragging_move = dragging.clone();
            let drag_value_move = drag_value.clone();

            window.on_mouse_event(move |ev: &MouseMoveEvent, _, window, _cx| {
                if !*dragging_move.borrow() {
                    return;
                }

                let v = compute_value(ev.position, bounds_move);
                *drag_value_move.borrow_mut() = v;
                window.refresh();
            });

            let bounds_up = bounds;
            let dragging_up = dragging.clone();
            let drag_value_up = drag_value.clone();
            let func_up = func.clone();

            window.on_mouse_event(move |ev: &MouseUpEvent, _, window, cx| {
                if !*dragging_up.borrow() {
                    return;
                }

                *dragging_up.borrow_mut() = false;

                let v = compute_value(ev.position, bounds_up);
                *drag_value_up.borrow_mut() = v;

                (func_up.borrow_mut())(v, window, cx);
            });
        }
    }
}

pub fn progress_slider() -> ProgressSlider {
    ProgressSlider {
        id: None,
        style: StyleRefinement::default(),
        current_time: 0.0,
        duration: 0.0,
        on_seek: None,
        hitbox: None,
    }
}
