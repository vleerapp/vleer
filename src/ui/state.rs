use gpui::{App, Global};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::data::config::SettingsConfig;
use crate::data::types::{Album, Artist, Cuid, Playlist, Song};
use crate::ui::views::AppView;

#[derive(Clone)]
pub struct State {
    inner: Arc<RwLock<StateInner>>,
}

struct StateInner {
    songs: HashMap<Cuid, Arc<Song>>,
    song_ids: Vec<Cuid>,
    artists: HashMap<Cuid, Arc<Artist>>,
    artist_ids: Vec<Cuid>,
    albums: HashMap<Cuid, Arc<Album>>,
    album_ids: Vec<Cuid>,
    playlists: HashMap<Cuid, Arc<Playlist>>,
    playlist_ids: Vec<Cuid>,
    current_view: AppView,
    config: SettingsConfig,
}

impl Global for State {}

impl State {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StateInner {
                songs: HashMap::new(),
                song_ids: Vec::new(),
                artists: HashMap::new(),
                artist_ids: Vec::new(),
                albums: HashMap::new(),
                album_ids: Vec::new(),
                playlists: HashMap::new(),
                playlist_ids: Vec::new(),
                current_view: AppView::default(),
                config: SettingsConfig::default(),
            })),
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(Self::new());
    }

    pub async fn get_current_view(&self) -> AppView {
        let inner = self.inner.read().await;
        inner.current_view
    }

    pub async fn set_current_view(&self, view: AppView) {
        let mut inner = self.inner.write().await;
        inner.current_view = view;
    }

    pub fn get_current_view_sync(&self) -> AppView {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_current_view())
        })
    }

    pub fn set_current_view_sync(&self, view: AppView) {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.set_current_view(view))
        })
    }

    pub async fn get_config(&self) -> SettingsConfig {
        let inner = self.inner.read().await;
        inner.config.clone()
    }

    pub async fn set_config(&self, config: SettingsConfig) {
        let mut inner = self.inner.write().await;
        inner.config = config;
    }

    pub async fn set_songs(&self, songs: Vec<Song>) {
        let mut inner = self.inner.write().await;
        inner.songs.clear();
        inner.song_ids.clear();

        for song in songs {
            let id = song.id.clone();
            inner.song_ids.push(id.clone());
            inner.songs.insert(id, Arc::new(song));
        }
    }

    pub async fn get_song(&self, id: &Cuid) -> Option<Arc<Song>> {
        let inner = self.inner.read().await;
        inner.songs.get(id).cloned()
    }

    pub async fn get_all_song_ids(&self) -> Vec<Cuid> {
        let inner = self.inner.read().await;
        inner.song_ids.clone()
    }

    pub async fn add_song(&self, song: Song) {
        let mut inner = self.inner.write().await;
        let id = song.id.clone();
        inner.song_ids.push(id.clone());
        inner.songs.insert(id, Arc::new(song));
    }

    pub async fn update_song(&self, song: Song) {
        let mut inner = self.inner.write().await;
        let id = song.id.clone();
        inner.songs.insert(id, Arc::new(song));
    }

    pub async fn remove_song(&self, id: &Cuid) {
        let mut inner = self.inner.write().await;
        inner.songs.remove(id);
        inner.song_ids.retain(|song_id| song_id != id);
    }

    pub async fn set_artists(&self, artists: Vec<Artist>) {
        let mut inner = self.inner.write().await;
        inner.artists.clear();
        inner.artist_ids.clear();

        for artist in artists {
            let id = artist.id.clone();
            inner.artist_ids.push(id.clone());
            inner.artists.insert(id, Arc::new(artist));
        }
    }

    pub async fn get_artist(&self, id: &Cuid) -> Option<Arc<Artist>> {
        let inner = self.inner.read().await;
        inner.artists.get(id).cloned()
    }

    pub async fn get_all_artist_ids(&self) -> Vec<Cuid> {
        let inner = self.inner.read().await;
        inner.artist_ids.clone()
    }

    pub async fn add_artist(&self, artist: Artist) {
        let mut inner = self.inner.write().await;
        let id = artist.id.clone();
        inner.artist_ids.push(id.clone());
        inner.artists.insert(id, Arc::new(artist));
    }

    pub async fn set_albums(&self, albums: Vec<Album>) {
        let mut inner = self.inner.write().await;
        inner.albums.clear();
        inner.album_ids.clear();

        for album in albums {
            let id = album.id.clone();
            inner.album_ids.push(id.clone());
            inner.albums.insert(id, Arc::new(album));
        }
    }

    pub async fn get_album(&self, id: &Cuid) -> Option<Arc<Album>> {
        let inner = self.inner.read().await;
        inner.albums.get(id).cloned()
    }

    pub async fn get_all_album_ids(&self) -> Vec<Cuid> {
        let inner = self.inner.read().await;
        inner.album_ids.clone()
    }

    pub async fn add_album(&self, album: Album) {
        let mut inner = self.inner.write().await;
        let id = album.id.clone();
        inner.album_ids.push(id.clone());
        inner.albums.insert(id, Arc::new(album));
    }

    pub async fn set_playlists(&self, playlists: Vec<Playlist>) {
        let mut inner = self.inner.write().await;
        inner.playlists.clear();
        inner.playlist_ids.clear();

        for playlist in playlists {
            let id = playlist.id.clone();
            inner.playlist_ids.push(id.clone());
            inner.playlists.insert(id, Arc::new(playlist));
        }
    }

    pub async fn get_playlist(&self, id: &Cuid) -> Option<Arc<Playlist>> {
        let inner = self.inner.read().await;
        inner.playlists.get(id).cloned()
    }

    pub async fn get_all_playlist_ids(&self) -> Vec<Cuid> {
        let inner = self.inner.read().await;
        inner.playlist_ids.clone()
    }

    pub async fn add_playlist(&self, playlist: Playlist) {
        let mut inner = self.inner.write().await;
        let id = playlist.id.clone();
        inner.playlist_ids.push(id.clone());
        inner.playlists.insert(id, Arc::new(playlist));
    }

    pub async fn get_artist_name_for_song(&self, song: &Song) -> String {
        if let Some(artist_id) = &song.artist_id {
            if let Some(artist) = self.get_artist(artist_id).await {
                return artist.name.clone();
            }
        }
        "Unknown".to_string()
    }

    pub async fn get_album_title_for_song(&self, song: &Song) -> String {
        if let Some(album_id) = &song.album_id {
            if let Some(album) = self.get_album(album_id).await {
                return album.title.clone();
            }
        }
        "Unknown".to_string()
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
