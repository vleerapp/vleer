mod album;
mod albums;
mod artists;
mod home;
mod playlist;
mod playlists;
mod settings;
mod songs;

use gpui::*;
use std::collections::HashMap;

use crate::data::models::Cuid;
use crate::ui::views::{
    album::AlbumView, albums::AlbumsView, artists::ArtistsView, home::HomeView,
    playlist::PlaylistView, playlists::PlaylistsView, settings::SettingsView, songs::SongsView,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppView {
    #[default]
    Home,
    Songs,
    Settings,
    Albums,
    Album,
    Artists,
    Playlists,
    Playlist,
}

impl AppView {
    pub fn title(&self) -> &'static str {
        match self {
            AppView::Home => "Home",
            AppView::Songs => "Songs",
            AppView::Settings => "Settings",
            AppView::Albums => "Albums",
            AppView::Album => "Album",
            AppView::Artists => "Artists",
            AppView::Playlists => "Playlists",
            AppView::Playlist => "Playlist",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveView(pub AppView);

impl Default for ActiveView {
    fn default() -> Self {
        Self(AppView::Home)
    }
}

impl Global for ActiveView {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedAlbum(pub Option<Cuid>);

impl Global for SelectedAlbum {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedPlaylist {
    pub id: Option<Cuid>,
    pub focus_title: bool,
}

impl Global for SelectedPlaylist {}

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
            AppView::Album,
            cx.new(|cx| AlbumView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Artists,
            cx.new(|cx| ArtistsView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Playlists,
            cx.new(|cx| PlaylistsView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Playlist,
            cx.new(|cx| PlaylistView::new(window, cx)).into(),
        );

        views
    }
}
