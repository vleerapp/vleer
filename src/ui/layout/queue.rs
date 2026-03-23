use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::data::db::repo::Database;
use crate::data::models::{Cuid, Song};
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use crate::ui::components::context_menu::QueueChanged;
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::icon::icon;
use crate::ui::components::icons::icons::{PLAY, X};
use crate::ui::components::scrollbar::{Scrollbar, ScrollbarAxis};
use crate::ui::variables::Variables;

const ANIMATION_FPS: f32 = 15.0;
const ROW_HEIGHT: f32 = 36.0;
const QUEUE_WIDTH: f32 = 300.0;

#[derive(Clone, Default)]
pub struct QueueVisible(pub bool);

impl Global for QueueVisible {}

#[derive(Clone)]
struct QueueDragPayload {
    from_index: usize,
    song: Song,
    position: Point<Pixels>,
}

impl QueueDragPayload {
    fn with_position(mut self, pos: Point<Pixels>) -> Self {
        self.position = pos;
        self
    }
}

impl Render for QueueDragPayload {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let song = self.song.clone();
        let pos = self.position;

        let is_current = cx
            .global::<Queue>()
            .get_current_song_id()
            .map(|id| id == song.id)
            .unwrap_or(false);

        let cover = if let Some(ref uri) = song.image_id {
            img(format!("!image://{}", uri))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element()
        } else {
            div().bg(variables.border).size_full().into_any_element()
        };

        let drag_w = QUEUE_WIDTH - variables.padding_16 * 2.0;

        div()
            .font_family("Feature Mono")
            .text_size(px(14.0))
            .line_height(px(14.0))
            .pl(pos.x - px(ROW_HEIGHT / 2.0))
            .pt(pos.y - px(ROW_HEIGHT / 2.0))
            .child(
                flex_row()
                    .w(px(drag_w))
                    .bg(variables.element)
                    .gap(px(variables.padding_8))
                    .pr(px(variables.padding_8))
                    .shadow_md()
                    .child(
                        div()
                            .size(px(ROW_HEIGHT))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .child(cover),
                    )
                    .child(
                        div()
                            .overflow_x_hidden()
                            .text_ellipsis()
                            .font_weight(FontWeight(500.0))
                            .text_color(if is_current {
                                variables.text
                            } else {
                                variables.text_secondary
                            })
                            .child(song.title.clone()),
                    )
                    .child(div().flex_1()),
            )
    }
}

pub struct QueuePane {
    songs: Vec<Song>,
    drag_from: Option<usize>,
    drag_over: Option<usize>,
    is_animating: bool,
    scroll_handle: UniformListScrollHandle,
    last_current_song_id: Option<Cuid>,
}

impl QueuePane {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<QueueChanged>(|this, cx| {
            this.drag_from = None;
            this.drag_over = None;
            this.reload_songs(cx);
        })
        .detach();

        let mut pane = Self {
            songs: Vec::new(),
            drag_from: None,
            drag_over: None,
            is_animating: false,
            scroll_handle: UniformListScrollHandle::default(),
            last_current_song_id: None,
        };
        pane.reload_songs(cx);
        pane
    }

    fn ensure_animation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_animating {
            return;
        }
        self.is_animating = true;
        cx.spawn_in(
            window,
            |view: WeakEntity<QueuePane>, cx: &mut AsyncWindowContext| {
                let mut cx = cx.clone();
                async move {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs_f32(1.0 / ANIMATION_FPS))
                            .await;

                        let should_continue = view.update(&mut cx, |this, cx| {
                            let has_playing = this.songs.iter().any(|s| {
                                cx.global::<Queue>()
                                    .get_current_song_id()
                                    .map(|id| id == s.id)
                                    .unwrap_or(false)
                                    && cx.global::<Playback>().get_playing()
                            });
                            if has_playing {
                                cx.notify();
                                true
                            } else {
                                this.is_animating = false;
                                false
                            }
                        });

                        if should_continue.is_err() || !should_continue.unwrap() {
                            break;
                        }
                    }
                }
            },
        )
        .detach();
    }

    fn reload_songs(&mut self, cx: &mut Context<Self>) {
        let items: Vec<Cuid> = cx.global::<Queue>().get_items();
        let db = cx.global::<Database>().clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let fetched = db.get_songs_by_ids(&items).await.unwrap_or_default();
            let id_to_song: std::collections::HashMap<_, _> =
                fetched.into_iter().map(|s| (s.id.clone(), s)).collect();
            let songs: Vec<Song> = items
                .iter()
                .filter_map(|id| id_to_song.get(id).cloned())
                .collect();
            cx.update(|cx| {
                this.update(cx, |pane, cx| {
                    pane.songs = songs;
                    cx.notify();
                })
            })
            .ok();
        })
        .detach();
    }
}

