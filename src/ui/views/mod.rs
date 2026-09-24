mod album;
mod albums;
mod artists;
mod genres;
mod home;
mod playlist;
mod playlists;
mod settings;
mod songs;

use gpui::*;
use std::collections::HashMap;

use crate::data::models::Cuid;
use crate::ui::views::{
    album::AlbumView, albums::AlbumsView, artists::ArtistsView, genres::GenresView, home::HomeView,
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
    Genres,
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
            AppView::Genres => "Genres",
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavEntry {
    pub view: AppView,
    pub album: Option<Cuid>,
    pub playlist: Option<Cuid>,
}

impl NavEntry {
    pub fn capture(view: AppView, cx: &App) -> Self {
        Self {
            view,
            album: (view == AppView::Album)
                .then(|| cx.global::<SelectedAlbum>().0.clone())
                .flatten(),
            playlist: (view == AppView::Playlist)
                .then(|| cx.global::<SelectedPlaylist>().id.clone())
                .flatten(),
        }
    }

    pub fn restore(&self, cx: &mut App) {
        if let Some(album) = self.album.clone() {
            cx.set_global(SelectedAlbum(Some(album)));
        }
        if let Some(playlist) = self.playlist.clone() {
            cx.update_global::<SelectedPlaylist, _>(|sel, _| {
                sel.id = Some(playlist);
                sel.focus_title = false;
            });
        }
    }
}

#[derive(Debug)]
pub struct NavHistory {
    current: NavEntry,
    back: Vec<NavEntry>,
    forward: Vec<NavEntry>,
}

impl NavHistory {
    pub fn new(view: AppView) -> Self {
        Self {
            current: NavEntry {
                view,
                album: None,
                playlist: None,
            },
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: NavEntry) -> bool {
        if self.current == entry {
            return false;
        }
        self.back.push(std::mem::replace(&mut self.current, entry));
        self.forward.clear();
        true
    }

    pub fn go_back(&mut self) -> Option<NavEntry> {
        let entry = self.back.pop()?;
        self.forward
            .push(std::mem::replace(&mut self.current, entry.clone()));
        Some(entry)
    }

    pub fn go_forward(&mut self) -> Option<NavEntry> {
        let entry = self.forward.pop()?;
        self.back
            .push(std::mem::replace(&mut self.current, entry.clone()));
        Some(entry)
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
            AppView::Album,
            cx.new(|cx| AlbumView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Artists,
            cx.new(|cx| ArtistsView::new(window, cx)).into(),
        );

        views.insert(
            AppView::Genres,
            cx.new(|cx| GenresView::new(window, cx)).into(),
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
