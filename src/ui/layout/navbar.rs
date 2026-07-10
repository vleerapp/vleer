use gpui::prelude::FluentBuilder;
use gpui::*;
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use crate::ui::{
    components::{
        div::flex_row,
        icons::{HOME, SETTINGS},
        nav_button::NavButton,
    },
    variables::Variables,
    views::AppView,
};

#[derive(Clone)]
pub struct ProgressEntry {
    pub text: String,
    pub ratio: Option<f32>,
}

#[derive(Default)]
pub struct ProgressReporter {
    entries: RwLock<BTreeMap<String, ProgressEntry>>,
}

impl ProgressReporter {
    pub fn set(&self, key: &str, text: impl Into<String>, ratio: Option<f32>) {
        self.entries.write().insert(
            key.to_string(),
            ProgressEntry {
                text: text.into(),
                ratio,
            },
        );
    }

    pub fn clear(&self, key: &str) {
        self.entries.write().remove(key);
    }

    pub fn entries(&self) -> Vec<ProgressEntry> {
        self.entries.read().values().cloned().collect()
    }
}

static PROGRESS: OnceLock<ProgressReporter> = OnceLock::new();

pub fn progress() -> &'static ProgressReporter {
    PROGRESS.get_or_init(ProgressReporter::default)
}

pub struct Navbar {
    _refresh_task: Task<()>,
}

pub struct NavbarProgressBar {
    _refresh_task: Task<()>,
}

fn spawn_refresh<T: 'static>(cx: &mut Context<T>) -> Task<()> {
    cx.spawn(async move |this, cx: &mut AsyncApp| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            if cx
                .update(|cx| {
                    this.update(cx, |_this, cx| {
                        cx.notify();
                    })
                })
                .is_err()
            {
                break;
            }
        }
    })
}

impl Navbar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            _refresh_task: spawn_refresh(cx),
        }
    }
}

impl NavbarProgressBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            _refresh_task: spawn_refresh(cx),
        }
    }
}

impl Render for Navbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let entries = progress().entries();

        flex_row()
            .h_full()
            .justify_between()
            .child(flex_row().p(px(variables.padding_16)).child(NavButton::new(
                HOME,
                Some("Home"),
                None,
                AppView::Home,
            )))
            .child({
                let mut row = flex_row().items_center().gap(px(variables.padding_16));
                for entry in entries {
                    row = row.child(
                        div()
                            .text_color(variables.accent)
                            .font_weight(FontWeight(500.0))
                            .child(entry.text),
                    );
                }
                row.child(div().p(px(variables.padding_16)).child(NavButton::new(
                    SETTINGS,
                    None,
                    None,
                    AppView::Settings,
                )))
            })
    }
}

impl Render for NavbarProgressBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let ratio = progress()
            .entries()
            .iter()
            .filter_map(|e| e.ratio)
            .fold(0.0_f32, f32::max);

        div()
            .absolute()
            .left_0()
            .bottom_0()
            .h(px(2.0))
            .when(ratio > 0.0, |this| {
                this.w(relative(ratio)).bg(variables.accent)
            })
    }
}