fn build_display_order(len: usize, from: usize, to: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    let item = order.remove(from);
    order.insert(to, item);
    order
}

fn render_row(
    view: &Entity<QueuePane>,
    display_idx: usize,
    real_idx: usize,
    song: &Song,
    is_current: bool,
    is_playing: bool,
    spectrum: [f32; 4],
    variables: &Variables,
) -> impl IntoElement {
    let song = song.clone();
    let variables = *variables;

    let cover = if let Some(ref uri) = song.image_id {
        img(format!("!image://{}", uri))
            .size_full()
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    } else {
        div().bg(variables.border).size_full().into_any_element()
    };

    let drag_payload = QueueDragPayload {
        from_index: real_idx,
        song: song.clone(),
        position: Point::default(),
    };

    let drag_view = view.clone();
    let drop_view = view.clone();

    div().pb(px(variables.padding_8)).child(
        flex_row()
            .w_full()
            .id(ElementId::Name(
                format!("queue-item-{}", display_idx).into(),
            ))
            .group("queue-item")
            .bg(variables.element)
            .hover(|s| s.bg(variables.element_hover))
            .gap(px(variables.padding_8))
            .pr(px(variables.padding_8))
            .cursor_move()
            .on_drag(
                drag_payload,
                move |payload: &QueueDragPayload, pos, _window, cx| {
                    cx.new(|_| payload.clone().with_position(pos))
                },
            )
            .on_drag_move(move |e: &DragMoveEvent<QueueDragPayload>, _window, cx| {
                if !e.bounds.contains(&e.event.position) {
                    return;
                }
                drag_view.update(cx, |this, cx| {
                    if this.drag_over != Some(display_idx) {
                        this.drag_from = Some(e.drag(cx).from_index);
                        this.drag_over = Some(display_idx);
                        cx.notify();
                    }
                });
            })
            .on_drop(move |payload: &QueueDragPayload, _window, cx| {
                let from = payload.from_index;
                drop_view.update(cx, |this, cx| {
                    this.drag_from = None;
                    this.drag_over = None;
                    if from != display_idx {
                        cx.update_global::<Queue, _>(|q, _| {
                            q.move_song(from, display_idx);
                        });
                        cx.set_global(QueueChanged::default());
                    } else {
                        cx.notify();
                    }
                });
            })
            .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                if event.click_count == 2 {
                    cx.update_global::<Queue, _>(|q, cx| {
                        q.set_current_index(real_idx, cx);
                    });
                    cx.update_global::<Playback, _>(|p, cx| {
                        p.play_queue(cx);
                    });
                    cx.set_global(QueueChanged::default());
                }
            })
            .child(
                div()
                    .size(px(ROW_HEIGHT))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .relative()
                    .group("cover-container")
                    .child(cover)
                    .child(
                        flex_row()
                            .absolute()
                            .inset_0()
                            .gap(px(3.0))
                            .p(px(5.0))
                            .bg(black().opacity(0.5))
                            .when(!is_playing, |s| s.invisible())
                            .group_hover("cover-container", |s| s.invisible())
                            .children((0..4).map(move |i| {
                                let height_pct = (spectrum[i] * 100.0).clamp(10.0, 80.0);
                                let height_px = ROW_HEIGHT * (height_pct / 100.0);
                                div().w(px(4.0)).h(px(height_px)).bg(variables.text)
                            })),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(black().opacity(0.5))
                            .invisible()
                            .group_hover("cover-container", |s| s.visible())
                            .child(icon(PLAY).size(px(16.0)).text_color(variables.text)),
                    ),
            )
            .child(
                div()
                    .overflow_x_hidden()
                    .text_ellipsis()
                    .font_weight(FontWeight(500.0))
                    .text_color(if is_current {
                        variables.text
                    } else {
                        variables.text_secondary
                    })
                    .child(song.title.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .invisible()
                    .group_hover("queue-item", |s| s.visible())
                    .flex_shrink_0()
                    .child(
                        div()
                            .p(px(4.0))
                            .hover(|s| s.bg(variables.element_hover))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                                cx.update_global::<Queue, _>(|q, _| {
                                    q.remove_at(real_idx);
                                });
                                cx.set_global(QueueChanged::default());
                            })
                            .child(icon(X).size(px(14.0)).text_color(variables.text_secondary)),
                    ),
            ),
    )
}

