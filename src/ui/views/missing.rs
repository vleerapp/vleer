use gpui::{prelude::FluentBuilder, *};
use rustc_hash::FxHashSet;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tracing::error;

use crate::{
    data::{
        db::Database,
        models::{Cuid, LibraryFile},
        scanner::{Scanner, expand_tilde, forget_missing, missing_paths},
    },
    ui::{
        assets::{ImageRequest, bucket_size, cover_uri},
        components::{
            button::{Button, ButtonVariant},
            context_menu::{LibraryDataChanged, QueueChanged},
            div::{flex_col, flex_row},
            scrollbar::{Scrollbar, ScrollbarAxis},
            scroller::SmoothScrollable,
        },
        layout::library::Search,
        variables::Variables,
        views::{ActiveView, AppView, SelectedSource},
    },
};

const COVER_SIZE: f32 = 36.0;
const ROW_PITCH: f32 = COVER_SIZE + 16.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grouping {
    None,
    Album,
    Artist,
    Folder,
}

impl Grouping {
    const ALL: [Self; 4] = [Self::None, Self::Album, Self::Artist, Self::Folder];

    fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Album => "Album",
            Self::Artist => "Artist",
            Self::Folder => "Folder",
        }
    }
}

enum Item {
    Header { name: String, ids: Arc<[usize]> },
    Row(usize),
}

pub struct MissingSongsView {
    source: Option<String>,
    root: String,
    files: Vec<LibraryFile>,
    selected: FxHashSet<Cuid>,
    grouping: Grouping,
    query: String,
    items: Vec<Item>,
    visible: Vec<usize>,
    scroll_handle: UniformListScrollHandle,
    load_task: Option<Task<()>>,
    remove_task: Option<Task<()>>,
}

fn relative_path<'a>(file: &'a LibraryFile, root: &str) -> &'a str {
    let sep = std::path::MAIN_SEPARATOR;
    file.file_path
        .strip_prefix(root.trim_end_matches(sep))
        .unwrap_or(&file.file_path)
        .trim_start_matches(sep)
}

fn relative_dir(file: &LibraryFile, root: &str) -> String {
    match relative_path(file, root).rsplit_once(std::path::MAIN_SEPARATOR) {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

impl MissingSongsView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            source: cx.global::<SelectedSource>().0.clone(),
            root: String::new(),
            files: Vec::new(),
            selected: FxHashSet::default(),
            grouping: Grouping::None,
            query: cx.global::<Search>().query.trim().to_lowercase(),
            items: Vec::new(),
            visible: Vec::new(),
            scroll_handle: UniformListScrollHandle::default(),
            load_task: None,
            remove_task: None,
        };

        cx.observe_global::<SelectedSource>(|this, cx| {
            let source = cx.global::<SelectedSource>().0.clone();
            if source != this.source {
                this.source = source;
                this.selected.clear();
                this.files.clear();
                this.rebuild();
            }
            if cx.global::<ActiveView>().0 == AppView::MissingSongs {
                this.reload(cx);
            }
        })
        .detach();

        cx.observe_global::<ActiveView>(|this, cx| {
            if cx.global::<ActiveView>().0 == AppView::MissingSongs {
                this.reload(cx);
            }
        })
        .detach();

        cx.observe_global::<LibraryDataChanged>(|this, cx| {
            if cx.global::<ActiveView>().0 == AppView::MissingSongs {
                this.reload(cx);
            }
        })
        .detach();

        cx.observe_global::<Search>(|this, cx| {
            let query = cx.global::<Search>().query.trim().to_lowercase();
            if query != this.query {
                this.query = query;
                this.rebuild();
                cx.notify();
            }
        })
        .detach();

        view.rebuild();
        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let root = expand_tilde(&source).to_string_lossy().into_owned();
        let db = cx.global::<Database>().clone();
        let bg = cx.background_executor().clone();
        let root_for_query = root.clone();

        self.load_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let files = bg
                .spawn(async move {
                    let missing: HashSet<String> = missing_paths().into_iter().collect();
                    db.get_library_files_in_dir(&root_for_query)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|f| missing.contains(&f.file_path))
                        .collect::<Vec<_>>()
                })
                .await;
            cx.update(|cx| {
                this.update(cx, |this, cx| {
                    this.root = root;
                    this.selected.retain(|id| files.iter().any(|f| &f.id == id));
                    this.files = files;
                    this.rebuild();
                    cx.notify();
                })
            })
            .ok();
        }));
    }

    fn rebuild(&mut self) {
        let query = self.query.clone();
        self.visible = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                query.is_empty()
                    || f.title.to_lowercase().contains(&query)
                    || f.artist.to_lowercase().contains(&query)
                    || f.album.to_lowercase().contains(&query)
                    || f.file_path.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();

        let key = |f: &LibraryFile| -> Option<String> {
            match self.grouping {
                Grouping::None => None,
                Grouping::Album => Some(if f.album.is_empty() {
                    "Unknown".to_string()
                } else {
                    f.album.clone()
                }),
                Grouping::Artist => Some(if f.artist.is_empty() {
                    "Unknown".to_string()
                } else {
                    f.artist.clone()
                }),
                Grouping::Folder => {
                    let dir = relative_dir(f, &self.root);
                    Some(if dir.is_empty() { "/".to_string() } else { dir })
                }
            }
        };

        if self.grouping == Grouping::None {
            self.items = self.visible.iter().map(|&i| Item::Row(i)).collect();
            return;
        }

        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &i in &self.visible {
            if let Some(k) = key(&self.files[i]) {
                groups.entry(k).or_default().push(i);
            }
        }
        let mut items = Vec::with_capacity(self.visible.len() + groups.len());
        for (name, ids) in groups {
            items.push(Item::Header {
                name,
                ids: ids.clone().into(),
            });
            items.extend(ids.into_iter().map(Item::Row));
        }
        self.items = items;
    }

    fn toggle(&mut self, indices: &[usize], cx: &mut Context<Self>) {
        let ids: Vec<Cuid> = indices.iter().map(|&i| self.files[i].id.clone()).collect();
        if ids.iter().all(|id| self.selected.contains(id)) {
            for id in &ids {
                self.selected.remove(id);
            }
        } else {
            self.selected.extend(ids);
        }
        cx.notify();
    }

    fn set_grouping(&mut self, grouping: Grouping, cx: &mut Context<Self>) {
        if self.grouping == grouping {
            return;
        }
        self.grouping = grouping;
        self.rebuild();
        cx.notify();
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        if self.selected.is_empty() || self.remove_task.is_some() {
            return;
        }
        let ids: Vec<Cuid> = self.selected.iter().cloned().collect();
        let paths: HashSet<String> = self
            .files
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .map(|f| f.file_path.clone())
            .collect();
        let db = cx.global::<Database>().clone();
        let scanner = cx.global::<Scanner>().clone();
        let bg = cx.background_executor().clone();

        self.remove_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = bg
                .spawn(async move { scanner.delete_songs_exclusive(&db, &ids).await })
                .await;
            cx.update(|cx| {
                match result {
                    Ok(_) => {
                        forget_missing(&paths);
                        cx.set_global(LibraryDataChanged);
                        cx.set_global(QueueChanged);
                    }
                    Err(e) => error!("failed to remove missing songs: {e}"),
                }
                this.update(cx, |this, cx| {
                    this.remove_task = None;
                    this.reload(cx);
                })
            })
            .ok();
        }));
    }
}

