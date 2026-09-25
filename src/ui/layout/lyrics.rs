use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::{Duration, Instant};

use crate::{
    data::{
        db::repo::Database,
        lyrics::{Line, Lyrics, active_line, is_gap, resolve},
        models::Cuid,
    },
    media::{playback::Playback, queue::Queue},
    services::lrclib::LrclibClient,
    ui::{
        components::{
            div::{flex_col, flex_row},
            scrollbar::{Scrollbar, ScrollbarAxis},
            scroller::SmoothScrollable,
        },
        layout::SidePanel,
        variables::Variables,
    },
};

const TICK: Duration = Duration::from_millis(100);
const EASE: f32 = 0.08;
const HERTZ: f32 = 180.0;
const MAX_FRAME_TIME: Duration = Duration::from_millis(64);
const REST: Pixels = px(0.5);

const LINE_SIZE: f32 = 20.0;
const LINE_HEIGHT: f32 = 30.0;
const SECONDARY_SIZE: f32 = 14.0;
const SECONDARY_HEIGHT: f32 = 20.0;
const PLAIN_SIZE: f32 = 16.0;
const PLAIN_HEIGHT: f32 = 26.0;
const GAP_HEIGHT: f32 = 34.0;
const DOT_SIZE: f32 = 9.0;
const IDLE_RESYNC: Duration = Duration::from_secs(3);
const FADE_TOP: f32 = 16.0;
const FADE_BOTTOM: f32 = 72.0;

enum Load {
    Idle,
    Loading,
    Missing,
    Ready(Lyrics),
}

pub struct LyricsPane {
    song_id: Option<Cuid>,
    load: Load,
    active: Option<usize>,
    follow: Option<usize>,
    snap: bool,
    target: Option<Pixels>,
    last_frame: Option<Instant>,
    shown: Vec<f32>,
    fade_frame: Option<Instant>,
    detached: bool,
    last_activity: Instant,
    last_offset: Pixels,
    scroll: ScrollHandle,
    task: Option<Task<()>>,
}