fn render_drop_slot(
    view: &Entity<QueuePane>,
    display_idx: usize,
    variables: &Variables,
) -> impl IntoElement {
    let slot_view = view.clone();
    let drop_view = view.clone();
    let variables = *variables;

    div()
        .id(ElementId::Name(
            format!("queue-slot-{}", display_idx).into(),
        ))
        .pb(px(variables.padding_8))
        .child(div().w_full().h(px(ROW_HEIGHT)))
        .on_drag_move(move |e: &DragMoveEvent<QueueDragPayload>, _window, cx| {
            if !e.bounds.contains(&e.event.position) {
                return;
            }
            slot_view.update(cx, |this, cx| {
                if this.drag_over != Some(display_idx) {
                    this.drag_from = Some(e.drag(cx).from_index);
                    this.drag_over = Some(display_idx);
                    cx.notify();
                }
            });
        })
        .on_drop(move |payload: &QueueDragPayload, _window, cx| {
            let from = payload.from_index;
            drop_view.update(cx, |this, cx| {
                this.drag_from = None;
                this.drag_over = None;
                if from != display_idx {
                    cx.update_global::<Queue, _>(|q, _| {
                        q.move_song(from, display_idx);
                    });
                    cx.set_global(QueueChanged::default());
                } else {
                    cx.notify();
                }
            });
        })
}

impl Render for QueuePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let is_empty = self.songs.is_empty();

        let current_song_id = cx.global::<Queue>().get_current_song_id();

        if current_song_id != self.last_current_song_id {
            self.last_current_song_id = current_song_id.clone();
            if let Some(ref song_id) = current_song_id {
                if let Some(display_idx) = self.songs.iter().position(|s| &s.id == song_id) {
                    self.scroll_handle
                        .scroll_to_item_strict(display_idx, ScrollStrategy::Top);
                }
            }
        }

        let is_globally_playing = cx.global::<Playback>().get_playing();
        let spectrum = if is_globally_playing {
            cx.global::<Playback>().get_spectrum()
        } else {
            [0.1, 0.1, 0.1, 0.1]
        };

        let any_playing = self.songs.iter().any(|s| {
            current_song_id
                .as_ref()
                .map(|id| id == &s.id)
                .unwrap_or(false)
                && is_globally_playing
        });
        if any_playing {
            self.ensure_animation(window, cx);
        }

        let drag_from = self.drag_from;
        let drag_over = self.drag_over;
        let songs = self.songs.clone();
        let row_count = songs.len();
        let view_handle = cx.entity();

        let display_order: Vec<usize> = if let (Some(from), Some(over)) = (drag_from, drag_over) {
            if from < row_count && over < row_count {
                build_display_order(row_count, from, over)
            } else {
                (0..row_count).collect()
            }
        } else {
            (0..row_count).collect()
        };

        let reset_view = cx.entity();
        let reset_view_out = cx.entity();

        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                reset_view.update(cx, |this: &mut QueuePane, cx| {
                    if this.drag_from.is_some() {
                        this.drag_from = None;
                        this.drag_over = None;
                        cx.notify();
                    }
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                reset_view_out.update(cx, |this: &mut QueuePane, cx| {
                    if this.drag_from.is_some() {
                        this.drag_from = None;
                        this.drag_over = None;
                        cx.notify();
                    }
                });
            })
            .when(is_empty, |this| {
                this.child(
                    flex_col()
                        .size_full()
                        .p(px(variables.padding_16))
                        .text_color(variables.text_secondary)
                        .child("Queue is empty"),
                )
            })
            .when(!is_empty, |this| {
                let scroll_handle = self.scroll_handle.clone();
                this.child(
                    div()
                        .size_full()
                        .min_h_0()
                        .relative()
                        .child(
                            div()
                                .size_full()
                                .p(px(variables.padding_16))
                                .pb(px(0.0))
                                .child(
                                    uniform_list(
                                        ElementId::Name("queue-list".into()),
                                        row_count,
                                        move |range, _window, _cx| {
                                            range
                                                .map(|display_idx| {
                                                    let real_idx = display_order[display_idx];
                                                    let song = &songs[real_idx];
                                                    let is_current = current_song_id
                                                        .as_ref()
                                                        .map(|id| id == &song.id)
                                                        .unwrap_or(false);
                                                    let is_playing =
                                                        is_current && is_globally_playing;

                                                    let is_slot = drag_from == Some(real_idx);

                                                    if is_slot {
                                                        render_drop_slot(
                                                            &view_handle,
                                                            display_idx,
                                                            &variables,
                                                        )
                                                        .into_any_element()
                                                    } else {
                                                        render_row(
                                                            &view_handle,
                                                            display_idx,
                                                            real_idx,
                                                            song,
                                                            is_current,
                                                            is_playing,
                                                            spectrum,
                                                            &variables,
                                                        )
                                                        .into_any_element()
                                                    }
                                                })
                                                .collect()
                                        },
                                    )
                                    .track_scroll(&scroll_handle)
                                    .size_full(),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .bottom_0()
                                .left_0()
                                .child(
                                    Scrollbar::new(&self.scroll_handle)
                                        .axis(ScrollbarAxis::Vertical),
                                ),
                        ),
                )
            })
    }
}
