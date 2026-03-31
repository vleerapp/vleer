use crate::data::{
    db::models::*,
    models::{
        Album, AlbumListItem, Artist, ArtistListItem, Cuid, Event, EventContext, EventType, Image,
        PinnedItem, Playlist, PlaylistTrack, RecentItem, Song, SongListItem, SongSort,
    },
};
use gpui::Global;
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Database {
    pub pool: Arc<SqlitePool>,
}

impl Global for Database {}

impl Database {
    pub async fn get_song(&self, id: Cuid) -> sqlx::Result<Option<Song>> {
        Ok(sqlx::query_as::<_, SongRow>(
            r#"
            SELECT s.*, ar.name AS artist_name
            FROM songs s
            LEFT JOIN artists ar ON s.artist_id = ar.id
            WHERE s.id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&*self.pool)
        .await?
        .map(Into::into))
    }

    pub async fn get_songs_by_ids(&self, ids: &[Cuid]) -> sqlx::Result<Vec<Song>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT s.*, ar.name AS artist_name FROM songs s LEFT JOIN artists ar ON s.artist_id = ar.id WHERE s.id IN ({})",
            placeholders
        );
        let mut query = sqlx::query_as::<_, SongRow>(&sql);
        for id in ids {
            query = query.bind(id);
        }
        Ok(query
            .fetch_all(&*self.pool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn get_song_by_path(&self, file_path: &str) -> sqlx::Result<Option<Song>> {
        Ok(sqlx::query_as::<_, SongRow>(
            r#"
            SELECT s.*, ar.name AS artist_name
            FROM songs s
            LEFT JOIN artists ar ON s.artist_id = ar.id
            WHERE s.file_path = ?
            "#,
        )
        .bind(file_path)
        .fetch_optional(&*self.pool)
        .await?
        .map(Into::into))
    }

    pub async fn upsert_song(
        &self,
        title: &str,
        artist_id: Option<&Cuid>,
        album_id: Option<&Cuid>,
        file_path: &str,
        duration: i32,
        track_number: Option<i32>,
        year: Option<i32>,
        genre: Option<&str>,
        image_id: Option<&str>,
        file_size: i64,
        file_modified: i64,
        lufs: Option<f32>,
    ) -> sqlx::Result<()> {
        let year_str = year.map(|y| y.to_string());

        sqlx::query(
            "INSERT INTO songs (id, title, artist_id, album_id, file_path, file_size, file_modified, genre, date, duration, image_id, track_number, lufs)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(file_path) DO UPDATE SET
                title = excluded.title,
                artist_id = excluded.artist_id,
                album_id = excluded.album_id,
                file_size = excluded.file_size,
                file_modified = excluded.file_modified,
                genre = excluded.genre,
                date = excluded.date,
                duration = excluded.duration,
                image_id = excluded.image_id,
                track_number = excluded.track_number,
                lufs = excluded.lufs"
        )
        .bind(Cuid::new())
        .bind(title)
        .bind(artist_id)
        .bind(album_id)
        .bind(file_path)
        .bind(file_size)
        .bind(file_modified)
        .bind(genre)
        .bind(year_str)
        .bind(duration)
        .bind(image_id)
        .bind(track_number)
        .bind(lufs)
        .execute(&*self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete_song(&self, id: &Cuid) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM songs WHERE id = ?")
            .bind(id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_song_by_path(&self, file_path: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM songs WHERE file_path = ?")
            .bind(file_path)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_songs_by_paths(&self, file_paths: &[String]) -> sqlx::Result<usize> {
        if file_paths.is_empty() {
            return Ok(0);
        }

        let placeholders = std::iter::repeat_n("?", file_paths.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM songs WHERE file_path IN ({placeholders})");

        let mut tx = self.pool.begin().await?;

        let mut query = sqlx::query(&sql);
        for path in file_paths {
            query = query.bind(path);
        }

        let result = query.execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result.rows_affected() as usize)
    }

    pub async fn get_song_paths(&self) -> sqlx::Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT file_path FROM songs")
            .fetch_all(&*self.pool)
            .await?;
        Ok(rows.into_iter().map(|(p,)| p).collect())
    }

    pub async fn get_song_file_states(&self) -> sqlx::Result<Vec<(String, i64, i64)>> {
        let rows: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT file_path, file_size, file_modified FROM songs")
                .fetch_all(&*self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn get_songs_count(&self, query: Option<&str>) -> sqlx::Result<i64> {
        let Some(query) = query else {
            let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM songs")
                .fetch_one(&*self.pool)
                .await?;
            return Ok(row.0);
        };

        let query = query.trim();
        if query.is_empty() {
            let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM songs")
                .fetch_one(&*self.pool)
                .await?;
            return Ok(row.0);
        }

        let Some(fts_query) = to_fts_query(query) else {
            return Ok(0);
        };

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM (
                SELECT song_id
                FROM songs_fts
                WHERE songs_fts MATCH ?1
                GROUP BY song_id
            ) matched
            "#,
        )
        .bind(fts_query)
        .fetch_one(&*self.pool)
        .await?;

        Ok(row.0)
    }

    pub async fn get_songs(
        &self,
        query: Option<&str>,
        sort: SongSort,
        ascending: bool,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<SongListItem>> {
        let has_query = query.map(|q| !q.trim().is_empty()).unwrap_or(false);
        let order_clause = if has_query {
            song_order(sort, ascending, true)
        } else {
            song_order(sort, ascending, false)
        };

        if !has_query {
            let sql = format!(
                r#"
                SELECT
                    s.id,
                    s.title,
                    ar.name AS artist_name,
                    al.title AS album_title,
                    s.duration,
                    s.image_id
                FROM songs s
                LEFT JOIN artists ar ON s.artist_id = ar.id
                LEFT JOIN albums al ON s.album_id = al.id
                ORDER BY {order_clause}
                LIMIT ?1 OFFSET ?2
                "#
            );

            return Ok(sqlx::query_as::<_, SongListRow>(&sql)
                .bind(limit)
                .bind(offset)
                .fetch_all(&*self.pool)
                .await?
                .into_iter()
                .map(Into::into)
                .collect());
        }

        let query = query.unwrap().trim();
        let Some(fts_query) = to_fts_query(query) else {
            return Ok(Vec::new());
        };

        let sql = format!(
            r#"
            SELECT
                s.id,
                s.title,
                ar.name AS artist_name,
                al.title AS album_title,
                s.duration,
                s.image_id
            FROM songs_fts
            JOIN songs s ON s.id = songs_fts.song_id
            LEFT JOIN artists ar ON s.artist_id = ar.id
            LEFT JOIN albums al ON s.album_id = al.id
            WHERE songs_fts MATCH ?2
            GROUP BY s.id, s.title, ar.name, al.title, s.duration, s.image_id
            ORDER BY {order_clause}
            LIMIT ?3 OFFSET ?4
            "#
        );

        Ok(sqlx::query_as::<_, SongListRow>(&sql)
            .bind(query)
            .bind(fts_query)
            .bind(limit)
            .bind(offset)
            .fetch_all(&*self.pool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    pub async fn get_song_ids_from_offset(
        &self,
        query: &str,
        sort: SongSort,
        ascending: bool,
        offset: i64,
    ) -> sqlx::Result<Vec<Cuid>> {
        let query = query.trim();
        let has_query = !query.is_empty();
        let order_clause = if has_query {
            song_order(sort, ascending, true)
        } else {
            song_order(sort, ascending, false)
        };

        if !has_query {
            let sql = format!(
                r#"
                SELECT s.id
                FROM songs s
                LEFT JOIN artists ar ON s.artist_id = ar.id
                LEFT JOIN albums al ON s.album_id = al.id
                ORDER BY {order_clause}
                LIMIT -1 OFFSET ?1
                "#
            );

            let rows: Vec<(Cuid,)> = sqlx::query_as(&sql)
                .bind(offset)
                .fetch_all(&*self.pool)
                .await?;
            return Ok(rows.into_iter().map(|(id,)| id).collect());
        }

        let Some(fts_query) = to_fts_query(query) else {
            return Ok(Vec::new());
        };

        let sql = format!(
            r#"
            SELECT s.id
            FROM songs_fts
            JOIN songs s ON s.id = songs_fts.song_id
            LEFT JOIN artists ar ON s.artist_id = ar.id
            LEFT JOIN albums al ON s.album_id = al.id
            WHERE songs_fts MATCH ?2
            GROUP BY s.id, s.title, al.title, s.duration
            ORDER BY {order_clause}
            LIMIT -1 OFFSET ?3
            "#
        );

        let rows: Vec<(Cuid,)> = sqlx::query_as(&sql)
            .bind(query)
            .bind(fts_query)
            .bind(offset)
            .fetch_all(&*self.pool)
            .await?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn get_album_songs(&self, album_id: &Cuid) -> sqlx::Result<Vec<Song>> {
        Ok(sqlx::query_as::<_, SongRow>(
            r#"
            SELECT s.*, ar.name AS artist_name
            FROM songs s
            LEFT JOIN artists ar ON s.artist_id = ar.id
            WHERE s.album_id = ?
            ORDER BY s.track_number ASC
            "#,
        )
        .bind(album_id)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    pub async fn get_artist(&self, id: Cuid) -> sqlx::Result<Option<Artist>> {
        Ok(
            sqlx::query_as::<_, ArtistRow>("SELECT * FROM artists WHERE id = ?")
                .bind(id)
                .fetch_optional(&*self.pool)
                .await?
                .map(Into::into),
        )
    }

    pub async fn upsert_artist(&self, name: &str) -> sqlx::Result<Cuid> {
        sqlx::query_scalar(
            "INSERT INTO artists (id, name) VALUES (?, ?)
             ON CONFLICT(name) DO UPDATE SET name = excluded.name
             RETURNING id",
        )
        .bind(Cuid::new())
        .bind(name)
        .fetch_one(&*self.pool)
        .await
    }

    pub async fn get_artists_count(&self, query: &str) -> sqlx::Result<usize> {
        let query = query.trim();
        if query.is_empty() {
            let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM artists")
                .fetch_one(&*self.pool)
                .await?;
            return Ok(row.0 as usize);
        }

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM artists ar
            WHERE
                ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            "#,
        )
        .bind(query)
        .fetch_one(&*self.pool)
        .await?;

        Ok(row.0 as usize)
    }

    pub async fn get_artists(
        &self,
        query: &str,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<ArtistListItem>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(sqlx::query_as::<_, ArtistListRow>(
                r#"
                SELECT ar.id, ar.name, ar.image_id
                FROM artists ar
                ORDER BY ar.name COLLATE NOCASE ASC
                LIMIT ?1 OFFSET ?2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&*self.pool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect());
        }

        Ok(sqlx::query_as::<_, ArtistListRow>(
            r#"
            SELECT ar.id, ar.name, ar.image_id
            FROM artists ar
            WHERE ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            ORDER BY ar.name COLLATE NOCASE ASC
            LIMIT ?2 OFFSET ?3
            "#,
        )
        .bind(query)
        .bind(limit)
        .bind(offset)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    pub async fn get_album(&self, id: Cuid) -> sqlx::Result<Option<Album>> {
        Ok(
            sqlx::query_as::<_, AlbumRow>("SELECT * FROM albums WHERE id = ?")
                .bind(id)
                .fetch_optional(&*self.pool)
                .await?
                .map(Into::into),
        )
    }

    pub async fn get_albums_count(&self, query: &str) -> sqlx::Result<usize> {
        let query = query.trim();
        if query.is_empty() {
            let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM albums")
                .fetch_one(&*self.pool)
                .await?;
            return Ok(row.0 as usize);
        }

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM albums al
            WHERE
                al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
                OR EXISTS (
                    SELECT 1
                    FROM artists ar
                    WHERE
                        ar.id = al.artist_id
                        AND ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
                )
            "#,
        )
        .bind(query)
        .fetch_one(&*self.pool)
        .await?;

        Ok(row.0 as usize)
    }

    pub async fn get_albums(
        &self,
        query: &str,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<AlbumListItem>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(sqlx::query_as::<_, AlbumListRow>(
                r#"
                SELECT
                    al.id,
                    al.title,
                    ar.name AS artist_name,
                    al.image_id,
                    MIN(s.date) AS year
                FROM albums al
                LEFT JOIN artists ar ON al.artist_id = ar.id
                LEFT JOIN songs s ON s.album_id = al.id
                GROUP BY al.id, al.title, ar.name, al.image_id
                ORDER BY al.title COLLATE NOCASE ASC
                LIMIT ?1 OFFSET ?2
                "#,
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&*self.pool)
            .await?
            .into_iter()
            .map(Into::into)
            .collect());
        }

        Ok(sqlx::query_as::<_, AlbumListRow>(
            r#"
            SELECT
                al.id,
                al.title,
                ar.name AS artist_name,
                al.image_id,
                MIN(s.date) AS year
            FROM albums al
            LEFT JOIN artists ar ON al.artist_id = ar.id
            LEFT JOIN songs s ON s.album_id = al.id
            WHERE
                al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
                OR EXISTS (
                    SELECT 1
                    FROM artists ar2
                    WHERE
                        ar2.id = al.artist_id
                        AND ar2.name LIKE '%' || ?1 || '%' COLLATE NOCASE
                )
            GROUP BY al.id, al.title, ar.name, al.image_id
            ORDER BY al.title COLLATE NOCASE ASC
            LIMIT ?2 OFFSET ?3
            "#,
        )
        .bind(query)
        .bind(limit)
        .bind(offset)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    pub async fn upsert_album(
        &self,
        title: &str,
        artist_id: Option<&Cuid>,
        image_id: Option<&str>,
    ) -> sqlx::Result<Cuid> {
        sqlx::query_scalar(
            "INSERT INTO albums (id, title, artist_id, image_id)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(title, artist_id) DO UPDATE SET
                image_id = COALESCE(excluded.image_id, albums.image_id)
             RETURNING id",
        )
        .bind(Cuid::new())
        .bind(title)
        .bind(artist_id)
        .bind(image_id)
        .fetch_one(&*self.pool)
        .await
    }

    pub async fn delete_album(&self, id: &Cuid) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM albums WHERE id = ?")
            .bind(id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_playlist(&self, id: &Cuid) -> sqlx::Result<Option<Playlist>> {
        Ok(
            sqlx::query_as::<_, PlaylistRow>("SELECT * FROM playlists WHERE id = ?")
                .bind(id)
                .fetch_optional(&*self.pool)
                .await?
                .map(Into::into),
        )
    }

    pub async fn upsert_playlist(
        &self,
        id: &Cuid,
        name: &str,
        description: Option<&str>,
        image_id: Option<&str>,
        pinned: bool,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO playlists (id, name, description, image_id, pinned)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                image_id = excluded.image_id,
                pinned = excluded.pinned,
                date_updated = DATETIME('now')",
        )
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(image_id)
        .bind(pinned)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &Cuid) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM playlists WHERE id = ?")
            .bind(id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_playlist_song(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
        position: i32,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO playlist_tracks (id, playlist_id, song_id, position)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(playlist_id, song_id) DO UPDATE SET
                position = excluded.position",
        )
        .bind(Cuid::new())
        .bind(playlist_id)
        .bind(song_id)
        .bind(position)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_playlist_song(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
    ) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ? AND song_id = ?")
            .bind(playlist_id)
            .bind(song_id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn clear_playlist(&self, playlist_id: &Cuid) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ?")
            .bind(playlist_id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_playlist_songs(&self, playlist_id: &Cuid) -> sqlx::Result<Vec<PlaylistTrack>> {
        Ok(sqlx::query(
            r#"
            SELECT
                pt.id,
                pt.playlist_id,
                pt.position,
                s.*,
                ar.name AS artist_name
            FROM playlist_tracks pt
            JOIN songs s ON s.id = pt.song_id
            LEFT JOIN artists ar ON s.artist_id = ar.id
            WHERE pt.playlist_id = ?
            ORDER BY pt.position ASC
            "#,
        )
        .bind(playlist_id)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(|row| PlaylistTrackRow::from_row(&row).map(Into::into))
        .collect::<Result<_, _>>()?)
    }

    pub async fn get_event(&self, id: &Cuid) -> sqlx::Result<Option<Event>> {
        Ok(
            sqlx::query_as::<_, EventRow>("SELECT * FROM events WHERE id = ?")
                .bind(id)
                .fetch_optional(&*self.pool)
                .await?
                .map(Into::into),
        )
    }

    pub async fn insert_event(
        &self,
        event_type: EventType,
        context_id: Option<&Cuid>,
    ) -> sqlx::Result<Cuid> {
        let id = Cuid::new();
        let event_type_str = match event_type {
            EventType::Play => "PLAY",
            EventType::Stop => "STOP",
            EventType::Pause => "PAUSE",
            EventType::Resume => "RESUME",
        };

        sqlx::query("INSERT INTO events (id, event_type, context_id) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(event_type_str)
            .bind(context_id)
            .execute(&*self.pool)
            .await?;

        Ok(id)
    }

    pub async fn get_events_by_type(&self, event_type: EventType) -> sqlx::Result<Vec<Event>> {
        let event_type_str = match event_type {
            EventType::Play => "PLAY",
            EventType::Stop => "STOP",
            EventType::Pause => "PAUSE",
            EventType::Resume => "RESUME",
        };

        Ok(sqlx::query_as::<_, EventRow>(
            "SELECT * FROM events WHERE event_type = ? ORDER BY timestamp DESC",
        )
        .bind(event_type_str)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    pub async fn insert_event_context(
        &self,
        song_id: Option<&Cuid>,
        playlist_id: Option<&Cuid>,
    ) -> sqlx::Result<Cuid> {
        let id = Cuid::new();
        sqlx::query("INSERT INTO event_contexts (id, song_id, playlist_id) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(song_id)
            .bind(playlist_id)
            .execute(&*self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get_event_context(&self, id: &Cuid) -> sqlx::Result<Option<EventContext>> {
        Ok(
            sqlx::query_as::<_, EventContextRow>("SELECT * FROM event_contexts WHERE id = ?")
                .bind(id)
                .fetch_optional(&*self.pool)
                .await?
                .map(Into::into),
        )
    }

    pub async fn get_event_context_by_song(
        &self,
        song_id: &Cuid,
    ) -> sqlx::Result<Vec<EventContext>> {
        Ok(
            sqlx::query_as::<_, EventContextRow>("SELECT * FROM event_contexts WHERE song_id = ?")
                .bind(song_id)
                .fetch_all(&*self.pool)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
        )
    }

    pub async fn get_event_context_by_playlist(
        &self,
        playlist_id: &Cuid,
    ) -> sqlx::Result<Vec<EventContext>> {
        Ok(sqlx::query_as::<_, EventContextRow>(
            "SELECT * FROM event_contexts WHERE playlist_id = ?",
        )
        .bind(playlist_id)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    pub async fn set_favorite<T: Toggleable>(&self, id: &Cuid, favorite: bool) -> sqlx::Result<()> {
        let sql = format!(
            "UPDATE {} SET favorite = ? WHERE {} = ?",
            T::TABLE,
            T::ID_COL
        );
        sqlx::query(&sql)
            .bind(favorite)
            .bind(id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_pinned<T: Toggleable>(&self, id: &Cuid, pinned: bool) -> sqlx::Result<()> {
        let sql = format!("UPDATE {} SET pinned = ? WHERE {} = ?", T::TABLE, T::ID_COL);
        sqlx::query(&sql)
            .bind(pinned)
            .bind(id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn search_library(
        &self,
        query: &str,
        limit: i64,
    ) -> sqlx::Result<Vec<(Cuid, String, Option<String>, String)>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let Some(fts_query) = to_fts_query(query) else {
            return Ok(Vec::new());
        };

        let per_type_limit = (limit.saturating_mul(2)).max(20);

        Ok(sqlx::query_as::<_, SearchResultRow>(
            r#"
            WITH
            search_params AS (
                SELECT ?1 AS query_text
            ),
            song_matches AS (
                SELECT DISTINCT
                    s.id,
                    s.title AS name,
                    s.image_id AS image,
                    'Song' AS item_type,
                    CASE
                        WHEN s.title = sp.query_text COLLATE NOCASE THEN 400
                        WHEN s.title LIKE sp.query_text || '%' COLLATE NOCASE THEN 300
                        WHEN EXISTS (
                            SELECT 1
                            FROM artists ar
                            WHERE
                                ar.id = s.artist_id
                                AND ar.name LIKE sp.query_text || '%' COLLATE NOCASE
                        ) THEN 220
                        WHEN EXISTS (
                            SELECT 1
                            FROM albums al
                            WHERE
                                al.id = s.album_id
                                AND al.title LIKE sp.query_text || '%' COLLATE NOCASE
                        ) THEN 200
                        ELSE 100
                    END AS score
                FROM songs_fts
                JOIN songs s ON s.id = songs_fts.song_id
                CROSS JOIN search_params sp
                WHERE songs_fts MATCH ?2
                ORDER BY score DESC, s.title COLLATE NOCASE ASC
                LIMIT ?3
            ),
            album_matches AS (
                SELECT
                    al.id,
                    al.title AS name,
                    al.image_id AS image,
                    'Album' AS item_type,
                    CASE
                        WHEN al.title = sp.query_text COLLATE NOCASE THEN 350
                        WHEN al.title LIKE sp.query_text || '%' COLLATE NOCASE THEN 260
                        WHEN EXISTS (
                            SELECT 1
                            FROM artists ar
                            WHERE
                                ar.id = al.artist_id
                                AND ar.name LIKE sp.query_text || '%' COLLATE NOCASE
                        ) THEN 180
                        ELSE 90
                    END AS score
                FROM albums al
                CROSS JOIN search_params sp
                WHERE
                    al.title LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                    OR EXISTS (
                        SELECT 1
                        FROM artists ar
                        WHERE
                            ar.id = al.artist_id
                            AND ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                    )
                ORDER BY score DESC, al.title COLLATE NOCASE ASC
                LIMIT ?3
            ),
            artist_matches AS (
                SELECT
                    ar.id,
                    ar.name AS name,
                    ar.image_id AS image,
                    'Artist' AS item_type,
                    CASE
                        WHEN ar.name = sp.query_text COLLATE NOCASE THEN 320
                        WHEN ar.name LIKE sp.query_text || '%' COLLATE NOCASE THEN 250
                        ELSE 80
                    END AS score
                FROM artists ar
                CROSS JOIN search_params sp
                WHERE
                    ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                ORDER BY score DESC, ar.name COLLATE NOCASE ASC
                LIMIT ?3
            ),
            playlist_matches AS (
                SELECT
                    p.id,
                    p.name AS name,
                    p.image_id AS image,
                    'Playlist' AS item_type,
                    CASE
                        WHEN p.name = sp.query_text COLLATE NOCASE THEN 300
                        WHEN p.name LIKE sp.query_text || '%' COLLATE NOCASE THEN 240
                        ELSE 70
                    END AS score
                FROM playlists p
                CROSS JOIN search_params sp
                WHERE
                    p.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                ORDER BY score DESC, p.name COLLATE NOCASE ASC
                LIMIT ?3
            ),
            all_matches AS (
                SELECT * FROM song_matches
                UNION ALL
                SELECT * FROM album_matches
                UNION ALL
                SELECT * FROM artist_matches
                UNION ALL
                SELECT * FROM playlist_matches
            )
            SELECT id, name, image, item_type
            FROM all_matches
            ORDER BY score DESC, name COLLATE NOCASE ASC
            LIMIT ?4
            "#,
        )
        .bind(query)
        .bind(fts_query)
        .bind(per_type_limit)
        .bind(limit)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
    }

    pub async fn get_search_match_counts(
        &self,
        query: &str,
    ) -> sqlx::Result<(usize, usize, usize, usize)> {
        let (songs, albums, artists, playlists) = tokio::try_join!(
            self.get_songs_count(Some(query)),
            self.get_albums_count(query),
            self.get_artists_count(query),
            self.get_playlists_count(query),
        )?;
        Ok((songs as usize, albums, artists, playlists as usize))
    }

    pub async fn get_playlists_count(&self, query: &str) -> sqlx::Result<i64> {
        let query = query.trim();
        if query.is_empty() {
            let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM playlists")
                .fetch_one(&*self.pool)
                .await?;
            return Ok(row.0);
        }

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM playlists p
            WHERE p.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            "#,
        )
        .bind(query)
        .fetch_one(&*self.pool)
        .await?;

        Ok(row.0)
    }

    pub async fn upsert_image(&self, id: &str, data: &[u8]) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO images (id, data)
             VALUES (?, ?)
             ON CONFLICT(id) DO UPDATE SET
                data = excluded.data,
                date_updated = DATETIME('now')",
        )
        .bind(id)
        .bind(data)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_image(&self, id: &str) -> sqlx::Result<Option<Image>> {
        Ok(
            sqlx::query_as::<_, ImageRow>("SELECT * FROM images WHERE id = ?")
                .bind(id)
                .fetch_optional(&*self.pool)
                .await?
                .map(Into::into),
        )
    }

    pub async fn delete_image(&self, id: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM images WHERE id = ?")
            .bind(id)
            .execute(&*self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_recently_added_items(&self, limit: i64) -> sqlx::Result<Vec<RecentItem>> {
        Ok(sqlx::query_as::<_, RecentItemRow>(
            r#"
            WITH recent_songs AS (
                SELECT
                    s.id,
                    s.title,
                    s.artist_id,
                    s.album_id,
                    s.image_id,
                    s.date_added,
                    s.date
                FROM songs s
                ORDER BY s.date_added DESC
                LIMIT ?
            ),
            image_groups AS (
                SELECT
                    COALESCE(rs.image_id, 'no_image_' || rs.id) as group_key,
                    rs.image_id,
                    rs.album_id,
                    MAX(rs.date_added) as most_recent_date,
                    COUNT(*) as song_count,
                    MIN(rs.id) as first_song_id
                FROM recent_songs rs
                GROUP BY group_key, rs.image_id, rs.album_id
            )
            SELECT
                ig.most_recent_date,
                ig.song_count,
                ig.first_song_id,
                s.title as first_song_title,
                s.artist_id as first_artist_id,
                ig.image_id,
                s.date as first_year,
                ig.album_id,
                al.title as album_title,
                ar.name as artist_name
            FROM image_groups ig
            JOIN songs s ON ig.first_song_id = s.id
            LEFT JOIN albums al ON ig.album_id = al.id
            LEFT JOIN artists ar ON s.artist_id = ar.id
            LEFT JOIN images img ON ig.image_id = img.id
            ORDER BY ig.most_recent_date DESC
            "#,
        )
        .bind(limit)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(|row| row.into_recent_item())
        .collect())
    }

    pub async fn get_recently_played_items(&self, limit: i64) -> sqlx::Result<Vec<RecentItem>> {
        Ok(sqlx::query_as::<_, RecentItemRow>(
            r#"
            WITH recent_song_plays AS (
                SELECT
                    ec.song_id,
                    MAX(e.timestamp) AS most_recent_date
                FROM events e
                JOIN event_contexts ec ON e.context_id = ec.id
                WHERE e.event_type = ?1
                    AND ec.song_id IS NOT NULL
                GROUP BY ec.song_id
                ORDER BY most_recent_date DESC
                LIMIT ?2
            )
            SELECT
                1 AS song_count,
                s.id AS first_song_id,
                s.title AS first_song_title,
                s.image_id,
                s.date AS first_year,
                s.album_id,
                al.title AS album_title,
                ar.name AS artist_name
            FROM recent_song_plays rsp
            JOIN songs s ON rsp.song_id = s.id
            LEFT JOIN albums al ON s.album_id = al.id
            LEFT JOIN artists ar ON s.artist_id = ar.id
            ORDER BY rsp.most_recent_date DESC
            "#,
        )
        .bind("PLAY")
        .bind(limit)
        .fetch_all(&*self.pool)
        .await?
        .into_iter()
        .map(|row| row.into_recent_item())
        .collect())
    }

    pub async fn get_pinned_items(&self) -> Vec<PinnedItem> {
        sqlx::query_as::<_, PinnedItemRow>(
            r#"
            SELECT id, title AS name, image_id, 'Song' AS item_type
            FROM songs WHERE pinned = TRUE
            UNION ALL
            SELECT id, title AS name, image_id, 'Album' AS item_type
            FROM albums WHERE pinned = TRUE
            UNION ALL
            SELECT id, name AS name, image_id, 'Artist' AS item_type
            FROM artists WHERE pinned = TRUE
            UNION ALL
            SELECT id, name AS name, image_id, 'Playlist' AS item_type
            FROM playlists WHERE pinned = TRUE
            ORDER BY name COLLATE NOCASE
            "#,
        )
        .fetch_all(&*self.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(Into::into)
        .collect()
    }
}

fn to_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '_')
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect();

    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

fn song_order(sort: SongSort, ascending: bool, has_query: bool) -> &'static str {
    match sort {
        SongSort::Title => {
            if ascending {
                "s.title COLLATE NOCASE ASC, s.id ASC"
            } else {
                "s.title COLLATE NOCASE DESC, s.id ASC"
            }
        }
        SongSort::Album => {
            if ascending {
                "COALESCE(al.title, '') COLLATE NOCASE ASC, s.id ASC"
            } else {
                "COALESCE(al.title, '') COLLATE NOCASE DESC, s.id ASC"
            }
        }
        SongSort::Duration => {
            if ascending {
                "s.duration ASC, s.id ASC"
            } else {
                "s.duration DESC, s.id ASC"
            }
        }
        SongSort::Default => {
            if has_query {
                r#"
                CASE
                    WHEN s.title = ?1 COLLATE NOCASE THEN 400
                    WHEN s.title LIKE ?1 || '%' COLLATE NOCASE THEN 300
                    WHEN EXISTS (
                        SELECT 1
                        FROM artists ar3
                        WHERE
                            ar3.id = s.artist_id
                            AND ar3.name LIKE ?1 || '%' COLLATE NOCASE
                    ) THEN 220
                    WHEN EXISTS (
                        SELECT 1
                        FROM albums al3
                        WHERE
                            al3.id = s.album_id
                            AND al3.title LIKE ?1 || '%' COLLATE NOCASE
                    ) THEN 200
                    ELSE 100
                END DESC,
                s.title COLLATE NOCASE ASC,
                s.id ASC
                "#
            } else {
                "s.date_added DESC, s.id ASC"
            }
        }
    }
}
