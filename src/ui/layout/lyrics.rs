use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::{Duration, Instant};

use crate::{
    data::{
        db::repo::Database,
        lyrics::{Line, Lyrics, active_line, active_line_by, is_gap, resolve},
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

const TICK: Duration = Duration::from_millis(40);
const LEAD_MIN: f32 = 0.25;
const LEAD_MAX: f32 = 0.6;
const EASE: f32 = 0.08;
const CLEAR_SECS: f32 = 0.3;
const HERTZ: f32 = 180.0;
const MAX_FRAME_TIME: Duration = Duration::from_millis(64);
const REST: Pixels = px(0.5);

const LINE_SIZE: f32 = 20.0;
const LINE_HEIGHT: f32 = 30.0;
const WORD_DIM: f32 = 0.5;
const NEXT_OPACITY: f32 = 0.55;
const SWEEP_FEATHER: f32 = 0.35;
const SWEEP_LAYERS: usize = 4;
const SECONDARY_SIZE: f32 = 14.0;
const SECONDARY_HEIGHT: f32 = 20.0;
const PLAIN_SIZE: f32 = 16.0;
const PLAIN_HEIGHT: f32 = 26.0;
const GAP_HEIGHT: f32 = 34.0;
const DOT_SIZE: f32 = 9.0;
const IDLE_RESYNC: Duration = Duration::from_secs(3);
const FADE_TOP: f32 = 16.0;
const FADE_BOTTOM: f32 = 72.0;
const BLUR: f32 = 0.2;
const HAZE: f32 = 0.7;
const PIN: f32 = 0.3;
const NEIGHBOR: Pixels = px(LINE_HEIGHT * 2.0);
const HAZE_LEAST: Pixels = px(0.1);

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
    led: Option<usize>,
    follow: Option<usize>,
    snap: bool,
    target: Option<Pixels>,
    last_frame: Option<Instant>,
    shown: Vec<f32>,
    clear: Vec<f32>,
    fade_frame: Option<Instant>,
    detached: bool,
    hovered: Option<usize>,
    hover_released: bool,
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
            led: None,
            follow: None,
            snap: true,
            target: None,
            last_frame: None,
            shown: Vec::new(),
            clear: Vec::new(),
            fade_frame: None,
            detached: false,
            hovered: None,
            hover_released: false,
            last_activity: Instant::now(),
            last_offset: Pixels::ZERO,
            scroll: ScrollHandle::new(),
            task: None,
        }
    }

    fn hover(&self) -> Option<usize> {
        if self.hover_released {
            None
        } else {
            self.hovered
        }
    }

    fn is_visible(cx: &App) -> bool {
        cx.try_global::<SidePanel>().copied() == Some(SidePanel::Lyrics)
    }

    fn reset_motion(&mut self) {
        self.active = None;
        self.led = None;
        self.follow = None;
        self.target = None;
        self.snap = true;
        self.last_frame = None;
        self.shown.clear();
        self.clear.clear();
        self.fade_frame = None;
        self.detached = false;
        self.hovered = None;
        self.hover_released = false;
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
        if self.last_activity.elapsed() >= IDLE_RESYNC {
            self.resync(cx);
            if self.hovered.is_some() && !self.hover_released {
                self.hover_released = true;
                cx.notify();
            }
        }
        let Load::Ready(Lyrics::Synced(lines)) = &self.load else {
            return;
        };
        let position = cx.global::<Playback>().get_position();
        let led = active_line_by(lines, position, |i| lines[i].at - lead(lines, i));
        let active = active_line(lines, position);
        if led != self.led {
            self.led = led;
            cx.notify();
        }
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

    fn step_fade(&mut self, lit: &[bool], active: Option<usize>, window: &mut Window) {
        let count = lit.len();
        let focus = self.hover().or(active);
        if self.shown.len() != count {
            self.shown = (0..count).map(|i| line_opacity(i, lit[i], focus)).collect();
            self.clear = (0..count).map(|i| clarity(i, lit[i], focus)).collect();
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
            let target = line_opacity(i, lit[i], focus);
            let distance = target - *shown;
            if distance.abs() < 0.005 {
                *shown = target;
            } else {
                *shown += distance * ease;
                moving = true;
            }
        }
        let step = elapsed.min(MAX_FRAME_TIME).as_secs_f32() / CLEAR_SECS;
        for (i, clear) in self.clear.iter_mut().enumerate() {
            let target = clarity(i, lit[i], focus);
            let distance = target - *clear;
            if distance.abs() <= step {
                *clear = target;
            } else {
                *clear += step * distance.signum();
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

fn lead(lines: &[Line], index: usize) -> f32 {
    let line = &lines[index];
    let previous_end = index
        .checked_sub(1)
        .map_or(line.at, |previous| lines[previous].end.unwrap_or(line.at));
    (line.at - previous_end).clamp(LEAD_MIN, LEAD_MAX)
}

fn is_singing(lines: &[Line], index: usize, position: f32) -> bool {
    let line = &lines[index];
    let end = line
        .end
        .or(lines.get(index + 1).map(|next| next.at))
        .unwrap_or(f32::MAX);
    position >= line.at && position < end
}

fn clarity(index: usize, lit: bool, focus: Option<usize>) -> f32 {
    if lit || focus == Some(index) {
        1.0
    } else {
        0.0
    }
}

fn line_opacity(index: usize, lit: bool, focus: Option<usize>) -> f32 {
    if lit {
        return 1.0;
    }
    match focus {
        None => 0.5,
        Some(focus) => match index.abs_diff(focus) {
            0 => 1.0,
            distance => (0.55 - 0.1 * (distance - 1) as f32).max(0.12),
        },
    }
}

fn syllable(text: String, sweep: Option<(f32, f32)>, variables: &Variables) -> AnyElement {
    let base = div().whitespace_nowrap();
    let Some((progress, opacity)) = sweep else {
        return base.child(text).into_any_element();
    };
    let color = rgb_to_hsla(variables.text);
    let engaged = ((opacity - NEXT_OPACITY) / (1.0 - NEXT_OPACITY)).clamp(0.0, 1.0);
    let dim = color.opacity(opacity * (1.0 - (1.0 - WORD_DIM) * engaged));
    let lit = color.opacity(opacity);
    if progress <= 0.0 {
        return base.text_color(dim).child(text).into_any_element();
    }
    if progress >= 1.0 {
        return base.text_color(lit).child(text).into_any_element();
    }
    let edge = progress * (1.0 + SWEEP_FEATHER);
    let mut cell = base.relative().text_color(dim).child(text.clone());
    for k in (1..=SWEEP_LAYERS).rev() {
        let reach = (edge - SWEEP_FEATHER * (SWEEP_LAYERS - k) as f32 / SWEEP_LAYERS as f32)
            .clamp(0.0, 1.0);
        if reach <= 0.0 {
            continue;
        }
        cell = cell.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(reach))
                .overflow_hidden()
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_color(color.opacity(opacity / k as f32))
                        .child(text.clone()),
                ),
        );
    }
    cell.into_any_element()
}

fn word_row(
    line: &Line,
    next_at: Option<f32>,
    position: f32,
    sweep_opacity: Option<f32>,
    variables: &Variables,
) -> AnyElement {
    let line_end = line
        .end
        .or(next_at)
        .unwrap_or_else(|| line.words.last().map_or(line.at, |w| w.at + 1.0));
    let mut groups: Vec<Vec<AnyElement>> = vec![Vec::new()];
    for (i, word) in line.words.iter().enumerate() {
        let end = line.words.get(i + 1).map_or(line_end, |next| next.at);
        let sweep = sweep_opacity.map(|opacity| {
            let progress = (position - word.at) / (end - word.at).max(0.001);
            (progress.clamp(0.0, 1.0), opacity)
        });
        if word.text.starts_with(char::is_whitespace)
            && groups.last().is_some_and(|group| !group.is_empty())
        {
            groups.push(Vec::new());
        }
        let group = groups.last_mut().expect("groups is never empty");
        group.push(syllable(word.text.clone(), sweep, variables));
        if word.text.ends_with(char::is_whitespace) {
            groups.push(Vec::new());
        }
    }
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .children(
            groups
                .into_iter()
                .filter(|group| !group.is_empty())
                .map(|group| div().flex().flex_row().flex_shrink_0().children(group)),
        )
        .into_any_element()
}

fn gap_row(progress: f32, opacity: f32, variables: &Variables) -> Div {
    flex_row()
        .h(px(GAP_HEIGHT))
        .gap(px(8.0))
        .items_center()
        .children((0..3).map(|i| {
            let lit = (progress * 3.0 - i as f32).clamp(0.0, 1.0);
            div()
                .size(px(DOT_SIZE))
                .rounded_full()
                .bg(rgb_to_hsla(variables.text).opacity(opacity * (0.25 + 0.75 * lit)))
        }))
}

fn row_top(scroll: &ScrollHandle, index: usize) -> Option<Pixels> {
    let item = scroll.bounds_for_item(index)?;
    Some(item.origin.y - scroll.bounds().origin.y + scroll.offset().y)
}

fn viewport_haze(scroll: &ScrollHandle, index: usize, pin: Pixels, margin: Pixels) -> f32 {
    let height = scroll.bounds().size.height;
    if height <= Pixels::ZERO {
        return 0.0;
    }
    let Some(top) = row_top(scroll, index) else {
        return 0.0;
    };
    let row_height = scroll
        .bounds_for_item(index)
        .map_or(Pixels::ZERO, |item| item.size.height);
    if top + row_height + margin < Pixels::ZERO || top - margin > height {
        return 0.0;
    }
    let travel = top - pin;
    let travel = if travel >= Pixels::ZERO {
        travel.max(NEIGHBOR)
    } else {
        travel.min(-NEIGHBOR)
    };
    let reach = if travel >= Pixels::ZERO {
        height - pin
    } else {
        pin.max(height * PIN)
    };
    (travel / reach.max(px(1.0)))
        .clamp(-1.0, 1.0)
        .abs()
        .powf(HAZE)
}

fn soften(body: Div, radius: Pixels) -> AnyElement {
    if radius > HAZE_LEAST {
        body.blur(radius).into_any_element()
    } else {
        body.into_any_element()
    }
}

fn synced_rows(
    lines: &[Line],
    shown: &[f32],
    clear: &[f32],
    scroll: &ScrollHandle,
    hovered: Option<usize>,
    active: Option<usize>,
    led: Option<usize>,
    position: f32,
    variables: &Variables,
    cx: &mut Context<LyricsPane>,
) -> Vec<AnyElement> {
    let variables = *variables;
    let blur = px(LINE_SIZE * BLUR);
    let focus = hovered.or(active);
    let pin = focus
        .and_then(|active| row_top(scroll, active))
        .unwrap_or(px(0.0))
        .clamp(px(0.0), scroll.bounds().size.height);
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let at = line.at;
            let opacity = shown.get(i).copied().unwrap_or(0.5);
            let sung = active == Some(i) || led == Some(i) || is_singing(lines, i, position);
            let clear = clear.get(i).copied().unwrap_or(0.0);
            let radius = if clear >= 1.0 {
                px(0.0)
            } else {
                let eased = clear * clear * (3.0 - 2.0 * clear);
                blur * viewport_haze(scroll, i, pin, blur) * (1.0 - eased)
            };
            if line.text.is_empty() {
                if !is_gap(lines, i) {
                    return div().id(("lyric-line", i)).h(px(10.0)).into_any_element();
                }
                let progress = if active == Some(i) {
                    let span = (lines[i + 1].at - line.at).max(f32::EPSILON);
                    ((position - line.at) / span).clamp(0.0, 1.0)
                } else if active.is_some_and(|active| active > i) {
                    1.0
                } else {
                    0.0
                };
                return div()
                    .id(("lyric-line", i))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, over: &bool, _window, cx| {
                        if *over && this.hovered != Some(i) {
                            this.hovered = Some(i);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |_this, _event, window, cx| {
                        cx.update_global::<Playback, _>(|playback, _cx| {
                            if let Err(e) = playback.seek(at) {
                                tracing::error!("Failed to seek: {}", e);
                            }
                        });
                        window.refresh();
                    }))
                    .child(soften(gap_row(progress, opacity, &variables), radius))
                    .into_any_element();
            }

            let next_at = lines.get(i + 1).map(|next| next.at);
            let body = {
                let main_text = if line.words.is_empty() || !sung {
                    line.text.clone().into_any_element()
                } else {
                    word_row(line, next_at, position, sung.then_some(opacity), &variables)
                };
                div()
                    .py(px(6.0))
                    .text_size(px(LINE_SIZE))
                    .line_height(px(LINE_HEIGHT))
                    .font_weight(FontWeight(600.0))
                    .child(flex_col().gap(px(2.0)).child(main_text).when_some(
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
            };
            div()
                .id(("lyric-line", i))
                .cursor_pointer()
                .text_color(rgb_to_hsla(variables.text).opacity(opacity))
                .when(hovered.is_some(), |this| {
                    this.hover(|s| s.text_color(variables.text))
                })
                .on_hover(cx.listener(move |this, over: &bool, _window, cx| {
                    if *over && this.hovered != Some(i) {
                        this.hovered = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |_this, _event, window, cx| {
                    cx.update_global::<Playback, _>(|playback, _cx| {
                        if let Err(e) = playback.seek(at) {
                            tracing::error!("Failed to seek: {}", e);
                        }
                    });
                    window.refresh();
                }))
                .child(soften(body, radius))
                .into_any_element()
        })
        .collect()
}

fn edge_fade(top: bool, strength: f32, variables: &Variables) -> Div {
    let solid = rgb_to_hsla(variables.background);
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

        let position = cx.global::<Playback>().get_position();
        self.drive_motion(window, px(variables.padding_16));
        if let Load::Ready(Lyrics::Synced(lines)) = &self.load {
            let lit: Vec<bool> = (0..lines.len())
                .map(|i| {
                    self.active == Some(i) || self.led == Some(i) || is_singing(lines, i, position)
                })
                .collect();
            self.step_fade(&lit, self.active, window);
        }

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
                if playing
                    && active.is_some_and(|i| {
                        is_gap(lines, i)
                            || !lines[i].words.is_empty()
                            || i > 0 && !lines[i - 1].words.is_empty()
                    })
                {
                    window.request_animation_frame();
                }
                synced_rows(
                    lines,
                    &self.shown,
                    &self.clear,
                    &self.scroll,
                    self.hover(),
                    active,
                    self.led,
                    position,
                    &variables,
                    cx,
                )
            }
            _ => Vec::new(),
        };

        div()
            .id("lyrics-view")
            .relative()
            .size_full()
            .min_h_0()
            .on_mouse_move(cx.listener(|this, _event, _window, cx| {
                this.last_activity = Instant::now();
                if this.hover_released {
                    this.hover_released = false;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                if !*hovered {
                    this.resync(cx);
                    if this.hovered.take().is_some() {
                        cx.notify();
                    }
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
