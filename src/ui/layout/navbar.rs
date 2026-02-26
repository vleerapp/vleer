use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::Duration;

use crate::{
    data::scanner::Scanner,
    ui::{
        components::{
            div::flex_row,
            icons::icons::{HOME, SETTINGS},
            nav_button::NavButton,
        },
        variables::Variables,
        views::AppView,
    },
};

pub struct Navbar {
    _refresh_task: Task<()>,
}

pub struct NavbarScanProgressBar {
    _refresh_task: Task<()>,
}

impl Navbar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let refresh_task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
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
        });

        Self {
            _refresh_task: refresh_task,
        }
    }
}

impl NavbarScanProgressBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let refresh_task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
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
        });

        Self {
            _refresh_task: refresh_task,
        }
    }
}

impl Render for Navbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let scanning_text = cx
            .try_global::<Scanner>()
            .map(|scanner| {
                let progress = scanner.get_scan_progress();
                if !progress.active || progress.total == 0 || progress.current == 0 {
                    return None;
                }

                let ratio = (progress.current as f32 / progress.total as f32).clamp(0.0, 1.0);
                let percent = (ratio as f64) * 100.0;
                Some(format!(
                    "Scanning: {}/{} - {:.2}%",
                    progress.current, progress.total, percent
                ))
            })
            .unwrap_or(None);

        flex_row()
            .h_full()
            .justify_between()
            .child(flex_row().p(px(variables.padding_16)).child(NavButton::new(
                HOME,
                Some("Home"),
                None,
                AppView::Home,
            )))
            .child(
                flex_row()
                    .items_center()
                    .when_some(scanning_text, |this, text| {
                        this.child(
                            div()
                                .text_color(variables.accent)
                                .font_weight(FontWeight(500.0))
                                .child(text),
                        )
                    })
                    .child(div().p(px(variables.padding_16)).child(NavButton::new(
                        SETTINGS,
                        None,
                        None,
                        AppView::Settings,
                    ))),
            )
    }
}

impl Render for NavbarScanProgressBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = cx.global::<Variables>();
        let scan_ratio = cx
            .try_global::<Scanner>()
            .and_then(|scanner| {
                let progress = scanner.get_scan_progress();
                if !progress.active || progress.total == 0 {
                    return None;
                }
                Some((progress.current as f32 / progress.total as f32).clamp(0.0, 1.0))
            })
            .unwrap_or(0.0);

        div()
            .absolute()
            .left_0()
            .bottom_0()
            .h(px(2.0))
            .when(scan_ratio > 0.0, |this| {
                this.w(relative(scan_ratio)).bg(variables.accent)
            })
    }
}
