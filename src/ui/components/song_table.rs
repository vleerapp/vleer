use gpui::{prelude::*, *};
use rustc_hash::FxHashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::data::types::{Cuid, Song};
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use crate::ui::components::div::{flex_col, flex_row};
use crate::ui::components::icons::icon::icon;
use crate::ui::components::icons::icons::{ARROW_DOWN, ARROW_UP, DURATION};
use crate::ui::components::scrollbar::{Scrollbar, ScrollbarAxis};
use crate::ui::state::State;
use crate::ui::variables::Variables;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnSize {
    Fixed(f32),
    Flex(),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongColumn {
    Number,
    Title,
    Album,
    Duration,
}

impl SongColumn {
    fn name(&self) -> &'static str {
        match self {
            SongColumn::Number => "#",
            SongColumn::Title => "Title",
            SongColumn::Album => "Album",
            SongColumn::Duration => "Duration",
        }
    }

    fn size(&self, number_width: f32, duration_width: f32) -> ColumnSize {
        match self {
            SongColumn::Number => ColumnSize::Fixed(number_width),
            SongColumn::Title => ColumnSize::Flex(),
            SongColumn::Album => ColumnSize::Flex(),
            SongColumn::Duration => ColumnSize::Fixed(duration_width),
        }
    }

    const ALL: [SongColumn; 4] = [
        SongColumn::Number,
        SongColumn::Title,
        SongColumn::Album,
        SongColumn::Duration,
    ];
}

#[derive(Clone, Copy, Debug)]
pub struct TableSort {
    pub column: SongColumn,
    pub ascending: bool,
}

#[derive(Clone)]
pub struct SongEntry {
    pub id: Cuid,
    pub number: usize,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: String,
    pub cover_uri: Option<String>,
}

impl SongEntry {
    fn get_column_value(&self, column: SongColumn) -> SharedString {
        match column {
            SongColumn::Number => self.number.to_string().into(),
            SongColumn::Title => self.title.clone().into(),
            SongColumn::Album => self.album.clone().into(),
            SongColumn::Duration => self.duration.clone().into(),
        }
    }
}

pub type OnSelectHandler = Rc<dyn Fn(&mut App, &Cuid) + 'static>;
pub type GetRowsHandler = Rc<dyn Fn(&mut App, Option<TableSort>) -> Vec<Cuid> + 'static>;
pub type GetRowHandler = Rc<dyn Fn(&mut App, Cuid) -> Option<Arc<SongEntry>> + 'static>;

type RowMap = FxHashMap<usize, Entity<SongTableItem>>;

fn prune_views(views: &Entity<RowMap>, render_counter: &Entity<usize>, idx: usize, cx: &mut App) {
    let counter = *render_counter.read(cx);
    if idx == 0 && counter > 0 {
        views.update(cx, |views, _| {
            views.retain(|&k, _| k < counter + 50);
        });
    }
    render_counter.update(cx, |c, _| *c = idx);
}

fn create_or_retrieve_view<F>(
    views: &Entity<RowMap>,
    idx: usize,
    create: F,
    cx: &mut App,
) -> Entity<SongTableItem>
where
    F: FnOnce(&mut App) -> Entity<SongTableItem>,
{
    if let Some(view) = views.read(cx).get(&idx) {
        return view.clone();
    }
    let view = create(cx);
    views.update(cx, |views, _| {
        views.insert(idx, view.clone());
    });
    view
}

#[derive(Clone)]
pub struct SongTableItem {
    data: Option<SongEntry>,
    on_select: Option<OnSelectHandler>,
    number_width: f32,
    duration_width: f32,
    row_number: usize,
    items: Option<Arc<Vec<Cuid>>>,
}

impl SongTableItem {
    pub fn new(
        cx: &mut App,
        id: Cuid,
        get_row: &GetRowHandler,
        on_select: Option<OnSelectHandler>,
        number_width: f32,
        duration_width: f32,
        row_number: usize,
        items: Option<Arc<Vec<Cuid>>>,
    ) -> Entity<Self> {
        let data = get_row(cx, id);

        cx.new(|_| Self {
            data: data.as_ref().map(|arc| (**arc).clone()),
            on_select,
            number_width,
            duration_width,
            row_number,
            items,
        })
    }
}

