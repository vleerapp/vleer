use std::{
    panic::Location,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    App, ElementId, Entity, IntoElement, Pixels, Point, RenderOnce, Window, point, prelude::*, px,
};

use super::scrollbar::ScrollbarHandle;

const EASE: f32 = 0.12;
const HERTZ: f32 = 180.0;
const MAX_FRAME_TIME: Duration = Duration::from_millis(64);
const REST: Pixels = px(0.5);

struct ScrollMotion {
    shown: Point<Pixels>,
    target: Point<Pixels>,
    active: bool,
    frame_pending: bool,
    last_frame: Option<Instant>,
}

impl ScrollMotion {
    fn new(offset: Point<Pixels>) -> Self {
        Self {
            shown: offset,
            target: offset,
            active: false,
            frame_pending: false,
            last_frame: None,
        }
    }

    fn stop(&mut self, offset: Point<Pixels>) {
        self.shown = offset;
        self.target = offset;
        self.active = false;
        self.last_frame = None;
    }

    fn sync(&mut self, offset: Point<Pixels>, maximum: Point<Pixels>) {
        if !self.active || offset != clamp_offset(self.shown, maximum) {
            self.stop(offset);
        }
    }

    fn nudge(&mut self, offset: Point<Pixels>, maximum: Point<Pixels>) {
        let delta = offset - self.shown;
        let from = if self.active { self.target } else { self.shown };
        self.target = clamp_offset(from + delta, maximum);
        self.shown = clamp_offset(self.shown, maximum);
        self.active = self.target != self.shown;
        if !self.active {
            self.last_frame = None;
        }
    }

    fn advance(&mut self, elapsed: Duration, maximum: Point<Pixels>) -> Point<Pixels> {
        self.target = clamp_offset(self.target, maximum);
        let distance = self.target - self.shown;
        if distance.x.abs() < REST && distance.y.abs() < REST {
            self.stop(self.target);
        } else {
            let ease = 1.0 - (1.0 - EASE).powf(elapsed.min(MAX_FRAME_TIME).as_secs_f32() * HERTZ);
            self.shown = clamp_offset(self.shown + distance * ease, maximum);
        }
        self.shown
    }
}

fn clamp_offset(offset: Point<Pixels>, maximum: Point<Pixels>) -> Point<Pixels> {
    point(
        offset.x.clamp(-maximum.x.max(Pixels::ZERO), Pixels::ZERO),
        offset.y.clamp(-maximum.y.max(Pixels::ZERO), Pixels::ZERO),
    )
}

fn schedule_frame(
    motion: &Entity<ScrollMotion>,
    scroll: Rc<dyn ScrollbarHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    let schedule = motion.update(cx, |motion, _| {
        if !motion.active || motion.frame_pending {
            return false;
        }
        motion.frame_pending = true;
        true
    });
    if !schedule {
        return;
    }

    let motion = motion.downgrade();
    window.on_next_frame(move |window, cx| {
        let Some(motion) = motion.upgrade() else {
            return;
        };
        motion.update(cx, |motion, cx| {
            motion.frame_pending = false;
            let maximum = scroll.max_offset();
            motion.sync(scroll.offset(), maximum);
            if !motion.active {
                return;
            }
            let now = Instant::now();
            let elapsed = motion
                .last_frame
                .replace(now)
                .map(|last| now.duration_since(last))
                .unwrap_or(Duration::from_secs_f32(1.0 / HERTZ));
            scroll.set_offset(motion.advance(elapsed, maximum));
            cx.notify();
        });
        schedule_frame(&motion, scroll, window, cx);
    });
}

pub trait SmoothScrollable: InteractiveElement + IntoElement + Sized + 'static {
    #[track_caller]
    fn smooth_scroll(self, scroll: &(impl ScrollbarHandle + Clone)) -> SmoothScroll<Self> {
        SmoothScroll {
            element: self,
            scroll: Rc::new(scroll.clone()),
            id: ElementId::CodeLocation(*Location::caller()),
        }
    }
}

impl<E: InteractiveElement + IntoElement + 'static> SmoothScrollable for E {}

