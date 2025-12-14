use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use gpui::{App, Global};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use tracing::debug;

use crate::data::types::{self, Album, Artist, Cuid, EventContext, Playlist, Song};

pub async fn create_pool(path: impl AsRef<Path>) -> Result<SqlitePool, sqlx::Error> {
    debug!("Creating database pool at {:?}", path.as_ref());

    let options = SqliteConnectOptions::new()
        .filename(path)
        .optimize_on_close(true, None)
        .synchronous(SqliteSynchronous::Normal)
        .journal_mode(SqliteJournalMode::Wal)
        .statement_cache_capacity(0)
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Global for Database {}

impl Database {
    pub fn init(cx: &mut App, pool: SqlitePool) -> anyhow::Result<()> {
        cx.set_global(Database { pool });
        Ok(())
    }

    pub async fn insert_song(
        &self,
        title: &str,
        artist_id: Option<&Cuid>,
        album_id: Option<&Cuid>,
        file_path: &str,
        duration: i32,
        track_number: Option<i32>,
        year: Option<i32>,
        genre: Option<&str>,
        cover: Option<&str>,
        track_lufs: Option<f32>,
    ) -> Result<Cuid, sqlx::Error> {
        let id = Cuid::new();
        let year_str = year.map(|y| y.to_string());
        sqlx::query(
            "INSERT INTO songs (id, title, artist_id, album_id, file_path, genre, date, duration, cover, track_number, track_lufs)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(title)
        .bind(artist_id)
        .bind(album_id)
        .bind(file_path)
        .bind(genre)
        .bind(year_str)
        .bind(duration)
        .bind(cover)
        .bind(track_number)
        .bind(track_lufs)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn get_all_songs(&self) -> Result<Vec<types::db::Song>, sqlx::Error> {
        sqlx
            ::query_as::<_, types::db::Song>(
                "SELECT id, title, artist_id, album_id, file_path, genre, date, date_added, duration, cover, track_number, favorite, track_lufs
         FROM songs"
            )
            .fetch_all(&self.pool).await
    }

    pub async fn get_song(&self, id: &Cuid) -> Result<types::db::Song, sqlx::Error> {
        sqlx::query_as::<_, types::db::Song>("SELECT * FROM songs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_song_by_path(
        &self,
        file_path: &str,
    ) -> Result<Option<types::db::Song>, sqlx::Error> {
        sqlx
            ::query_as::<_, types::db::Song>(
                "SELECT id, title, artist_id, album_id, file_path, genre, date, date_added, duration, cover, track_number, favorite, track_lufs
         FROM songs WHERE file_path = ?"
            )
            .bind(file_path)
            .fetch_optional(&self.pool).await
    }

    pub async fn delete_song(&self, id: &Cuid) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM songs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_song_by_path(&self, file_path: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM songs WHERE file_path = ?")
            .bind(file_path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_song_metadata(
        &self,
        id: &Cuid,
        title: &str,
        artist_id: Option<&Cuid>,
        album_id: Option<&Cuid>,
        duration: i32,
        track_number: Option<i32>,
        year: Option<i32>,
        genre: Option<&str>,
        cover: Option<&str>,
        track_lufs: Option<f32>,
    ) -> Result<(), sqlx::Error> {
        let year_str = year.map(|y| y.to_string());
        sqlx
            ::query(
                "UPDATE songs SET title = ?, artist_id = ?, album_id = ?, duration = ?, track_number = ?, date = ?, genre = ?, cover = ?, track_lufs = ?
         WHERE id = ?"
            )
            .bind(title)
            .bind(artist_id)
            .bind(album_id)
            .bind(duration)
            .bind(track_number)
            .bind(year_str)
            .bind(genre)
            .bind(cover)
            .bind(track_lufs)
            .bind(id)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_recently_played_songs(&self) -> Result<Vec<types::db::Song>, sqlx::Error> {
        sqlx
            ::query_as::<_, types::db::Song>(
                "SELECT DISTINCT s.id, s.title, s.artist_id, s.album_id, s.file_path, s.genre, s.date, s.date_added,
                s.duration, s.cover, s.track_number, s.favorite, s.track_lufs
         FROM playback_history ph
         JOIN songs s ON ph.song_id = s.id
         WHERE ph.event_type = 'PLAY'
         ORDER BY ph.timestamp DESC"
            )
            .fetch_all(&self.pool).await
    }

    pub async fn get_recently_added_songs(
        &self,
        limit: i32,
    ) -> Result<Vec<types::db::Song>, sqlx::Error> {
        sqlx
            ::query_as::<_, types::db::Song>(
                "SELECT id, title, artist_id, album_id, file_path, genre, date, date_added, duration, cover, track_number, favorite, track_lufs
         FROM songs
         ORDER BY date_added DESC
         LIMIT ?"
            )
            .bind(limit)
            .fetch_all(&self.pool).await
    }

    pub async fn get_artist_name(&self, id: &Cuid) -> Result<Option<String>, sqlx::Error> {
        let result: Option<(String,)> = sqlx::query_as("SELECT name FROM artists WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(result.map(|(name,)| name))
    }

    pub async fn insert_artist(
        &self,
        name: &str,
        _album_artist: Option<&str>,
    ) -> Result<Cuid, sqlx::Error> {
        let existing: Option<(Cuid,)> = sqlx::query_as("SELECT id FROM artists WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;

        if let Some((id,)) = existing {
            return Ok(id);
        }

        let id = Cuid::new();
        sqlx::query("INSERT INTO artists (id, name) VALUES (?, ?)")
            .bind(&id)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get_all_artists(&self) -> Result<Vec<types::db::Artist>, sqlx::Error> {
        sqlx::query_as::<_, types::db::Artist>("SELECT id, name, image, favorite FROM artists")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_artist(&self, id: &Cuid) -> Result<types::db::Artist, sqlx::Error> {
        sqlx::query_as::<_, types::db::Artist>("SELECT * FROM artists WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn insert_album(
        &self,
        title: &str,
        artist: Option<&Cuid>,
        _year: Option<i32>,
        _genre: Option<&str>,
        cover: Option<&str>,
    ) -> Result<Cuid, sqlx::Error> {
        let existing: Option<(Cuid,)> = sqlx
            ::query_as(
                "SELECT id FROM albums WHERE title = ? AND (artist = ? OR (artist IS NULL AND ? IS NULL))"
            )
            .bind(title)
            .bind(artist)
            .bind(artist)
            .fetch_optional(&self.pool).await?;

        if let Some((id,)) = existing {
            if let Some(cover_path) = cover {
                sqlx::query("UPDATE albums SET cover = ? WHERE id = ?")
                    .bind(cover_path)
                    .bind(&id)
                    .execute(&self.pool)
                    .await?;
            }
            return Ok(id);
        }

        let id = Cuid::new();
        sqlx::query("INSERT INTO albums (id, title, artist, cover) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(title)
            .bind(artist)
            .bind(cover)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get_all_albums(&self) -> Result<Vec<types::db::Album>, sqlx::Error> {
        sqlx::query_as::<_, types::db::Album>(
            "SELECT id, title, artist, cover, favorite FROM albums",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_album(&self, id: &Cuid) -> Result<types::db::Album, sqlx::Error> {
        sqlx::query_as::<_, types::db::Album>("SELECT * FROM albums WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn insert_playlist(
        &self,
        name: String,
        description: Option<String>,
        image: Option<String>,
    ) -> Result<Cuid, sqlx::Error> {
        let id = Cuid::new();
        sqlx::query("INSERT INTO playlists (id, name, description, image) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(&name)
            .bind(&description)
            .bind(&image)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get_all_playlists(&self) -> Result<Vec<Playlist>, sqlx::Error> {
        sqlx::query_as::<_, Playlist>(
            "SELECT id, name, description, image, date_created, date_updated FROM playlists",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_playlist(&self, id: &Cuid) -> Result<Playlist, sqlx::Error> {
        sqlx::query_as::<_, Playlist>("SELECT * FROM playlists WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn insert_event(
        &self,
        event_type: crate::data::types::EventType,
        context_id: Option<Cuid>,
    ) -> Result<Cuid, sqlx::Error> {
        let id = Cuid::new();
        sqlx::query("INSERT INTO events (id, event_type, context_id) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&event_type)
            .bind(&context_id)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn insert_event_context(
        &self,
        song_id: Option<Cuid>,
        playlist_id: Option<Cuid>,
    ) -> Result<Cuid, sqlx::Error> {
        let id = Cuid::new();
        sqlx::query("INSERT INTO event_contexts (id, song_id, playlist_id) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&song_id)
            .bind(&playlist_id)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get_event_context(&self, id: &Cuid) -> Result<EventContext, sqlx::Error> {
        sqlx::query_as::<_, EventContext>("SELECT * FROM event_contexts WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn cleanup_orphaned_artists(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx
            ::query(
                "DELETE FROM artists WHERE id NOT IN (SELECT DISTINCT artist_id FROM songs WHERE artist_id IS NOT NULL)"
            )
            .execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    pub async fn cleanup_orphaned_albums(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx
            ::query(
                "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM songs WHERE album_id IS NOT NULL)"
            )
            .execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    pub async fn hydrate(&self, db_songs: Vec<types::db::Song>) -> Result<Vec<Song>, sqlx::Error> {
        let db_artists = self.get_all_artists().await?;
        let db_albums = self.get_all_albums().await?;

        let mut artists: HashMap<Cuid, Arc<Artist>> = HashMap::new();
        let mut albums: HashMap<Cuid, Arc<Album>> = HashMap::new();

        for db_artist in db_artists {
            let artist = Artist {
                id: db_artist.id.clone(),
                name: db_artist.name,
                image: db_artist.image,
                favorite: db_artist.favorite,
            };
            artists.insert(db_artist.id, Arc::new(artist));
        }

        for db_album in db_albums {
            let artist = db_album
                .artist
                .as_ref()
                .and_then(|id| artists.get(id).cloned());
            let album = Album {
                id: db_album.id.clone(),
                title: db_album.title,
                artist,
                cover: db_album.cover,
                favorite: db_album.favorite,
            };
            albums.insert(db_album.id, Arc::new(album));
        }

        let songs: Vec<Song> = db_songs
            .into_iter()
            .map(|db_song| {
                let artist = db_song
                    .artist_id
                    .as_ref()
                    .and_then(|id| artists.get(id).cloned());
                let album = db_song
                    .album_id
                    .as_ref()
                    .and_then(|id| albums.get(id).cloned());

                Song {
                    id: db_song.id,
                    title: db_song.title,
                    artist,
                    album,
                    file_path: db_song.file_path,
                    genre: db_song.genre,
                    date: db_song.date,
                    date_added: db_song.date_added,
                    duration: db_song.duration,
                    cover: db_song.cover,
                    track_number: db_song.track_number,
                    favorite: db_song.favorite,
                    track_lufs: db_song.track_lufs,
                }
            })
            .collect();

        Ok(songs)
    }
}
