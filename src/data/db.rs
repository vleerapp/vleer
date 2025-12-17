use std::path::Path;

use gpui::{App, Global};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};
use tracing::debug;

use crate::data::types::{self, Cuid, EventContext, Playlist};

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
        sqlx::query_as::<_, types::db::Song>(
            "SELECT id, title, artist_id, album_id, file_path, genre, date, duration, cover, track_number, favorite, track_lufs, pinned, date_added
             FROM songs"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_song(&self, id: &Cuid) -> Result<types::db::Song, sqlx::Error> {
        sqlx::query_as::<_, types::db::Song>(
            "SELECT id, title, artist_id, album_id, file_path, genre, date, duration, cover, track_number, favorite, track_lufs, pinned, date_added
             FROM songs WHERE id = ?"
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_song_by_path(
        &self,
        file_path: &str,
    ) -> Result<Option<types::db::Song>, sqlx::Error> {
        sqlx::query_as::<_, types::db::Song>(
            "SELECT id, title, artist_id, album_id, file_path, genre, date, duration, cover, track_number, favorite, track_lufs, pinned, date_added
             FROM songs WHERE file_path = ?"
        )
        .bind(file_path)
        .fetch_optional(&self.pool)
        .await
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
        sqlx::query(
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_recently_played_songs(&self) -> Result<Vec<types::db::Song>, sqlx::Error> {
        sqlx::query_as::<_, types::db::Song>(
            "SELECT DISTINCT s.id, s.title, s.artist_id, s.album_id, s.file_path, s.genre, s.date, s.duration, 
             s.cover, s.track_number, s.favorite, s.track_lufs, s.pinned, s.date_added
             FROM playback_history ph
             JOIN songs s ON ph.song_id = s.id
             WHERE ph.event_type = 'PLAY'
             ORDER BY ph.timestamp DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_recently_added_songs(
        &self,
        limit: i32,
    ) -> Result<Vec<types::db::Song>, sqlx::Error> {
        sqlx::query_as::<_, types::db::Song>(
            "SELECT id, title, artist_id, album_id, file_path, genre, date, duration, cover, track_number, favorite, track_lufs, pinned, date_added
             FROM songs
             ORDER BY date_added DESC
             LIMIT ?"
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
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
        sqlx::query_as::<_, types::db::Artist>(
            "SELECT id, name, image, favorite, pinned FROM artists",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_artist(&self, id: &Cuid) -> Result<types::db::Artist, sqlx::Error> {
        sqlx::query_as::<_, types::db::Artist>(
            "SELECT id, name, image, favorite, pinned FROM artists WHERE id = ?",
        )
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
        let existing: Option<(Cuid,)> = sqlx::query_as(
            "SELECT id FROM albums WHERE title = ? AND (artist = ? OR (artist IS NULL AND ? IS NULL))"
        )
        .bind(title)
        .bind(artist)
        .bind(artist)
        .fetch_optional(&self.pool)
        .await?;

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
            "SELECT id, title, artist, cover, favorite, pinned FROM albums",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_album(&self, id: &Cuid) -> Result<types::db::Album, sqlx::Error> {
        sqlx::query_as::<_, types::db::Album>(
            "SELECT id, title, artist, cover, favorite, pinned FROM albums WHERE id = ?",
        )
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
            "SELECT id, name, description, image, pinned, date_updated, date_created FROM playlists"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_playlist(&self, id: &Cuid) -> Result<Playlist, sqlx::Error> {
        sqlx::query_as::<_, Playlist>(
            "SELECT id, name, description, image, pinned, date_updated, date_created FROM playlists WHERE id = ?"
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn add_song_to_playlist(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
        position: i32,
    ) -> Result<Cuid, sqlx::Error> {
        let id = Cuid::new();
        sqlx::query(
            "INSERT INTO playlist_tracks (id, playlist_id, song_id, position) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(playlist_id)
        .bind(song_id)
        .bind(position)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn remove_song_from_playlist(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ? AND song_id = ?")
            .bind(playlist_id)
            .bind(song_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_playlist_tracks(
        &self,
        playlist_id: &Cuid,
    ) -> Result<Vec<types::db::PlaylistTrack>, sqlx::Error> {
        sqlx::query_as::<_, types::db::PlaylistTrack>(
            "SELECT id, playlist_id, song_id, position FROM playlist_tracks WHERE playlist_id = ? ORDER BY position"
        )
        .bind(playlist_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_all_playlist_tracks(
        &self,
    ) -> Result<Vec<types::db::PlaylistTrack>, sqlx::Error> {
        sqlx::query_as::<_, types::db::PlaylistTrack>(
            "SELECT id, playlist_id, song_id, position FROM playlist_tracks ORDER BY position",
        )
        .fetch_all(&self.pool)
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
        sqlx::query_as::<_, EventContext>(
            "SELECT id, song_id, playlist_id, date_created FROM event_contexts WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn cleanup_orphaned_artists(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM artists WHERE id NOT IN (SELECT DISTINCT artist_id FROM songs WHERE artist_id IS NOT NULL)"
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn cleanup_orphaned_albums(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM songs WHERE album_id IS NOT NULL)"
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