#[derive(IntoElement)]
pub struct SmoothScroll<E: InteractiveElement + IntoElement + 'static> {
    element: E,
    scroll: Rc<dyn ScrollbarHandle>,
    id: ElementId,
}

impl<E: InteractiveElement + IntoElement + 'static> RenderOnce for SmoothScroll<E> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let scroll = self.scroll;
        let motion = window.use_keyed_state(self.id, cx, |_, _| ScrollMotion::new(scroll.offset()));
        motion.update(cx, |motion, _| {
            motion.sync(scroll.offset(), scroll.max_offset())
        });

        self.element
            .capture_any_mouse_down({
                let motion = motion.clone();
                let scroll = scroll.clone();
                move |_, _, cx| {
                    motion.update(cx, |motion, _| motion.stop(scroll.offset()));
                }
            })
            .on_scroll_wheel(move |event, window, cx| {
                motion.update(cx, |motion, _| {
                    if event.delta.precise() {
                        motion.stop(scroll.offset());
                    } else {
                        motion.nudge(scroll.offset(), scroll.max_offset());
                        scroll.set_offset(motion.shown);
                    }
                });
                schedule_frame(&motion, scroll.clone(), window, cx);
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offset(y: f32) -> Point<Pixels> {
        point(px(0.0), px(y))
    }

    #[test]
    fn repeated_wheel_events_accumulate_and_reverse() {
        let mut motion = ScrollMotion::new(offset(0.0));
        motion.nudge(offset(-60.0), offset(1000.0));
        motion.advance(Duration::from_millis(16), offset(1000.0));
        assert!(motion.shown.y < px(0.0) && motion.shown.y > px(-60.0));
        motion.nudge(motion.shown + offset(-60.0), offset(1000.0));
        assert_eq!(motion.target, offset(-120.0));
        motion.nudge(motion.shown + offset(90.0), offset(1000.0));
        assert_eq!(motion.target, offset(-30.0));
    }

    #[test]
    fn easing_is_independent_of_refresh_rate() {
        let run = |frames, seconds| {
            let mut motion = ScrollMotion::new(offset(0.0));
            motion.nudge(offset(-1000.0), offset(2000.0));
            for _ in 0..frames {
                motion.advance(Duration::from_secs_f32(seconds), offset(2000.0));
            }
            motion.shown.y
        };
        assert!((run(6, 1.0 / 60.0) - run(18, 1.0 / 180.0)).abs() < px(0.01));
    }

    #[test]
    fn scroll_bounds_clamp_targets_and_shrinking_content() {
        let mut motion = ScrollMotion::new(offset(-100.0));
        motion.nudge(offset(-500.0), offset(200.0));
        assert_eq!(motion.target, offset(-200.0));
        let shown = motion.advance(Duration::from_millis(16), offset(40.0));
        assert_eq!(shown, offset(-40.0));
        motion.advance(Duration::from_millis(16), offset(40.0));
        assert!(!motion.active);
        motion.nudge(offset(500.0), offset(40.0));
        assert_eq!(motion.target, offset(0.0));
    }

    #[test]
    fn direct_input_and_programmatic_jumps_cancel_motion() {
        let mut motion = ScrollMotion::new(offset(0.0));
        motion.nudge(offset(-100.0), offset(1000.0));
        motion.stop(offset(-25.0));
        assert!(!motion.active);
        motion.nudge(offset(-50.0), offset(1000.0));
        motion.sync(offset(-800.0), offset(1000.0));
        assert!(!motion.active);
        assert_eq!(motion.shown, offset(-800.0));
    }

    #[test]
    fn motion_settles_exactly_and_empty_regions_stay_idle() {
        let mut motion = ScrollMotion::new(offset(0.0));
        motion.nudge(offset(-80.0), offset(0.0));
        assert!(!motion.active);
        motion.nudge(offset(-80.0), offset(1000.0));
        for _ in 0..120 {
            if motion.active {
                motion.advance(Duration::from_millis(16), offset(1000.0));
            }
        }
        assert!(!motion.active);
        assert_eq!(motion.shown, offset(-80.0));
    }
}