impl LyricsPane {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Queue>(|this, cx| this.sync(cx))
            .detach();
        cx.observe_global::<SidePanel>(|this, cx| this.sync(cx))
            .detach();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = cx.update(|cx| this.update(cx, |this, cx| this.tick(cx)).is_ok());
                if !alive {
                    break;
                }
            }
        })
        .detach();

        Self {
            song_id: None,
            load: Load::Idle,
            active: None,
            follow: None,
            snap: true,
            target: None,
            last_frame: None,
            shown: Vec::new(),
            fade_frame: None,
            detached: false,
            last_activity: Instant::now(),
            last_offset: Pixels::ZERO,
            scroll: ScrollHandle::new(),
            task: None,
        }
    }

    fn is_visible(cx: &App) -> bool {
        cx.try_global::<SidePanel>().copied() == Some(SidePanel::Lyrics)
    }

    fn reset_motion(&mut self) {
        self.active = None;
        self.follow = None;
        self.target = None;
        self.snap = true;
        self.last_frame = None;
        self.shown.clear();
        self.fade_frame = None;
        self.detached = false;
        self.set_y(px(0.0));
    }

    fn set_y(&mut self, y: Pixels) {
        self.scroll.set_offset(point(px(0.0), y));
        self.last_offset = y;
    }

    fn detach(&mut self) {
        self.detached = true;
        self.last_activity = Instant::now();
        self.target = None;
        self.follow = None;
        self.last_frame = None;
    }

    fn resync(&mut self, cx: &mut Context<Self>) {
        if !self.detached {
            return;
        }
        self.detached = false;
        self.snap = false;
        if let Load::Ready(Lyrics::Synced(lines)) = &self.load
            && let Some(active) = self.active
        {
            self.follow = Some(anchor(lines, active));
        }
        cx.notify();
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        if !Self::is_visible(cx) {
            return;
        }

        let id = cx.global::<Queue>().get_current_song_id();
        if id == self.song_id && !matches!(self.load, Load::Idle) {
            return;
        }

        self.song_id = id.clone();
        self.reset_motion();
        self.task = None;

        let Some(id) = id else {
            self.load = Load::Missing;
            cx.notify();
            return;
        };

        self.load = Load::Loading;
        cx.notify();

        let db = cx.global::<Database>().clone();
        let client = cx.global::<LrclibClient>().clone();
        let bg = cx.background_executor().clone();

        self.task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let lyrics = bg
                .spawn(async move {
                    let song = db.get_song(&id).ok().flatten()?;
                    resolve(&db, &client, &song)
                })
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    this.load = match lyrics {
                        Some(lyrics) => Load::Ready(lyrics),
                        None => Load::Missing,
                    };
                    cx.notify();
                })
            })
            .ok();
        }));
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !Self::is_visible(cx) {
            return;
        }
        if self.detached && self.last_activity.elapsed() >= IDLE_RESYNC {
            self.resync(cx);
        }
        let Load::Ready(Lyrics::Synced(lines)) = &self.load else {
            return;
        };
        let active = active_line(lines, cx.global::<Playback>().get_position());
        if active == self.active {
            return;
        }
        self.snap = self.active.is_none();
        self.active = active;
        if !self.detached {
            self.follow = active.map(|active| anchor(lines, active));
        }
        cx.notify();
    }

    fn top_offset(&self, index: usize, inset: Pixels) -> Option<Pixels> {
        let item = self.scroll.bounds_for_item(index)?;
        let view = self.scroll.bounds();
        if view.size.height <= Pixels::ZERO {
            return None;
        }
        let content_y = item.origin.y - view.origin.y;
        let max = self.scroll.max_offset().y.max(Pixels::ZERO);
        Some((inset - content_y).clamp(-max, Pixels::ZERO))
    }

    fn step_fade(&mut self, count: usize, active: Option<usize>, window: &mut Window) {
        if self.shown.len() != count {
            self.shown = (0..count).map(|i| line_opacity(i, active)).collect();
            self.fade_frame = None;
            return;
        }
        let now = Instant::now();
        let elapsed = self
            .fade_frame
            .replace(now)
            .map(|last| now.duration_since(last))
            .unwrap_or(Duration::from_secs_f32(1.0 / HERTZ));
        let ease = 1.0 - (1.0 - EASE).powf(elapsed.min(MAX_FRAME_TIME).as_secs_f32() * HERTZ);
        let mut moving = false;
        for (i, shown) in self.shown.iter_mut().enumerate() {
            let target = line_opacity(i, active);
            let distance = target - *shown;
            if distance.abs() < 0.005 {
                *shown = target;
            } else {
                *shown += distance * ease;
                moving = true;
            }
        }
        if moving {
            window.request_animation_frame();
        } else {
            self.fade_frame = None;
        }
    }

    fn drive_motion(&mut self, window: &mut Window, inset: Pixels) {
        let current = self.scroll.offset().y;
        if !self.detached && self.target.is_none() && (current - self.last_offset).abs() > px(1.0) {
            self.detach();
        }
        self.last_offset = current;

        if let Some(index) = self.follow {
            match self.top_offset(index, inset) {
                Some(y) if self.snap => {
                    self.set_y(y);
                    self.follow = None;
                    self.snap = false;
                }
                Some(y) => {
                    self.target = Some(y);
                    self.follow = None;
                }
                None => window.request_animation_frame(),
            }
        }

        let Some(target) = self.target else {
            return;
        };
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .replace(now)
            .map(|last| now.duration_since(last))
            .unwrap_or(Duration::from_secs_f32(1.0 / HERTZ));
        let offset = self.scroll.offset();
        let distance = target - offset.y;
        if distance.abs() < REST {
            self.set_y(target);
            self.target = None;
            self.last_frame = None;
            return;
        }
        let ease = 1.0 - (1.0 - EASE).powf(elapsed.min(MAX_FRAME_TIME).as_secs_f32() * HERTZ);
        self.set_y(offset.y + distance * ease);
        window.request_animation_frame();
    }
}

fn anchor(lines: &[Line], active: usize) -> usize {
    (0..active)
        .rev()
        .find(|&i| !lines[i].text.is_empty())
        .unwrap_or(active)
}

fn message(text: &'static str, variables: &Variables) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(variables.text_secondary)
        .child(text)
}

fn line_opacity(index: usize, active: Option<usize>) -> f32 {
    match active {
        None => 0.5,
        Some(active) => match index.abs_diff(active) {
            0 => 1.0,
            distance => (0.55 - 0.1 * (distance - 1) as f32).max(0.12),
        },
    }
}

fn gap_row(progress: Option<f32>, variables: &Variables) -> Div {
    let row = flex_row().h(px(GAP_HEIGHT)).gap(px(8.0)).items_center();
    let Some(progress) = progress else {
        return row;
    };
    row.children((0..3).map(|i| {
        let lit = (progress * 3.0 - i as f32).clamp(0.0, 1.0);
        div()
            .size(px(DOT_SIZE))
            .rounded_full()
            .bg(Hsla::from(variables.text).opacity(0.25 + 0.75 * lit))
    }))
}