impl Render for SongTableItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let row_data = self.data.clone();
        let on_select = self.on_select.clone();
        let items = self.items.clone();

        let element_id = self
            .data
            .as_ref()
            .map(|d| ElementId::Name(format!("song-{}", d.number).into()))
            .unwrap_or_else(|| ElementId::Name("empty-row".into()));

        let mut row = flex_row()
            .w_full()
            .id(element_id)
            .items_center()
            .gap(px(variables.padding_8))
            .px(px(variables.padding_8))
            .pb(px(variables.padding_16))
            .when_some(on_select, move |div, handler| {
                let row_data = row_data.clone();
                div.on_click(move |_, _, cx| {
                    if let Some(data) = &row_data {
                        handler(cx, &data.id);
                    }
                })
                .cursor_pointer()
            });

        if let Some(data) = &self.data {
            for column in SongColumn::ALL {
                let size = column.size(self.number_width, self.duration_width);

                let mut column_div = div();

                match size {
                    ColumnSize::Fixed(width) => {
                        column_div = column_div.w(px(width)).flex_shrink_0();
                    }
                    ColumnSize::Flex() => {
                        column_div = column_div.flex_1().min_w_0();
                    }
                }

                if matches!(column, SongColumn::Title) {
                    let image_size = 36.0;
                    column_div = column_div
                        .flex()
                        .flex_row()
                        .gap(px(variables.padding_8))
                        .items_center()
                        .child(
                            div()
                                .id("cover")
                                .size(px(image_size))
                                .flex_shrink_0()
                                .bg(variables.element)
                                .when_some(data.cover_uri.clone(), |div, image| {
                                    div.child(
                                        img(image)
                                            .size(px(image_size))
                                            .object_fit(ObjectFit::Cover),
                                    )
                                })
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, {
                                    let items = items.clone();
                                    let row_number = self.row_number;
                                    move |_event, _window, cx| {
                                        if let Some(all_items) = &items {
                                            let items_clone = all_items.clone();
                                            let state = cx.global::<State>().clone();
                                            let start_index = row_number - 1;

                                            cx.spawn(move |cx: &mut gpui::AsyncApp| {
                                                let cx = cx.clone();
                                                async move {
                                                    let songs: Vec<Arc<Song>> = {
                                                        let mut result = Vec::new();
                                                        for cuid in
                                                            items_clone.iter().skip(start_index)
                                                        {
                                                            if let Some(song) =
                                                                state.get_song(cuid).await
                                                            {
                                                                result.push(song);
                                                            }
                                                        }
                                                        result
                                                    };

                                                    cx.update(|cx| {
                                                        cx.update_global::<Queue, _>(
                                                            |queue, _cx| {
                                                                queue.clear_and_queue_songs(&songs);
                                                            },
                                                        );

                                                        if let Err(e) =
                                                            Playback::play_queue(cx)
                                                        {
                                                            tracing::error!(
                                                                "Failed to start playback: {}",
                                                                e
                                                            );
                                                        }
                                                    })
                                                    .ok();
                                                }
                                            })
                                            .detach();
                                        }
                                    }
                                }),
                        )
                        .child(
                            flex_col()
                                .id("title-and-artist")
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_sm()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_weight(FontWeight(500.0))
                                        .hover(|this| this.underline())
                                        .child(data.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_color(variables.text_secondary)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(data.artist.clone()),
                                ),
                        );
                } else {
                    let value = if matches!(column, SongColumn::Number) {
                        self.row_number.to_string().into()
                    } else {
                        data.get_column_value(column)
                    };
                    column_div = column_div
                        .text_sm()
                        .text_color(variables.text_secondary)
                        .text_ellipsis()
                        .child(value);
                }

                row = row.child(column_div);
            }
        }

        row
    }
}

pub enum SongTableEvent {
    NewRows,
}

#[derive(Clone)]
pub struct SongTable {
    views: Entity<RowMap>,
    render_counter: Entity<usize>,
    items: Option<Arc<Vec<Cuid>>>,
    sort_method: Entity<Option<TableSort>>,
    on_select: Option<OnSelectHandler>,
    get_rows: GetRowsHandler,
    get_row: GetRowHandler,
    number_width: f32,
    duration_width: f32,
    scroll_handle: UniformListScrollHandle,
}

impl EventEmitter<SongTableEvent> for SongTable {}

fn calculate_column_widths(item_count: usize) -> (f32, f32) {
    const CHAR_WIDTH: f32 = 8.5;

    let digit_count = if item_count == 0 {
        1
    } else {
        (item_count as f32).log10().floor() as usize + 1
    };

    let number_width = digit_count as f32 * CHAR_WIDTH;
    let duration_width = 6.0 * CHAR_WIDTH;

    (number_width, duration_width)
}

impl SongTable {
    pub fn new(
        cx: &mut App,
        get_rows: GetRowsHandler,
        get_row: GetRowHandler,
        on_select: Option<OnSelectHandler>,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let views = cx.new(|_| FxHashMap::default());
            let render_counter = cx.new(|_| 0);
            let sort_method = cx.new(|_| None);

            let items = Some(Arc::new(get_rows(cx, None)));
            let (number_width, duration_width) =
                calculate_column_widths(items.as_ref().map(|i| i.len()).unwrap_or(0));

            let get_rows_clone = get_rows.clone();
            cx.observe(&sort_method, move |this: &mut SongTable, sort, cx| {
                let sort_method = *sort.read(cx);
                let items = Some(Arc::new((this.get_rows)(cx, sort_method)));
                let (number_width, duration_width) =
                    calculate_column_widths(items.as_ref().map(|i| i.len()).unwrap_or(0));

                this.views = cx.new(|_| FxHashMap::default());
                this.render_counter = cx.new(|_| 0);
                this.items = items;
                this.number_width = number_width;
                this.duration_width = duration_width;

                cx.notify();
            })
            .detach();

            let get_rows_for_event = get_rows_clone;
            cx.subscribe(&cx.entity(), move |this, _, event, cx| match event {
                SongTableEvent::NewRows => {
                    let sort_method = *this.sort_method.read(cx);
                    let items = Some(Arc::new((get_rows_for_event)(cx, sort_method)));
                    let (number_width, duration_width) =
                        calculate_column_widths(items.as_ref().map(|i| i.len()).unwrap_or(0));

                    this.views = cx.new(|_| FxHashMap::default());
                    this.render_counter = cx.new(|_| 0);
                    this.items = items;
                    this.number_width = number_width;
                    this.duration_width = duration_width;

                    cx.notify();
                }
            })
            .detach();

            Self {
                views,
                render_counter,
                items,
                sort_method,
                on_select,
                get_rows,
                get_row,
                number_width,
                duration_width,
                scroll_handle: UniformListScrollHandle::default(),
            }
        })
    }
}

