use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::Duration;

pub use crate::status::status;

use crate::ui::{
    components::{
        div::flex_row,
        icons::{HOME, SETTINGS},
        nav_button::NavButton,
    },
    variables::Variables,
    views::AppView,
};

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
        let entries = status().entries();

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
                    let color = match entry.color {
                        crate::status::StatusColor::Accent => variables.accent,
                        crate::status::StatusColor::Warning => variables.warning,
                        crate::status::StatusColor::Destructive => variables.destructive,
                    };
                    row = row.child(
                        div()
                            .text_color(color)
                            .font_weight(FontWeight(500.0))
                            .child(entry.text),
                    );
                }
                row.child(div().p(px(variables.padding_16)).child(NavButton::new(
                    SETTINGS,
                    Some("Settings"),
                    None,
                    AppView::Settings,
                )))
            })
    }
}

impl Render for NavbarProgressBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let entries = status().entries();
        let bar_color = entries
            .iter()
            .filter_map(|e| e.ratio.map(|r| (r, e.color)))
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(_, c)| c)
            .unwrap_or(crate::status::StatusColor::Accent);
        let ratio = entries
            .iter()
            .filter_map(|e| e.ratio)
            .fold(0.0_f32, f32::max);

        let bg = match bar_color {
            crate::status::StatusColor::Accent => variables.accent,
            crate::status::StatusColor::Warning => variables.warning,
            crate::status::StatusColor::Destructive => variables.destructive,
        };

        div()
            .absolute()
            .left_0()
            .bottom_0()
            .h(px(2.0))
            .when(ratio > 0.0, |this| this.w(relative(ratio)).bg(bg))
    }
}