fn synced_rows(
    lines: &[Line],
    shown: &[f32],
    active: Option<usize>,
    position: f32,
    variables: &Variables,
    cx: &mut Context<LyricsPane>,
) -> Vec<AnyElement> {
    let variables = *variables;
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if line.text.is_empty() {
                if !is_gap(lines, i) {
                    return div().id(("lyric-line", i)).h(px(10.0)).into_any_element();
                }
                let progress = (active == Some(i)).then(|| {
                    let span = (lines[i + 1].at - line.at).max(f32::EPSILON);
                    ((position - line.at) / span).clamp(0.0, 1.0)
                });
                return div()
                    .id(("lyric-line", i))
                    .child(gap_row(progress, &variables))
                    .into_any_element();
            }

            let at = line.at;
            let opacity = shown.get(i).copied().unwrap_or(0.5);
            div()
                .id(("lyric-line", i))
                .py(px(6.0))
                .text_size(px(LINE_SIZE))
                .line_height(px(LINE_HEIGHT))
                .font_weight(FontWeight(600.0))
                .cursor_pointer()
                .text_color(Hsla::from(variables.text).opacity(opacity))
                .hover(|s| s.text_color(variables.text))
                .on_click(cx.listener(move |_this, _event, window, cx| {
                    cx.update_global::<Playback, _>(|playback, _cx| {
                        if let Err(e) = playback.seek(at) {
                            tracing::error!("Failed to seek: {}", e);
                        }
                    });
                    window.refresh();
                }))
                .child(flex_col().gap(px(2.0)).child(line.text.clone()).when_some(
                    line.secondary.clone(),
                    |this, secondary| {
                        this.child(
                            div()
                                .text_size(px(SECONDARY_SIZE))
                                .line_height(px(SECONDARY_HEIGHT))
                                .font_weight(FontWeight(500.0))
                                .opacity(0.7)
                                .child(secondary),
                        )
                    },
                ))
                .into_any_element()
        })
        .collect()
}

fn edge_fade(top: bool, strength: f32, variables: &Variables) -> Div {
    let solid = Hsla::from(variables.background);
    let clear = solid.opacity(0.0);
    let fade = div()
        .absolute()
        .opacity(strength)
        .left_0()
        .right_0()
        .h(px(if top { FADE_TOP } else { FADE_BOTTOM }))
        .bg(linear_gradient(
            if top { 180.0 } else { 0.0 },
            linear_color_stop(solid, 0.0),
            linear_color_stop(clear, 1.0),
        ));
    if top { fade.top_0() } else { fade.bottom_0() }
}

impl Render for LyricsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();

        match &self.load {
            Load::Idle => return message("No song playing", &variables).into_any_element(),
            Load::Loading => return message("Loading lyrics", &variables).into_any_element(),
            Load::Missing => return message("No lyrics found", &variables).into_any_element(),
            Load::Ready(Lyrics::Instrumental) => {
                return message("Instrumental", &variables).into_any_element();
            }
            Load::Ready(_) => {}
        }

        self.drive_motion(window, px(variables.padding_16));
        if let Load::Ready(Lyrics::Synced(lines)) = &self.load {
            let count = lines.len();
            self.step_fade(count, self.active, window);
        }

        let position = cx.global::<Playback>().get_position();
        let playing = cx.global::<Playback>().get_playing();
        let active = self.active;
        let offset = self.scroll.offset().y;
        let hidden_below = self.scroll.max_offset().y + offset;
        let top_fade = (-f32::from(offset) / FADE_TOP).clamp(0.0, 1.0);
        let bottom_fade = (f32::from(hidden_below) / FADE_BOTTOM).clamp(0.0, 1.0);

        let rows: Vec<AnyElement> = match &self.load {
            Load::Ready(Lyrics::Plain(lines)) => lines
                .iter()
                .enumerate()
                .map(|(i, text)| {
                    div()
                        .id(("lyric-plain", i))
                        .py(px(3.0))
                        .text_size(px(PLAIN_SIZE))
                        .line_height(px(PLAIN_HEIGHT))
                        .text_color(variables.text)
                        .child(if text.is_empty() {
                            "\u{00a0}".to_string()
                        } else {
                            text.clone()
                        })
                        .into_any_element()
                })
                .collect(),
            Load::Ready(Lyrics::Synced(lines)) => {
                if playing && active.is_some_and(|i| is_gap(lines, i)) {
                    window.request_animation_frame();
                }
                synced_rows(lines, &self.shown, active, position, &variables, cx)
            }
            _ => Vec::new(),
        };

        div()
            .id("lyrics-view")
            .relative()
            .size_full()
            .min_h_0()
            .on_mouse_move(cx.listener(|this, _event, _window, _cx| {
                if this.detached {
                    this.last_activity = Instant::now();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                if !*hovered {
                    this.resync(cx);
                }
            }))
            .child(
                flex_col()
                    .id("lyrics-scroll")
                    .size_full()
                    .items_stretch()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(|this, _event, _window, _cx| this.detach()))
                    .px(px(variables.padding_16))
                    .pt(px(variables.padding_16))
                    .pb(px(variables.padding_16))
                    .children(rows),
            )
            .child(edge_fade(true, top_fade, &variables))
            .child(edge_fade(false, bottom_fade, &variables))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .child(Scrollbar::new(&self.scroll).axis(ScrollbarAxis::Vertical)),
            )
            .smooth_scroll(&self.scroll)
            .into_any_element()
    }
}
