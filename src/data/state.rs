use gpui::{App, Global};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use crate::data::config::{Config, SettingsConfig};
use crate::data::db::Database;
use crate::data::telemetry::Telemetry;
use crate::data::types::{self, Album, Artist, Cuid, Playlist, Song};
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
    playlist_tracks: HashMap<Cuid, Vec<Cuid>>,
    current_view: AppView,
    config: SettingsConfig,
    search_query: String,
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
                playlist_tracks: HashMap::new(),
                current_view: AppView::default(),
                config: SettingsConfig::default(),
                search_query: String::new(),
            })),
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(Self::new());
        State::prepare(cx);
    }

    pub fn prepare(cx: &mut App) {
        let config = cx.global::<Config>().clone();
        let state = cx.global::<State>().clone();

        let config_for_settings = config.clone();
        let state_for_settings = state.clone();

        tokio::spawn(async move {
            state_for_settings
                .set_config(config_for_settings.get().clone())
                .await;
        });

        let db = cx.global::<Database>().clone();
        let telemetry = cx.global::<Telemetry>().clone();

        tokio::spawn(async move {
            Self::refresh(&db, &state).await;
            telemetry.submit(&state, &config).await;
        });
    }

    pub async fn refresh(db: &Database, state: &State) {
        let db_songs = db.get_all_songs().await.expect("Failed to fetch songs");
        let db_artists = db.get_all_artists().await.expect("Failed to fetch artists");
        let db_albums = db.get_all_albums().await.expect("Failed to fetch albums");
        let db_playlists = db
            .get_all_playlists()
            .await
            .expect("Failed to fetch playlists");
        let db_playlist_tracks = db
            .get_all_playlist_tracks()
            .await
            .expect("Failed to fetch playlist tracks");

        let artists: Vec<Artist> = db_artists
            .into_iter()
            .map(|a| Artist {
                id: a.id,
                name: a.name,
                image: a.image,
                favorite: a.favorite,
                pinned: a.pinned,
            })
            .collect();

        let artist_map: HashMap<Cuid, Arc<Artist>> = artists
            .iter()
            .map(|a| (a.id.clone(), Arc::new(a.clone())))
            .collect();

        let albums: Vec<Album> = db_albums
            .into_iter()
            .map(|a| {
                let artist = a.artist.as_ref().and_then(|id| artist_map.get(id).cloned());
                Album {
                    id: a.id,
                    title: a.title,
                    artist,
                    cover: a.cover,
                    favorite: a.favorite,
                    pinned: a.pinned,
                }
            })
            .collect();

        let album_map: HashMap<Cuid, Arc<Album>> = albums
            .iter()
            .map(|a| (a.id.clone(), Arc::new(a.clone())))
            .collect();

        let songs: Vec<Song> = db_songs
            .into_iter()
            .map(|db_song| {
                let artist = db_song
                    .artist_id
                    .as_ref()
                    .and_then(|id| artist_map.get(id).cloned());
                let album = db_song
                    .album_id
                    .as_ref()
                    .and_then(|id| album_map.get(id).cloned());

                Song {
                    id: db_song.id,
                    title: db_song.title,
                    artist,
                    album,
                    file_path: db_song.file_path,
                    genre: db_song.genre,
                    date: db_song.date,
                    duration: db_song.duration,
                    cover: db_song.cover,
                    track_number: db_song.track_number,
                    favorite: db_song.favorite,
                    track_lufs: db_song.track_lufs,
                    pinned: db_song.pinned,
                    date_added: db_song.date_added,
                }
            })
            .collect();

        let mut tracks_by_playlist: HashMap<Cuid, Vec<types::db::PlaylistTrack>> = HashMap::new();
        for track in db_playlist_tracks {
            tracks_by_playlist
                .entry(track.playlist_id.clone())
                .or_default()
                .push(track);
        }

        let mut final_playlist_tracks: HashMap<Cuid, Vec<Cuid>> = HashMap::new();
        for (playlist_id, mut tracks) in tracks_by_playlist {
            tracks.sort_by_key(|t| t.position);
            let song_ids = tracks.into_iter().map(|t| t.song_id).collect();
            final_playlist_tracks.insert(playlist_id, song_ids);
        }

        state.set_artists(artists).await;
        state.set_albums(albums).await;
        state.set_playlists(db_playlists).await;
        state.set_songs(songs).await;
        state.set_playlist_tracks(final_playlist_tracks).await;

        info!(
            "Successfully refreshed state with {} songs, {} artists, {} albums, {} playlists",
            state.get_all_song_ids().await.len(),
            state.get_all_artist_ids().await.len(),
            state.get_all_album_ids().await.len(),
            state.get_all_playlist_ids().await.len()
        );
    }

    pub async fn get_search_query(&self) -> String {
        let inner = self.inner.read().await;
        inner.search_query.clone()
    }

    pub async fn set_search_query(&self, query: String) {
        let mut inner = self.inner.write().await;
        inner.search_query = query;
    }

    pub fn get_search_query_sync(&self) -> String {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_search_query())
        })
    }

    pub fn set_search_query_sync(&self, query: String) {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.set_search_query(query))
        })
    }

    pub async fn get_all_songs(&self) -> Vec<Arc<Song>> {
        let inner = self.inner.read().await;
        inner.songs.values().cloned().collect()
    }

    pub fn get_all_songs_sync(&self) -> Vec<Arc<Song>> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_all_songs())
        })
    }

    pub async fn search_all_items(
        &self,
        query: &str,
    ) -> Vec<(Cuid, String, Option<String>, String)> {
        let query_lower = query.to_lowercase();
        let inner = self.inner.read().await;
        let mut items = Vec::new();
        let mut album_titles = std::collections::HashSet::new();

        for album in inner.albums.values() {
            if album.title.to_lowercase().contains(&query_lower)
                || album
                    .artist
                    .as_ref()
                    .map_or(false, |a| a.name.to_lowercase().contains(&query_lower))
            {
                items.push((
                    album.id.clone(),
                    album.title.clone(),
                    album.cover.clone(),
                    "Album".to_string(),
                ));
                album_titles.insert(album.title.to_lowercase());
            }
        }

        for song in inner.songs.values() {
            let song_title_lower = song.title.to_lowercase();
            if (song_title_lower.contains(&query_lower)
                || song
                    .artist
                    .as_ref()
                    .map_or(false, |a| a.name.to_lowercase().contains(&query_lower))
                || song
                    .album
                    .as_ref()
                    .map_or(false, |a| a.title.to_lowercase().contains(&query_lower)))
                && !album_titles.contains(&song_title_lower)
            {
                items.push((
                    song.id.clone(),
                    song.title.clone(),
                    song.cover.clone(),
                    "Song".to_string(),
                ));
            }
        }

        for artist in inner.artists.values() {
            if artist.name.to_lowercase().contains(&query_lower) {
                items.push((
                    artist.id.clone(),
                    artist.name.clone(),
                    artist.image.clone(),
                    "Artist".to_string(),
                ));
            }
        }

        for playlist in inner.playlists.values() {
            if playlist.name.to_lowercase().contains(&query_lower) {
                items.push((
                    playlist.id.clone(),
                    playlist.name.clone(),
                    playlist.image.clone(),
                    "Playlist".to_string(),
                ));
            }
        }

        items.sort_by(|a, b| {
            let title_a_lower = a.1.to_lowercase();
            let title_b_lower = b.1.to_lowercase();

            let is_exact_a = match a.3.as_str() {
                "Artist" | "Playlist" => title_a_lower == query_lower,
                "Album" | "Song" => title_a_lower == query_lower,
                _ => false,
            };
            let is_exact_b = match b.3.as_str() {
                "Artist" | "Playlist" => title_b_lower == query_lower,
                "Album" | "Song" => title_b_lower == query_lower,
                _ => false,
            };
            let exact_score_a = if is_exact_a { 0 } else { 1 };
            let exact_score_b = if is_exact_b { 0 } else { 1 };

            let len_score_a = if a.1.len() <= 30 { 0 } else { 1 };
            let len_score_b = if b.1.len() <= 30 { 0 } else { 1 };

            exact_score_a
                .cmp(&exact_score_b)
                .then(len_score_a.cmp(&len_score_b))
                .then(title_a_lower.cmp(&title_b_lower))
        });

        items
    }

    pub fn search_all_items_sync(
        &self,
        query: &str,
    ) -> Vec<(Cuid, String, Option<String>, String)> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.search_all_items(query))
        })
    }

    pub async fn get_search_match_counts(&self, query: &str) -> (usize, usize, usize, usize) {
        let query_lower = query.to_lowercase();
        let inner = self.inner.read().await;

        let song_count = inner
            .songs
            .values()
            .filter(|song| {
                song.title.to_lowercase().contains(&query_lower)
                    || song
                        .artist
                        .as_ref()
                        .map_or(false, |a| a.name.to_lowercase().contains(&query_lower))
                    || song
                        .album
                        .as_ref()
                        .map_or(false, |a| a.title.to_lowercase().contains(&query_lower))
            })
            .count();

        let album_count = inner
            .albums
            .values()
            .filter(|album| {
                album.title.to_lowercase().contains(&query_lower)
                    || album
                        .artist
                        .as_ref()
                        .map_or(false, |a| a.name.to_lowercase().contains(&query_lower))
            })
            .count();

        let artist_count = inner
            .artists
            .values()
            .filter(|artist| artist.name.to_lowercase().contains(&query_lower))
            .count();

        let playlist_count = inner
            .playlists
            .values()
            .filter(|playlist| playlist.name.to_lowercase().contains(&query_lower))
            .count();

        (song_count, album_count, artist_count, playlist_count)
    }

    pub fn get_search_match_counts_sync(&self, query: &str) -> (usize, usize, usize, usize) {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.get_search_match_counts(query))
        })
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

    pub async fn set_playlist_tracks(&self, tracks: HashMap<Cuid, Vec<Cuid>>) {
        let mut inner = self.inner.write().await;
        inner.playlist_tracks = tracks;
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

    pub async fn get_album_songs(&self, id: &Cuid) -> Option<Vec<Arc<Song>>> {
        let inner = self.inner.read().await;
        if !inner.albums.contains_key(id) {
            return None;
        }

        let mut songs: Vec<Arc<Song>> = inner
            .songs
            .values()
            .filter(|song| song.album.as_ref().map(|a| &a.id) == Some(id))
            .cloned()
            .collect();

        songs.sort_by_key(|s| s.track_number);

        Some(songs)
    }

    pub async fn get_playlist_songs(&self, id: &Cuid) -> Option<Vec<Arc<Song>>> {
        let inner = self.inner.read().await;

        if !inner.playlists.contains_key(id) {
            return None;
        }

        let song_ids = inner.playlist_tracks.get(id)?;

        let mut songs = Vec::new();
        for song_id in song_ids {
            if let Some(song) = inner.songs.get(song_id) {
                songs.push(song.clone());
            }
        }

        Some(songs)
    }

    pub fn get_pinned_items_sync(&self) -> Vec<(Cuid, String, Option<String>, String)> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let inner = self.inner.read().await;
                let mut items = Vec::new();

                for artist in inner.artists.values() {
                    if artist.pinned {
                        items.push((
                            artist.id.clone(),
                            artist.name.clone(),
                            artist.image.clone(),
                            "Artist".to_string(),
                        ));
                    }
                }
                for album in inner.albums.values() {
                    if album.pinned {
                        items.push((
                            album.id.clone(),
                            album.title.clone(),
                            album.cover.clone(),
                            "Album".to_string(),
                        ));
                    }
                }
                for playlist in inner.playlists.values() {
                    if playlist.pinned {
                        items.push((
                            playlist.id.clone(),
                            playlist.name.clone(),
                            playlist.image.clone(),
                            "Playlist".to_string(),
                        ));
                    }
                }
                for song in inner.songs.values() {
                    if song.pinned {
                        items.push((
                            song.id.clone(),
                            song.title.clone(),
                            song.cover.clone(),
                            "Song".to_string(),
                        ));
                    }
                }
                items
            })
        })
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
