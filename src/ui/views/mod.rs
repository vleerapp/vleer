mod albums;
mod artists;
mod home;
mod playlists;
mod settings;
mod songs;

use gpui::*;
use std::collections::HashMap;

use crate::ui::views::{
    albums::AlbumsView, artists::ArtistsView, home::HomeView, playlists::PlaylistsView,
    settings::SettingsView, songs::SongsView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppView {
    Home,
    Songs,
    Settings,
    Albums,
    Artists,
    Playlists,
}

impl AppView {
    pub fn title(&self) -> &'static str {
        match self {
            AppView::Home => "Home",
            AppView::Songs => "Songs",
            AppView::Settings => "Settings",
            AppView::Albums => "Albums",
            AppView::Artists => "Artists",
            AppView::Playlists => "Playlists",
        }
    }
}

impl Default for AppView {
    fn default() -> Self {
        Self::Home
    }
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

        views.insert(
            AppView::Albums,
            cx.new(|cx| AlbumsView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Artists,
            cx.new(|cx| ArtistsView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Playlists,
            cx.new(|cx| PlaylistsView::new(window, cx)).into(),
        );

        views
    }
}