impl Render for SongTable {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let sort_method = self.sort_method.read(cx).clone();
        let items = self.items.clone();
        let views_model = self.views.clone();
        let render_counter = self.render_counter.clone();
        let handler = self.on_select.clone();
        let get_row = self.get_row.clone();
        let number_width = self.number_width;
        let duration_width = self.duration_width;

        let mut header = flex_row()
            .w_full()
            .gap(px(variables.padding_8))
            .px(px(variables.padding_8))
            .pb(px(variables.padding_8))
            .text_color(variables.text_secondary)
            .border_b_1()
            .border_color(variables.element);

        for (i, column) in SongColumn::ALL.iter().enumerate() {
            let column_id = *column;
            let size = column.size(number_width, duration_width);
            let is_sortable = !matches!(column_id, SongColumn::Number);

            let mut header_col = flex_row()
                .id(ElementId::Name(format!("header-col-{}", i).into()))
                .gap(px(variables.padding_8))
                .items_center()
                .when(is_sortable, |div| div.cursor_pointer());

            match size {
                ColumnSize::Fixed(width) => {
                    header_col = header_col.w(px(width)).flex_shrink_0();
                }
                ColumnSize::Flex() => {
                    header_col = header_col.flex_1().min_w_0();
                }
            }

            let is_sorted = is_sortable
                && sort_method
                    .as_ref()
                    .map_or(false, |m| m.column == column_id);

            header_col = header_col.when(is_sorted, |div| div.text_color(variables.text));

            if matches!(column_id, SongColumn::Duration) {
                header_col = header_col
                    .child(icon(DURATION).when(is_sorted, |i| i.text_color(variables.text)));
            } else {
                header_col = header_col.child(SharedString::new_static(column_id.name()));
            }

            header_col = header_col.when(is_sortable, |this| {
                this.when_some(sort_method.as_ref(), |this, method| {
                    this.when(method.column == column_id, |this| {
                        let arrow_icon = if method.ascending {
                            icon(ARROW_UP)
                        } else {
                            icon(ARROW_DOWN)
                        };
                        this.child(arrow_icon.text_color(variables.text))
                    })
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sort_method.update(cx, move |this, cx| {
                        if let Some(method) = this.as_mut() {
                            if method.column == column_id {
                                if method.ascending {
                                    method.ascending = false;
                                } else {
                                    *this = None;
                                }
                            } else {
                                *this = Some(TableSort {
                                    column: column_id,
                                    ascending: true,
                                });
                            }
                        } else {
                            *this = Some(TableSort {
                                column: column_id,
                                ascending: true,
                            });
                        }
                        cx.notify();
                    })
                }))
            });

            header = header.child(header_col);
        }

        div().h_full().w_full().flex_col().child(header).child(
            div()
                .flex_1()
                .size_full()
                .min_h_0()
                .id("song-table-view")
                .gap(px(variables.padding_16))
                .pt(px(variables.padding_16))
                .when_some(items, |this, items| {
                    this.child(
                        div()
                            .relative()
                            .size_full()
                            .child(
                                uniform_list(
                                    ElementId::Name("song-table-list".into()),
                                    items.len(),
                                    move |range, _, cx| {
                                        let start = range.start;
                                        items[range]
                                            .iter()
                                            .enumerate()
                                            .map(|(idx, item)| {
                                                let idx = idx + start;
                                                prune_views(&views_model, &render_counter, idx, cx);
                                                let get_row_clone = get_row.clone();
                                                let items_clone = items.clone();
                                                create_or_retrieve_view(
                                                    &views_model,
                                                    idx,
                                                    |cx| {
                                                        SongTableItem::new(
                                                            cx,
                                                            item.clone(),
                                                            &get_row_clone,
                                                            handler.clone(),
                                                            number_width,
                                                            duration_width,
                                                            idx + 1,
                                                            Some(items_clone),
                                                        )
                                                    },
                                                    cx,
                                                )
                                                .into_any_element()
                                            })
                                            .collect()
                                    },
                                )
                                .track_scroll(&self.scroll_handle.clone())
                                .size_full(),
                            )
                            .child(
                                Scrollbar::new(&self.scroll_handle).axis(ScrollbarAxis::Vertical),
                            ),
                    )
                }),
        )
    }
}
