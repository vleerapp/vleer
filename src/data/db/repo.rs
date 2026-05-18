use crate::data::{
    db::models::*,
    models::{
        Album, AlbumListItem, Artist, ArtistListItem, Cuid, Event, EventContext, EventType, Image,
        PinnedItem, Playlist, PlaylistTrack, RecentItem, Song, SongListItem, SongSort,
    },
};
use anyhow::Result;
use gpui::Global;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{OptionalExtension, ToSql, params};
use std::path::Path;

fn build_pool(
    path: &Path,
    max_size: u32,
    busy_timeout_ms: u32,
) -> Result<Pool<SqliteConnectionManager>> {
    let manager = SqliteConnectionManager::file(path).with_init(move |c| {
        c.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = {busy_timeout_ms};
             PRAGMA auto_vacuum = FULL;"
        ))
    });
    Ok(Pool::builder().max_size(max_size).build(manager)?)
}

fn collect_mapped<T, U, F>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
    mapper: F,
) -> Result<Vec<U>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    T: Into<U>,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params, mapper)?
        .collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(Clone)]
pub struct Database {
    pub pool: Pool<SqliteConnectionManager>,
    pub image_pool: Pool<SqliteConnectionManager>,
}

impl Global for Database {}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let pool = build_pool(path, 8, 3000)?;
        let image_pool = build_pool(path, 4, 5000)?;
        Ok(Self { pool, image_pool })
    }

    pub fn get_song(&self, id: &Cuid) -> Result<Option<Song>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT s.*, ar.name AS artist_name
                 FROM songs s
                 LEFT JOIN artists ar ON s.artist_id = ar.id
                 WHERE s.id = ?1",
                params![id],
                SongRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn get_songs_by_ids(&self, ids: &[Cuid]) -> Result<Vec<Song>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT s.*, ar.name AS artist_name
             FROM songs s
             LEFT JOIN artists ar ON s.artist_id = ar.id
             WHERE s.id IN ({placeholders})"
        );
        let conn = self.pool.get()?;
        let params: Vec<&dyn ToSql> = ids.iter().map(|id| id as &dyn ToSql).collect();
        collect_mapped::<SongRow, Song, _>(&conn, &sql, params.as_slice(), SongRow::from_row)
    }

    pub fn get_song_by_path(&self, file_path: &str) -> Result<Option<Song>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT s.*, ar.name AS artist_name
                 FROM songs s
                 LEFT JOIN artists ar ON s.artist_id = ar.id
                 WHERE s.file_path = ?1",
                params![file_path],
                SongRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_song(
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
    ) -> Result<()> {
        let year_str = year.map(|y| y.to_string());
        let id = Cuid::new();
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO songs (id, title, artist_id, album_id, file_path, file_size, file_modified, genre, date, duration, image_id, track_number, lufs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
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
                lufs = excluded.lufs",
            params![
                id,
                title,
                artist_id,
                album_id,
                file_path,
                file_size,
                file_modified,
                genre,
                year_str,
                duration,
                image_id,
                track_number,
                lufs,
            ],
        )?;
        Ok(())
    }

    pub fn delete_song(&self, id: &Cuid) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM songs WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_song_by_path(&self, file_path: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM songs WHERE file_path = ?1", params![file_path])?;
        Ok(())
    }

    pub fn delete_songs_by_paths(&self, file_paths: &[String]) -> Result<usize> {
        if file_paths.is_empty() {
            return Ok(0);
        }

        let placeholders = (1..=file_paths.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM songs WHERE file_path IN ({placeholders})");

        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let params: Vec<&dyn ToSql> = file_paths.iter().map(|p| p as &dyn ToSql).collect();
        let count = tx.execute(&sql, params.as_slice())?;
        tx.commit()?;
        Ok(count)
    }

    pub fn get_song_paths(&self) -> Result<Vec<String>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT file_path FROM songs")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_song_file_states(&self) -> Result<Vec<(String, i64, i64)>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT file_path, file_size, file_modified FROM songs")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_songs_count(&self, query: Option<&str>) -> Result<i64> {
        let conn = self.pool.get()?;
        let trimmed = query.map(|q| q.trim()).filter(|q| !q.is_empty());

        let Some(query) = trimmed else {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM songs", [], |row| row.get(0))?;
            return Ok(count);
        };

        let Some(fts_query) = to_fts_query(query) else {
            return Ok(0);
        };

        let count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM (
                 SELECT song_id
                 FROM songs_fts
                 WHERE songs_fts MATCH ?1
                 GROUP BY song_id
             ) matched",
            params![fts_query],
            |row| row.get(0),
        )?;

        Ok(count)
    }

    pub fn get_songs(
        &self,
        query: Option<&str>,
        sort: SongSort,
        ascending: bool,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<SongListItem>> {
        let has_query = query.map(|q| !q.trim().is_empty()).unwrap_or(false);
        let order_clause = song_order(sort, ascending, has_query);
        let conn = self.pool.get()?;

        if !has_query {
            let sql = format!(
                "SELECT s.id, s.title, ar.name AS artist_name, al.title AS album_title,
                        s.duration, s.image_id
                 FROM songs s
                 LEFT JOIN artists ar ON s.artist_id = ar.id
                 LEFT JOIN albums al ON s.album_id = al.id
                 ORDER BY {order_clause}
                 LIMIT ?1 OFFSET ?2"
            );
            return collect_mapped::<SongListRow, SongListItem, _>(
                &conn,
                &sql,
                params![limit, offset],
                SongListRow::from_row,
            );
        }

        let query = query.unwrap().trim();
        let Some(fts_query) = to_fts_query(query) else {
            return Ok(Vec::new());
        };

        let sql = format!(
            "SELECT s.id, s.title, ar.name AS artist_name, al.title AS album_title,
                    s.duration, s.image_id
             FROM songs_fts
             JOIN songs s ON s.id = songs_fts.song_id
             LEFT JOIN artists ar ON s.artist_id = ar.id
             LEFT JOIN albums al ON s.album_id = al.id
             WHERE songs_fts MATCH ?2
             GROUP BY s.id, s.title, ar.name, al.title, s.duration, s.image_id
             ORDER BY {order_clause}
             LIMIT ?3 OFFSET ?4"
        );

        collect_mapped::<SongListRow, SongListItem, _>(
            &conn,
            &sql,
            params![query, fts_query, limit, offset],
            SongListRow::from_row,
        )
    }

    pub fn get_song_ids_from_offset(
        &self,
        query: &str,
        sort: SongSort,
        ascending: bool,
        offset: i64,
    ) -> Result<Vec<Cuid>> {
        let query = query.trim();
        let has_query = !query.is_empty();
        let order_clause = song_order(sort, ascending, has_query);
        let conn = self.pool.get()?;

        if !has_query {
            let sql = format!(
                "SELECT s.id
                 FROM songs s
                 LEFT JOIN artists ar ON s.artist_id = ar.id
                 LEFT JOIN albums al ON s.album_id = al.id
                 ORDER BY {order_clause}
                 LIMIT -1 OFFSET ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![offset], |row| row.get::<_, Cuid>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(rows);
        }

        let Some(fts_query) = to_fts_query(query) else {
            return Ok(Vec::new());
        };

        let sql = format!(
            "SELECT s.id
             FROM songs_fts
             JOIN songs s ON s.id = songs_fts.song_id
             LEFT JOIN artists ar ON s.artist_id = ar.id
             LEFT JOIN albums al ON s.album_id = al.id
             WHERE songs_fts MATCH ?2
             GROUP BY s.id, s.title, al.title, s.duration
             ORDER BY {order_clause}
             LIMIT -1 OFFSET ?3"
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![query, fts_query, offset], |row| {
                row.get::<_, Cuid>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_album_songs(&self, album_id: &Cuid) -> Result<Vec<Song>> {
        let conn = self.pool.get()?;
        collect_mapped::<SongRow, Song, _>(
            &conn,
            "SELECT s.*, ar.name AS artist_name
             FROM songs s
             LEFT JOIN artists ar ON s.artist_id = ar.id
             WHERE s.album_id = ?1
             ORDER BY s.track_number ASC",
            params![album_id],
            SongRow::from_row,
        )
    }

    pub fn get_artist(&self, id: &Cuid) -> Result<Option<Artist>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT * FROM artists WHERE id = ?1",
                params![id],
                ArtistRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn upsert_artist(&self, name: &str) -> Result<Cuid> {
        let id = Cuid::new();
        let conn = self.pool.get()?;
        let result_id: Cuid = conn.query_row(
            "INSERT INTO artists (id, name) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET name = excluded.name
             RETURNING id",
            params![id, name],
            |row| row.get(0),
        )?;
        Ok(result_id)
    }

    pub fn get_artists_count(&self, query: &str) -> Result<usize> {
        let conn = self.pool.get()?;
        let query = query.trim();
        if query.is_empty() {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM artists", [], |row| row.get(0))?;
            return Ok(count as usize);
        }

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM artists ar
             WHERE ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE",
            params![query],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn get_artists(&self, query: &str, offset: i64, limit: i64) -> Result<Vec<ArtistListItem>> {
        let conn = self.pool.get()?;
        let query = query.trim();
        if query.is_empty() {
            return collect_mapped::<ArtistListRow, ArtistListItem, _>(
                &conn,
                "SELECT ar.id, ar.name, ar.image_id
                 FROM artists ar
                 ORDER BY ar.name COLLATE NOCASE ASC
                 LIMIT ?1 OFFSET ?2",
                params![limit, offset],
                ArtistListRow::from_row,
            );
        }

        collect_mapped::<ArtistListRow, ArtistListItem, _>(
            &conn,
            "SELECT ar.id, ar.name, ar.image_id
             FROM artists ar
             WHERE ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
             ORDER BY ar.name COLLATE NOCASE ASC
             LIMIT ?2 OFFSET ?3",
            params![query, limit, offset],
            ArtistListRow::from_row,
        )
    }

    pub fn get_album(&self, id: &Cuid) -> Result<Option<Album>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT * FROM albums WHERE id = ?1",
                params![id],
                AlbumRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn get_albums_count(&self, query: &str) -> Result<usize> {
        let conn = self.pool.get()?;
        let query = query.trim();
        if query.is_empty() {
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM albums", [], |row| row.get(0))?;
            return Ok(count as usize);
        }

        let count: i64 = conn.query_row(
            "SELECT COUNT(*)
             FROM albums al
             WHERE
                 al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
                 OR EXISTS (
                     SELECT 1
                     FROM artists ar
                     WHERE ar.id = al.artist_id
                       AND ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
                 )",
            params![query],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn get_albums(&self, query: &str, offset: i64, limit: i64) -> Result<Vec<AlbumListItem>> {
        let conn = self.pool.get()?;
        let query = query.trim();
        if query.is_empty() {
            return collect_mapped::<AlbumListRow, AlbumListItem, _>(
                &conn,
                "SELECT al.id, al.title, ar.name AS artist_name, al.image_id,
                        MIN(s.date) AS year
                 FROM albums al
                 LEFT JOIN artists ar ON al.artist_id = ar.id
                 LEFT JOIN songs s ON s.album_id = al.id
                 GROUP BY al.id, al.title, ar.name, al.image_id
                 ORDER BY al.title COLLATE NOCASE ASC
                 LIMIT ?1 OFFSET ?2",
                params![limit, offset],
                AlbumListRow::from_row,
            );
        }

        collect_mapped::<AlbumListRow, AlbumListItem, _>(
            &conn,
            "SELECT al.id, al.title, ar.name AS artist_name, al.image_id,
                    MIN(s.date) AS year
             FROM albums al
             LEFT JOIN artists ar ON al.artist_id = ar.id
             LEFT JOIN songs s ON s.album_id = al.id
             WHERE
                 al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
                 OR EXISTS (
                     SELECT 1
                     FROM artists ar2
                     WHERE ar2.id = al.artist_id
                       AND ar2.name LIKE '%' || ?1 || '%' COLLATE NOCASE
                 )
             GROUP BY al.id, al.title, ar.name, al.image_id
             ORDER BY al.title COLLATE NOCASE ASC
             LIMIT ?2 OFFSET ?3",
            params![query, limit, offset],
            AlbumListRow::from_row,
        )
    }

    pub fn upsert_album(
        &self,
        title: &str,
        artist_id: Option<&Cuid>,
        image_id: Option<&str>,
    ) -> Result<Cuid> {
        let id = Cuid::new();
        let conn = self.pool.get()?;
        let result_id: Cuid = conn.query_row(
            "INSERT INTO albums (id, title, artist_id, image_id)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(title, artist_id) DO UPDATE SET
                image_id = COALESCE(excluded.image_id, albums.image_id)
             RETURNING id",
            params![id, title, artist_id, image_id],
            |row| row.get(0),
        )?;
        Ok(result_id)
    }

    pub fn delete_album(&self, id: &Cuid) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM albums WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_playlist(&self, id: &Cuid) -> Result<Option<Playlist>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT * FROM playlists WHERE id = ?1",
                params![id],
                PlaylistRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    #[allow(dead_code)]
    pub fn upsert_playlist(
        &self,
        id: &Cuid,
        name: &str,
        description: Option<&str>,
        image_id: Option<&str>,
        pinned: bool,
    ) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO playlists (id, name, description, image_id, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                image_id = excluded.image_id,
                pinned = excluded.pinned,
                date_updated = DATETIME('now')",
            params![id, name, description, image_id, pinned],
        )?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: &Cuid) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn upsert_playlist_song(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
        position: i32,
    ) -> Result<()> {
        let id = Cuid::new();
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO playlist_tracks (id, playlist_id, song_id, position)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(playlist_id, song_id) DO UPDATE SET
                position = excluded.position",
            params![id, playlist_id, song_id, position],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn delete_playlist_song(&self, playlist_id: &Cuid, song_id: &Cuid) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND song_id = ?2",
            params![playlist_id, song_id],
        )?;
        Ok(())
    }

    pub fn clear_playlist(&self, playlist_id: &Cuid) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
        Ok(())
    }

    pub fn get_playlist_songs(&self, playlist_id: &Cuid) -> Result<Vec<PlaylistTrack>> {
        let conn = self.pool.get()?;
        collect_mapped::<PlaylistTrackRow, PlaylistTrack, _>(
            &conn,
            "SELECT pt.id, pt.playlist_id, pt.position, s.*, ar.name AS artist_name
             FROM playlist_tracks pt
             JOIN songs s ON s.id = pt.song_id
             LEFT JOIN artists ar ON s.artist_id = ar.id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position ASC",
            params![playlist_id],
            PlaylistTrackRow::from_row,
        )
    }

    #[allow(dead_code)]
    pub fn get_event(&self, id: &Cuid) -> Result<Option<Event>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT * FROM events WHERE id = ?1",
                params![id],
                EventRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn insert_event(&self, event_type: EventType, context_id: Option<&Cuid>) -> Result<Cuid> {
        let id = Cuid::new();
        let event_type_str = match event_type {
            EventType::Play => "PLAY",
            EventType::Stop => "STOP",
            EventType::Pause => "PAUSE",
            EventType::Resume => "RESUME",
        };

        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO events (id, event_type, context_id) VALUES (?1, ?2, ?3)",
            params![id, event_type_str, context_id],
        )?;

        Ok(id)
    }

    #[allow(dead_code)]
    pub fn get_events_by_type(&self, event_type: EventType) -> Result<Vec<Event>> {
        let event_type_str = match event_type {
            EventType::Play => "PLAY",
            EventType::Stop => "STOP",
            EventType::Pause => "PAUSE",
            EventType::Resume => "RESUME",
        };
        let conn = self.pool.get()?;
        collect_mapped::<EventRow, Event, _>(
            &conn,
            "SELECT * FROM events WHERE event_type = ?1 ORDER BY timestamp DESC",
            params![event_type_str],
            EventRow::from_row,
        )
    }

    pub fn insert_event_context(
        &self,
        song_id: Option<&Cuid>,
        playlist_id: Option<&Cuid>,
    ) -> Result<Cuid> {
        let id = Cuid::new();
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO event_contexts (id, song_id, playlist_id) VALUES (?1, ?2, ?3)",
            params![id, song_id, playlist_id],
        )?;
        Ok(id)
    }

    #[allow(dead_code)]
    pub fn get_event_context(&self, id: &Cuid) -> Result<Option<EventContext>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT * FROM event_contexts WHERE id = ?1",
                params![id],
                EventContextRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    #[allow(dead_code)]
    pub fn get_event_context_by_song(&self, song_id: &Cuid) -> Result<Vec<EventContext>> {
        let conn = self.pool.get()?;
        collect_mapped::<EventContextRow, EventContext, _>(
            &conn,
            "SELECT * FROM event_contexts WHERE song_id = ?1",
            params![song_id],
            EventContextRow::from_row,
        )
    }

    #[allow(dead_code)]
    pub fn get_event_context_by_playlist(&self, playlist_id: &Cuid) -> Result<Vec<EventContext>> {
        let conn = self.pool.get()?;
        collect_mapped::<EventContextRow, EventContext, _>(
            &conn,
            "SELECT * FROM event_contexts WHERE playlist_id = ?1",
            params![playlist_id],
            EventContextRow::from_row,
        )
    }

    pub fn set_favorite<T: Toggleable>(&self, id: &Cuid, favorite: bool) -> Result<()> {
        let sql = format!(
            "UPDATE {} SET favorite = ?1 WHERE {} = ?2",
            T::TABLE,
            T::ID_COL
        );
        let conn = self.pool.get()?;
        conn.execute(&sql, params![favorite, id])?;
        Ok(())
    }

    pub fn set_pinned<T: Toggleable>(&self, id: &Cuid, pinned: bool) -> Result<()> {
        let sql = format!(
            "UPDATE {} SET pinned = ?1 WHERE {} = ?2",
            T::TABLE,
            T::ID_COL
        );
        let conn = self.pool.get()?;
        conn.execute(&sql, params![pinned, id])?;
        Ok(())
    }

    pub fn search_library(
        &self,
        query: &str,
        limit: i64,
    ) -> Result<Vec<(Cuid, String, Option<String>, String)>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let Some(fts_query) = to_fts_query(query) else {
            return Ok(Vec::new());
        };

        let per_type_limit = (limit.saturating_mul(2)).max(20);
        let conn = self.pool.get()?;

        collect_mapped::<SearchResultRow, (Cuid, String, Option<String>, String), _>(
            &conn,
            r#"
            WITH
            search_params AS (SELECT ?1 AS query_text),
            song_matches AS (
                SELECT DISTINCT
                    s.id, s.title AS name, s.image_id AS image, 'Song' AS item_type,
                    CASE
                        WHEN s.title = sp.query_text COLLATE NOCASE THEN 400
                        WHEN s.title LIKE sp.query_text || '%' COLLATE NOCASE THEN 300
                        WHEN EXISTS (SELECT 1 FROM artists ar WHERE ar.id = s.artist_id AND ar.name LIKE sp.query_text || '%' COLLATE NOCASE) THEN 220
                        WHEN EXISTS (SELECT 1 FROM albums al WHERE al.id = s.album_id AND al.title LIKE sp.query_text || '%' COLLATE NOCASE) THEN 200
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
                    al.id, al.title AS name, al.image_id AS image, 'Album' AS item_type,
                    CASE
                        WHEN al.title = sp.query_text COLLATE NOCASE THEN 350
                        WHEN al.title LIKE sp.query_text || '%' COLLATE NOCASE THEN 260
                        WHEN EXISTS (SELECT 1 FROM artists ar WHERE ar.id = al.artist_id AND ar.name LIKE sp.query_text || '%' COLLATE NOCASE) THEN 180
                        ELSE 90
                    END AS score
                FROM albums al
                CROSS JOIN search_params sp
                WHERE al.title LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                   OR EXISTS (SELECT 1 FROM artists ar WHERE ar.id = al.artist_id AND ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE)
                ORDER BY score DESC, al.title COLLATE NOCASE ASC
                LIMIT ?3
            ),
            artist_matches AS (
                SELECT
                    ar.id, ar.name AS name, ar.image_id AS image, 'Artist' AS item_type,
                    CASE
                        WHEN ar.name = sp.query_text COLLATE NOCASE THEN 320
                        WHEN ar.name LIKE sp.query_text || '%' COLLATE NOCASE THEN 250
                        ELSE 80
                    END AS score
                FROM artists ar
                CROSS JOIN search_params sp
                WHERE ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                ORDER BY score DESC, ar.name COLLATE NOCASE ASC
                LIMIT ?3
            ),
            playlist_matches AS (
                SELECT
                    p.id, p.name AS name, p.image_id AS image, 'Playlist' AS item_type,
                    CASE
                        WHEN p.name = sp.query_text COLLATE NOCASE THEN 300
                        WHEN p.name LIKE sp.query_text || '%' COLLATE NOCASE THEN 240
                        ELSE 70
                    END AS score
                FROM playlists p
                CROSS JOIN search_params sp
                WHERE p.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                ORDER BY score DESC, p.name COLLATE NOCASE ASC
                LIMIT ?3
            ),
            all_matches AS (
                SELECT * FROM song_matches
                UNION ALL SELECT * FROM album_matches
                UNION ALL SELECT * FROM artist_matches
                UNION ALL SELECT * FROM playlist_matches
            )
            SELECT id, name, image, item_type
            FROM all_matches
            ORDER BY score DESC, name COLLATE NOCASE ASC
            LIMIT ?4
            "#,
            params![query, fts_query, per_type_limit, limit],
            SearchResultRow::from_row,
        )
    }

    pub fn get_search_match_counts(&self, query: &str) -> Result<(usize, usize, usize, usize)> {
        let songs = self.get_songs_count(Some(query))?;
        let albums = self.get_albums_count(query)?;
        let artists = self.get_artists_count(query)?;
        let playlists = self.get_playlists_count(query)?;
        Ok((songs as usize, albums, artists, playlists as usize))
    }

    pub fn get_playlists_count(&self, query: &str) -> Result<i64> {
        let conn = self.pool.get()?;
        let query = query.trim();
        if query.is_empty() {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM playlists", [], |row| row.get(0))?;
            return Ok(count);
        }

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM playlists p
             WHERE p.name LIKE '%' || ?1 || '%' COLLATE NOCASE",
            params![query],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn upsert_image(&self, id: &str, data: &[u8]) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO images (id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET
                data = excluded.data,
                date_updated = DATETIME('now')",
            params![id, data],
        )?;
        Ok(())
    }

    pub fn get_image(&self, id: &str) -> Result<Option<Image>> {
        let conn = self.pool.get()?;
        let row = conn
            .query_row(
                "SELECT * FROM images WHERE id = ?1",
                params![id],
                ImageRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    #[allow(dead_code)]
    pub fn delete_image(&self, id: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM images WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_recently_added_items(&self, limit: i64) -> Result<Vec<RecentItem>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            r#"
            WITH recent_songs AS (
                SELECT s.id, s.title, s.artist_id, s.album_id, s.image_id, s.date_added, s.date
                FROM songs s
                ORDER BY s.date_added DESC
                LIMIT ?1
            ),
            image_groups AS (
                SELECT
                    COALESCE(rs.image_id, 'no_image_' || rs.id) AS group_key,
                    rs.image_id,
                    rs.album_id,
                    MAX(rs.date_added) AS most_recent_date,
                    COUNT(*) AS song_count,
                    MIN(rs.id) AS first_song_id
                FROM recent_songs rs
                GROUP BY group_key, rs.image_id, rs.album_id
            )
            SELECT
                ig.song_count,
                ig.first_song_id,
                s.title AS first_song_title,
                ig.image_id,
                s.date AS first_year,
                ig.album_id,
                al.title AS album_title,
                ar.name AS artist_name
            FROM image_groups ig
            JOIN songs s ON ig.first_song_id = s.id
            LEFT JOIN albums al ON ig.album_id = al.id
            LEFT JOIN artists ar ON s.artist_id = ar.id
            LEFT JOIN images img ON ig.image_id = img.id
            ORDER BY ig.most_recent_date DESC
            "#,
        )?;
        let rows = stmt
            .query_map(params![limit], RecentItemRow::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|r| r.into_recent_item()).collect())
    }

    pub fn get_recently_played_items(&self, limit: i64) -> Result<Vec<RecentItem>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            r#"
            WITH recent_song_plays AS (
                SELECT ec.song_id, MAX(e.timestamp) AS most_recent_date
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
        )?;
        let rows = stmt
            .query_map(params!["PLAY", limit], RecentItemRow::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|r| r.into_recent_item()).collect())
    }

    pub fn get_pinned_items(&self) -> Vec<PinnedItem> {
        let run = || -> Result<Vec<PinnedItem>> {
            let conn = self.pool.get()?;
            collect_mapped::<PinnedItemRow, PinnedItem, _>(
                &conn,
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
                [],
                PinnedItemRow::from_row,
            )
        };
        run().unwrap_or_default()
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
                    WHEN EXISTS (SELECT 1 FROM artists ar3 WHERE ar3.id = s.artist_id AND ar3.name LIKE ?1 || '%' COLLATE NOCASE) THEN 220
                    WHEN EXISTS (SELECT 1 FROM albums al3 WHERE al3.id = s.album_id AND al3.title LIKE ?1 || '%' COLLATE NOCASE) THEN 200
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