impl Render for MissingSongsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = *cx.global::<Variables>();
        let view = cx.entity();

        let selected_count = self.selected.len();
        let all_selected = !self.visible.is_empty()
            && self
                .visible
                .iter()
                .all(|&i| self.selected.contains(&self.files[i].id));
        let source = self.source.clone().unwrap_or_default();
        let total = self.files.len();

        let header = flex_col()
            .w_full()
            .flex_shrink_0()
            .gap(px(variables.padding_8))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(18.0))
                    .line_height(px(22.0))
                    .child("Missing songs"),
            )
            .child(
                div()
                    .text_color(variables.text_secondary)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!(
                        "{source} \u{00B7} {total} song{}",
                        if total == 1 { "" } else { "s" }
                    )),
            )
            .child(
                flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(variables.padding_8))
                    .pt(px(variables.padding_8))
                    .pb(px(variables.padding_8))
                    .border_b_1()
                    .border_color(variables.element)
                    .child(
                        Button::new("missing-select-all")
                            .variant(ButtonVariant::Default)
                            .child(if all_selected {
                                "Deselect all"
                            } else {
                                "Select all"
                            })
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        let all = this.visible.clone();
                                        this.toggle(&all, cx);
                                    });
                                }
                            }),
                    )
                    .children(Grouping::ALL.into_iter().map(|grouping| {
                        let view = view.clone();
                        Button::new(format!("missing-group-{}", grouping.label()))
                            .variant(if self.grouping == grouping {
                                ButtonVariant::Active
                            } else {
                                ButtonVariant::Default
                            })
                            .child(grouping.label())
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.set_grouping(grouping, cx));
                            })
                    }))
                    .when(selected_count > 0, |this| {
                        let view = view.clone();
                        this.child(
                            Button::new("missing-remove")
                                .color(variables.destructive)
                                .hover_color(variables.destructive)
                                .bg_color(variables.element)
                                .child(format!("Remove {selected_count}"))
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |this, cx| this.remove_selected(cx));
                                }),
                        )
                    }),
            );

        let count = self.items.len();
        let list: AnyElement = if count == 0 {
            div()
                .id("missing-empty")
                .w_full()
                .pt(px(variables.padding_16))
                .text_color(variables.text_secondary)
                .child(if self.files.is_empty() {
                    "No missing songs"
                } else {
                    "No Results Found"
                })
                .into_any_element()
        } else {
            let list_view = view.clone();
            uniform_list(
                ElementId::Name("missing-rows".into()),
                count,
                move |range, _, cx| {
                    let variables = *cx.global::<Variables>();
                    range
                        .map(|idx| {
                            let this = list_view.read(cx);
                            let id = ElementId::Name(format!("missing-item-{idx}").into());
                            let wrap = flex_col().id(id).w_full().h(px(ROW_PITCH));
                            match &this.items[idx] {
                                Item::Header { name, ids } => {
                                    let ids = ids.clone();
                                    let all = ids
                                        .iter()
                                        .all(|&i| this.selected.contains(&this.files[i].id));
                                    let name = format!("{name} \u{00B7} {}", ids.len());
                                    let list_view = list_view.clone();
                                    wrap.justify_end()
                                        .px(px(variables.padding_8))
                                        .pb(px(variables.padding_16))
                                        .cursor_pointer()
                                        .text_color(if all {
                                            variables.accent
                                        } else {
                                            variables.text_secondary
                                        })
                                        .font_weight(FontWeight::MEDIUM)
                                        .on_click(move |_, _, cx| {
                                            list_view.update(cx, |this, cx| {
                                                this.toggle(&ids, cx);
                                            });
                                        })
                                        .child(div().overflow_hidden().text_ellipsis().child(name))
                                        .into_any_element()
                                }
                                Item::Row(file_idx) => {
                                    let file_idx = *file_idx;
                                    let file = &this.files[file_idx];
                                    let selected = this.selected.contains(&file.id);
                                    let cover = cover_uri(
                                        file.image_id.as_deref(),
                                        match &file.album_id {
                                            Some(album_id) => ImageRequest::Album(album_id.clone()),
                                            None => ImageRequest::Track(file.id.clone()),
                                        },
                                    );
                                    let cover = format!("{cover}?size={}", bucket_size(COVER_SIZE));
                                    let title = file.title.clone();
                                    let artist = file.artist.clone();
                                    let album = file.album.clone();
                                    let path = relative_path(file, &this.root).to_string();
                                    let list_view = list_view.clone();
                                    wrap.pb(px(variables.padding_16))
                                        .child(
                                            flex_row()
                                                .id(ElementId::Name(
                                                    format!("missing-row-{idx}").into(),
                                                ))
                                                .size_full()
                                                .items_center()
                                                .gap(px(variables.padding_8))
                                                .px(px(variables.padding_8))
                                                .cursor_pointer()
                                                .when(selected, |row| row.bg(variables.element))
                                                .hover_style(|s| s.bg(variables.element))
                                                .on_click(move |_, _, cx| {
                                                    list_view.update(cx, |this, cx| {
                                                        this.toggle(&[file_idx], cx);
                                                    });
                                                })
                                                .child(
                                                    flex_row()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .gap(px(variables.padding_8))
                                                        .items_center()
                                                        .child(
                                                            div()
                                                                .size(px(COVER_SIZE))
                                                                .flex_shrink_0()
                                                                .bg(variables.element)
                                                                .child(
                                                                    img(cover)
                                                                        .size(px(COVER_SIZE))
                                                                        .object_fit(
                                                                            ObjectFit::Cover,
                                                                        ),
                                                                ),
                                                        )
                                                        .child(
                                                            flex_col()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .gap(px(2.0))
                                                                .items_start()
                                                                .child(
                                                                    div()
                                                                        .max_w_full()
                                                                        .whitespace_nowrap()
                                                                        .overflow_hidden()
                                                                        .text_ellipsis()
                                                                        .font_weight(FontWeight(
                                                                            500.0,
                                                                        ))
                                                                        .when(selected, |d| {
                                                                            d.text_color(
                                                                                variables.accent,
                                                                            )
                                                                        })
                                                                        .child(title),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .max_w_full()
                                                                        .text_color(
                                                                            variables
                                                                                .text_secondary,
                                                                        )
                                                                        .whitespace_nowrap()
                                                                        .overflow_hidden()
                                                                        .text_ellipsis()
                                                                        .child(artist),
                                                                ),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_color(variables.text_secondary)
                                                        .whitespace_nowrap()
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .child(album),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .text_color(variables.text_secondary)
                                                        .whitespace_nowrap()
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .child(path),
                                                ),
                                        )
                                        .into_any_element()
                                }
                            }
                        })
                        .collect()
                },
            )
            .track_scroll(&self.scroll_handle)
            .size_full()
            .pt(px(variables.padding_16))
            .pb(px(variables.padding_24 - variables.padding_16))
            .into_any_element()
        };

        flex_col()
            .size_full()
            .pt(px(variables.padding_24))
            .px(px(variables.padding_16))
            .child(div().px(px(variables.padding_8)).child(header))
            .child(
                div()
                    .id("missing-scroll-container")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .relative()
                    .child(div().id("missing-content").size_full().child(list))
                    .when(count > 0, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top_0()
                                .right(px(-variables.padding_8))
                                .bottom_0()
                                .left_0()
                                .child(
                                    Scrollbar::new(&self.scroll_handle)
                                        .axis(ScrollbarAxis::Vertical),
                                ),
                        )
                    })
                    .smooth_scroll(&self.scroll_handle),
            )
    }
}
