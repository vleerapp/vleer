mod home;
mod settings;
mod songs;

use gpui::*;
use std::collections::HashMap;

use crate::ui::views::{home::HomeView, settings::SettingsView, songs::SongsView};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppView {
    Home,
    Songs,
    Settings,
}

impl Default for AppView {
    fn default() -> Self {
        Self::Home
    }
}

pub trait HoverableView {
    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>)
    where
        Self: Sized;
}

pub struct ViewRegistry;

impl ViewRegistry {
    pub fn register_all(window: &mut Window, cx: &mut App) -> HashMap<AppView, AnyView> {
        let mut views = HashMap::new();

        views.insert(AppView::Home, cx.new(|cx| HomeView::new(window, cx)).into());

        views.insert(
            AppView::Songs,
            cx.new(|cx| SongsView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Settings,
            cx.new(|cx| SettingsView::new(window, cx)).into(),
        );

        views
    }

    pub fn set_hovered(view_type: AppView, view: &AnyView, hovered: bool, cx: &mut App) {
        match view_type {
            AppView::Home => {
                if let Ok(entity) = view.clone().downcast::<HomeView>() {
                    entity.update(cx, |v, cx| {
                        v.set_hovered(hovered, cx);
                    });
                }
            }
            AppView::Songs => {
                if let Ok(entity) = view.clone().downcast::<SongsView>() {
                    entity.update(cx, |v, cx| {
                        v.set_hovered(hovered, cx);
                    });
                }
            }
            AppView::Settings => {
                if let Ok(entity) = view.clone().downcast::<SettingsView>() {
                    entity.update(cx, |v, cx| {
                        v.set_hovered(hovered, cx);
                    });
                }
            }
        }
    }
}
