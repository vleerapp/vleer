use crate::data::{
    db::models::*,
    models::{
        Album, AlbumListItem, Artist, ArtistListItem, Cuid, Event, EventContext, EventType, Image,
        PinnedItem, Playlist, PlaylistListItem, PlaylistTrack, RecentItem, Song, SongListItem,
        SongSort,
    },
    search::{
        AlbumSearchEntry, ArtistSearchEntry, PlaylistSearchEntry, SearchIndex, SongSearchEntry,
    },
};
use anyhow::Result;
use gpui::Global;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, ToSql, params};
use rusqlite_migration::Migrations;
use rust_embed::RustEmbed;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "./migrations"]
struct MigrationFiles;

fn open_connection(path: &Path, busy_timeout_ms: u32) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = {busy_timeout_ms};
         PRAGMA auto_vacuum = FULL;"
    ))?;
    conn.set_prepared_statement_cache_capacity(64);
    Ok(conn)
}

fn run_migrations(conn: &mut Connection) -> Result<()> {
    let mut files: Vec<(String, String)> = MigrationFiles::iter()
        .filter_map(|name| {
            let sql = MigrationFiles::get(&name)?;
            let text = std::str::from_utf8(sql.data.as_ref()).ok()?.to_owned();
            Some((name.into_owned(), text))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let migrations: Vec<rusqlite_migration::M> = files
        .iter()
        .map(|(_, sql)| rusqlite_migration::M::up(sql))
        .collect();

    Migrations::new(migrations).to_latest(conn)?;
    Ok(())
}

fn collect_mapped<T, U, F>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    mapper: F,
) -> Result<Vec<U>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    T: Into<U>,
{
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt
        .query_map(params, mapper)?
        .collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    pub image_conn: Arc<Mutex<Connection>>,
    search_index: Arc<Mutex<SearchIndex>>,
}

impl Global for Database {}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let mut bootstrap = Connection::open(path)?;
        run_migrations(&mut bootstrap)?;
        drop(bootstrap);

        let conn = open_connection(path, 3000)?;
        let image_conn = open_connection(path, 5000)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            image_conn: Arc::new(Mutex::new(image_conn)),
            search_index: Arc::new(Mutex::new(SearchIndex::default())),
        };
        db.rebuild_search_index();
        Ok(db)
    }

    pub fn rebuild_search_index(&self) {
        match self.load_search_index_data() {
            Ok(index) => *self.search_index.lock() = index,
            Err(e) => tracing::error!("rebuild_search_index failed: {e}"),
        }
    }

    fn load_search_index_data(&self) -> Result<SearchIndex> {
        let conn = self.conn.lock();

        let songs = {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title,
                        COALESCE((SELECT GROUP_CONCAT(name, ', ')
                                  FROM (SELECT ar.name FROM songs_artists sa
                                        JOIN artists ar ON sa.artist_id = ar.id
                                        WHERE sa.song_id = s.id ORDER BY sa.position)), '') AS artist,
                        COALESCE(al.title, '') AS album,
                        s.image_id
                 FROM songs s
                 LEFT JOIN albums al ON s.album_id = al.id",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Cuid>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(id, title, artist, album, image_id)| {
                SongSearchEntry::new(id, title, artist, album, image_id)
            })
            .collect()
        };

        let artists = {
            let mut stmt = conn.prepare("SELECT id, name, image_id FROM artists")?;
            stmt.query_map([], |row| {
                Ok(ArtistSearchEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    image_id: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let albums = {
            let mut stmt = conn.prepare(
                "SELECT al.id, al.title,
                        COALESCE((SELECT GROUP_CONCAT(name, ', ')
                                  FROM (SELECT ar.name FROM albums_artists aa
                                        JOIN artists ar ON aa.artist_id = ar.id
                                        WHERE aa.album_id = al.id ORDER BY aa.position)), '') AS artist,
                        al.image_id
                 FROM albums al",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Cuid>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(id, title, artist, image_id)| AlbumSearchEntry::new(id, title, artist, image_id))
            .collect()
        };

        let playlists = {
            let mut stmt = conn.prepare("SELECT id, name, image_id FROM playlists")?;
            stmt.query_map([], |row| {
                Ok(PlaylistSearchEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    image_id: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        Ok(SearchIndex {
            songs,
            artists,
            albums,
            playlists,
        })
    }

    pub fn get_song(&self, id: &Cuid) -> Result<Option<Song>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT s.*,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM songs s
             WHERE s.id = ?1",
        )?;
        let row = stmt.query_row(params![id], SongRow::from_row).optional()?;
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
            "SELECT s.*,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM songs s
             WHERE s.id IN ({placeholders})"
        );
        let conn = self.conn.lock();
        let params: Vec<&dyn ToSql> = ids.iter().map(|id| id as &dyn ToSql).collect();
        collect_mapped::<SongRow, Song, _>(&conn, &sql, params.as_slice(), SongRow::from_row)
    }

    pub fn get_song_by_path(&self, file_path: &str) -> Result<Option<Song>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT s.*,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM songs s
             WHERE s.file_path = ?1",
        )?;
        let row = stmt
            .query_row(params![file_path], SongRow::from_row)
            .optional()?;
        Ok(row.map(Into::into))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_song(
        &self,
        title: &str,
        artists: &[&str],
        album_id: Option<&Cuid>,
        file_path: &str,
        duration: i32,
        track_number: Option<i32>,
        year: Option<i32>,
        genres: &[&str],
        image_id: Option<&str>,
        file_size: i64,
        file_modified: i64,
        lufs: Option<f32>,
    ) -> Result<()> {
        let year_str = year.map(|y| y.to_string());
        let id = Cuid::new();
        let (song_id, artist_entries, album_title) = {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;

            let song_id: Cuid = tx
                .prepare_cached(
                    "INSERT INTO songs (id, title, album_id, file_path, file_size, file_modified, date, duration, image_id, track_number, lufs)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(file_path) DO UPDATE SET
                        title = excluded.title,
                        album_id = excluded.album_id,
                        file_size = excluded.file_size,
                        file_modified = excluded.file_modified,
                        date = excluded.date,
                        duration = excluded.duration,
                        image_id = excluded.image_id,
                        track_number = excluded.track_number,
                        lufs = excluded.lufs
                     RETURNING id",
                )?
                .query_row(
                    params![id, title, album_id, file_path, file_size, file_modified, year_str, duration, image_id, track_number, lufs],
                    |row| row.get(0),
                )?;

            tx.execute(
                "DELETE FROM songs_artists WHERE song_id = ?1",
                params![song_id],
            )?;
            let mut artist_entries: Vec<(Cuid, String)> = Vec::with_capacity(artists.len());
            for (position, &artist_name) in artists.iter().enumerate() {
                let artist_id = Cuid::new();
                let actual_artist_id: Cuid = tx
                    .prepare_cached(
                        "INSERT INTO artists (id, name) VALUES (?1, ?2)
                         ON CONFLICT(name) DO UPDATE SET name = excluded.name
                         RETURNING id",
                    )?
                    .query_row(params![artist_id, artist_name], |row| row.get(0))?;
                tx.execute(
                    "INSERT INTO songs_artists (song_id, artist_id, position) VALUES (?1, ?2, ?3)
                     ON CONFLICT(song_id, artist_id) DO UPDATE SET position = excluded.position",
                    params![song_id, actual_artist_id, position as i64],
                )?;
                artist_entries.push((actual_artist_id, artist_name.to_string()));
            }

            tx.execute(
                "DELETE FROM songs_genres WHERE song_id = ?1",
                params![song_id],
            )?;
            for &genre_name in genres {
                let genre_id = Cuid::new();
                let actual_genre_id: Cuid = tx
                    .prepare_cached(
                        "INSERT INTO genres (id, name) VALUES (?1, ?2)
                         ON CONFLICT(name) DO UPDATE SET name = excluded.name
                         RETURNING id",
                    )?
                    .query_row(params![genre_id, genre_name], |row| row.get(0))?;
                tx.execute(
                    "INSERT INTO songs_genres (song_id, genre_id) VALUES (?1, ?2)
                     ON CONFLICT(song_id, genre_id) DO NOTHING",
                    params![song_id, actual_genre_id],
                )?;
            }

            let album_title: String = if let Some(aid) = album_id {
                tx.prepare_cached("SELECT title FROM albums WHERE id = ?1")?
                    .query_row(params![aid], |row| row.get(0))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            tx.commit()?;
            (song_id, artist_entries, album_title)
        };

        let artist_joined = artist_entries
            .iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut index = self.search_index.lock();
        for (aid, name) in artist_entries {
            index.upsert_artist(aid, name);
        }
        index.upsert_song(
            song_id,
            title.to_string(),
            artist_joined,
            album_title,
            image_id.map(String::from),
        );
        Ok(())
    }

    pub fn delete_song(&self, id: &Cuid) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM songs WHERE id = ?1", params![id])?;
        }
        self.rebuild_search_index();
        Ok(())
    }

    pub fn get_song_paths(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached("SELECT file_path FROM songs")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_song_file_states(&self) -> Result<Vec<(String, i64, i64)>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare_cached("SELECT file_path, file_size, file_modified FROM songs")?;
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
        let trimmed = query.map(|q| q.trim()).filter(|q| !q.is_empty());
        let Some(q) = trimmed else {
            let conn = self.conn.lock();
            let count: i64 = conn
                .prepare_cached("SELECT COUNT(*) FROM songs")?
                .query_row([], |row| row.get(0))?;
            return Ok(count);
        };
        let index = self.search_index.lock();
        Ok(index.fuzzy_song_ids(q).len() as i64)
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

        if !has_query {
            let order_clause = song_order(sort, ascending);
            let sql = format!(
                "SELECT s.id, s.title,
                        (SELECT GROUP_CONCAT(name, ', ') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artist_name,
                        al.title AS album_title,
                        s.album_id, s.duration, s.image_id,
                        (SELECT GROUP_CONCAT(g.name, ', ') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
                 FROM songs s
                 LEFT JOIN albums al ON s.album_id = al.id
                 ORDER BY {order_clause}
                 LIMIT ?1 OFFSET ?2"
            );
            let conn = self.conn.lock();
            return collect_mapped::<SongListRow, SongListItem, _>(
                &conn,
                &sql,
                params![limit, offset],
                SongListRow::from_row,
            );
        }

        let q = query.unwrap().trim();
        let page_ids: Vec<Cuid> = {
            let index = self.search_index.lock();
            index
                .fuzzy_song_ids(q)
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(|(_, id)| id)
                .collect()
        };
        let conn = self.conn.lock();
        fetch_songs_by_ids(&conn, &page_ids)
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

        if !has_query {
            let order_clause = song_order(sort, ascending);
            let sql = format!(
                "SELECT s.id
                 FROM songs s
                 LEFT JOIN albums al ON s.album_id = al.id
                 ORDER BY {order_clause}
                 LIMIT -1 OFFSET ?1"
            );
            let conn = self.conn.lock();
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt
                .query_map(params![offset], |row| row.get::<_, Cuid>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(rows);
        }

        let index = self.search_index.lock();
        Ok(index
            .fuzzy_song_ids(query)
            .into_iter()
            .skip(offset as usize)
            .map(|(_, id)| id)
            .collect())
    }

    pub fn get_album_songs(&self, album_id: &Cuid) -> Result<Vec<Song>> {
        let conn = self.conn.lock();
        collect_mapped::<SongRow, Song, _>(
            &conn,
            "SELECT s.*,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM songs s
             WHERE s.album_id = ?1
             ORDER BY s.track_number ASC",
            params![album_id],
            SongRow::from_row,
        )
    }

    pub fn get_artist(&self, id: &Cuid) -> Result<Option<Artist>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT * FROM artists WHERE id = ?1",
                params![id],
                ArtistRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn get_artists_count(&self, query: &str) -> Result<usize> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            let count: i64 = conn
                .prepare_cached("SELECT COUNT(*) FROM artists")?
                .query_row([], |row| row.get(0))?;
            return Ok(count.max(0) as usize);
        }
        let index = self.search_index.lock();
        Ok(index.fuzzy_artist_ids(query).len())
    }

    pub fn get_artists(&self, query: &str, offset: i64, limit: i64) -> Result<Vec<ArtistListItem>> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
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

        let page_ids: Vec<Cuid> = {
            let index = self.search_index.lock();
            index
                .fuzzy_artist_ids(query)
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(|(_, id)| id)
                .collect()
        };
        let conn = self.conn.lock();
        fetch_artists_by_ids(&conn, &page_ids)
    }

    pub fn get_album(&self, id: &Cuid) -> Result<Option<Album>> {
        let conn = self.conn.lock();
        let row = conn
            .prepare_cached(
                "SELECT al.id, al.title, al.image_id, al.favorite, al.pinned,
                        (SELECT GROUP_CONCAT(name, ',')
                         FROM (SELECT ar.name FROM albums_artists aa JOIN artists ar ON aa.artist_id = ar.id WHERE aa.album_id = al.id ORDER BY aa.position)) AS artists
                 FROM albums al WHERE al.id = ?1",
            )?
            .query_row(params![id], AlbumRow::from_row)
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn get_artist_by_name(&self, name: &str) -> Result<Option<Artist>> {
        let conn = self.conn.lock();
        let row = conn
            .prepare_cached("SELECT * FROM artists WHERE name = ?1")?
            .query_row(params![name], ArtistRow::from_row)
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn get_albums_count(&self, query: &str) -> Result<usize> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            let count: i64 = conn
                .prepare_cached("SELECT COUNT(*) FROM albums")?
                .query_row([], |row| row.get(0))?;
            return Ok(count.max(0) as usize);
        }
        let index = self.search_index.lock();
        Ok(index.fuzzy_album_ids(query).len())
    }

    pub fn get_albums(&self, query: &str, offset: i64, limit: i64) -> Result<Vec<AlbumListItem>> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            return collect_mapped::<AlbumListRow, AlbumListItem, _>(
                &conn,
                "SELECT al.id, al.title,
                        (SELECT GROUP_CONCAT(name, ', ')
                         FROM (SELECT ar.name FROM albums_artists aa JOIN artists ar ON aa.artist_id = ar.id WHERE aa.album_id = al.id ORDER BY aa.position)) AS artist_name,
                        al.image_id, MIN(s.date) AS year
                 FROM albums al
                 LEFT JOIN songs s ON s.album_id = al.id
                 GROUP BY al.id
                 ORDER BY al.title COLLATE NOCASE ASC
                 LIMIT ?1 OFFSET ?2",
                params![limit, offset],
                AlbumListRow::from_row,
            );
        }

        let page_ids: Vec<Cuid> = {
            let index = self.search_index.lock();
            index
                .fuzzy_album_ids(query)
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(|(_, id)| id)
                .collect()
        };
        let conn = self.conn.lock();
        fetch_albums_by_ids(&conn, &page_ids)
    }

    pub fn upsert_album(
        &self,
        title: &str,
        artists: &[&str],
        image_id: Option<&str>,
    ) -> Result<Cuid> {
        let id = Cuid::new();
        let (album_id, new_artists, all_artist_names, effective_image) = {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            let album_id: Cuid = tx
                .prepare_cached(
                    "INSERT INTO albums (id, title, image_id)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(title) DO UPDATE SET
                        image_id = COALESCE(excluded.image_id, albums.image_id)
                     RETURNING id",
                )?
                .query_row(params![id, title, image_id], |row| row.get(0))?;

            let existing_names: Vec<String> = {
                let mut stmt = tx.prepare_cached(
                    "SELECT a.name FROM artists a
                     JOIN albums_artists aa ON a.id = aa.artist_id
                     WHERE aa.album_id = ?1",
                )?;
                stmt.query_map(params![album_id], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
            };

            let existing_set: std::collections::HashSet<&str> =
                existing_names.iter().map(|s| s.as_str()).collect();

            let max_position: i64 = tx
                .prepare_cached(
                    "SELECT COALESCE(MAX(position), -1) FROM albums_artists WHERE album_id = ?1",
                )?
                .query_row(params![album_id], |row| row.get(0))
                .unwrap_or(-1);

            let mut next_position = max_position + 1;
            let mut new_artists: Vec<(Cuid, String)> = Vec::new();
            for artist_name in artists {
                if existing_set.contains(artist_name) {
                    continue;
                }
                let artist_id = Cuid::new();
                let actual_artist_id: Cuid = tx
                    .prepare_cached(
                        "INSERT INTO artists (id, name) VALUES (?1, ?2)
                         ON CONFLICT(name) DO UPDATE SET name = excluded.name
                         RETURNING id",
                    )?
                    .query_row(params![artist_id, artist_name], |row| row.get(0))?;
                tx.prepare_cached(
                    "INSERT INTO albums_artists (album_id, artist_id, position) VALUES (?1, ?2, ?3)
                     ON CONFLICT(album_id, artist_id) DO UPDATE SET position = excluded.position",
                )?
                .execute(params![album_id, actual_artist_id, next_position])?;
                next_position += 1;
                new_artists.push((actual_artist_id, (*artist_name).to_string()));
            }

            let all_artist_names: String = tx
                .prepare_cached(
                    "SELECT COALESCE((SELECT GROUP_CONCAT(name, ', ')
                                      FROM (SELECT ar.name FROM albums_artists aa
                                            JOIN artists ar ON aa.artist_id = ar.id
                                            WHERE aa.album_id = ?1 ORDER BY aa.position)), '')",
                )?
                .query_row(params![album_id], |row| row.get(0))
                .unwrap_or_default();

            let effective_image: Option<String> = tx
                .prepare_cached("SELECT image_id FROM albums WHERE id = ?1")?
                .query_row(params![album_id], |row| row.get(0))
                .unwrap_or(None);

            tx.commit()?;
            (album_id, new_artists, all_artist_names, effective_image)
        };

        let mut index = self.search_index.lock();
        for (aid, name) in new_artists {
            index.upsert_artist(aid, name);
        }
        index.upsert_album(
            album_id.clone(),
            title.to_string(),
            all_artist_names,
            effective_image,
        );
        Ok(album_id)
    }

    pub fn delete_album(&self, id: &Cuid) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM albums WHERE id = ?1", params![id])?;
        }
        self.rebuild_search_index();
        Ok(())
    }

    pub fn get_playlist(&self, id: &Cuid) -> Result<Option<Playlist>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT * FROM playlists WHERE id = ?1",
                params![id],
                PlaylistRow::from_row,
            )
            .optional()?;
        Ok(row.map(Into::into))
    }

    pub fn upsert_playlist(
        &self,
        id: &Cuid,
        name: &str,
        description: Option<&str>,
        image_id: Option<&str>,
        pinned: bool,
    ) -> Result<()> {
        {
            let conn = self.conn.lock();
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
        }
        let mut index = self.search_index.lock();
        index.upsert_playlist(id.clone(), name.to_string(), image_id.map(String::from));
        Ok(())
    }

    pub fn delete_playlist(&self, id: &Cuid) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        }
        let mut index = self.search_index.lock();
        index.remove_playlist(id);
        Ok(())
    }

    pub fn upsert_playlist_song(&self, playlist_id: &Cuid, song_id: &Cuid) -> Result<()> {
        let id = Cuid::new();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO playlist_songs (id, playlist_id, song_id, position)
             VALUES (?1, ?2, ?3, COALESCE((SELECT MAX(position) FROM playlist_songs WHERE playlist_id = ?2), -1) + 1)
             ON CONFLICT(playlist_id, song_id) DO NOTHING",
            params![id, playlist_id, song_id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn delete_playlist_song(&self, playlist_id: &Cuid, song_id: &Cuid) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM playlist_songs WHERE playlist_id = ?1 AND song_id = ?2",
            params![playlist_id, song_id],
        )?;
        Ok(())
    }

    pub fn clear_playlist(&self, playlist_id: &Cuid) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM playlist_songs WHERE playlist_id = ?1",
            params![playlist_id],
        )?;
        Ok(())
    }

    pub fn get_playlist_songs(&self, playlist_id: &Cuid) -> Result<Vec<PlaylistTrack>> {
        let conn = self.conn.lock();
        collect_mapped::<PlaylistTrackRow, PlaylistTrack, _>(
            &conn,
            "SELECT pt.id AS pt_id, pt.playlist_id, pt.position, s.*,
                    al.title AS album_title,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM playlist_songs pt
             JOIN songs s ON s.id = pt.song_id
             LEFT JOIN albums al ON s.album_id = al.id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position ASC",
            params![playlist_id],
            PlaylistTrackRow::from_row,
        )
    }

    #[allow(dead_code)]
    pub fn get_event(&self, id: &Cuid) -> Result<Option<Event>> {
        let conn = self.conn.lock();
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

        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO event_contexts (id, song_id, playlist_id) VALUES (?1, ?2, ?3)",
            params![id, song_id, playlist_id],
        )?;
        Ok(id)
    }

    #[allow(dead_code)]
    pub fn get_event_context(&self, id: &Cuid) -> Result<Option<EventContext>> {
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
        collect_mapped::<EventContextRow, EventContext, _>(
            &conn,
            "SELECT * FROM event_contexts WHERE song_id = ?1",
            params![song_id],
            EventContextRow::from_row,
        )
    }

    #[allow(dead_code)]
    pub fn get_event_context_by_playlist(&self, playlist_id: &Cuid) -> Result<Vec<EventContext>> {
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
        conn.execute(&sql, params![favorite, id])?;
        Ok(())
    }

    pub fn set_pinned<T: Toggleable>(&self, id: &Cuid, pinned: bool) -> Result<()> {
        let sql = format!(
            "UPDATE {} SET pinned = ?1 WHERE {} = ?2",
            T::TABLE,
            T::ID_COL
        );
        let conn = self.conn.lock();
        conn.execute(&sql, params![pinned, id])?;
        Ok(())
    }

    pub fn search_library(&self, query: &str, limit: i64) -> Result<Vec<SearchResultRow>> {
        let query = query.trim();
        if query.is_empty() || limit <= 0 {
            return Ok(Vec::new());
        }
        let index = self.search_index.lock();
        Ok(index.fuzzy_search_all(query, limit as usize))
    }

    pub fn get_search_match_counts(&self, query: &str) -> Result<(usize, usize, usize, usize)> {
        let query = query.trim();
        if query.is_empty() {
            return Ok((0, 0, 0, 0));
        }
        let index = self.search_index.lock();
        Ok((
            index.fuzzy_song_ids(query).len(),
            index.fuzzy_album_ids(query).len(),
            index.fuzzy_artist_ids(query).len(),
            index.fuzzy_playlist_ids(query).len(),
        ))
    }

    pub fn get_playlists_count(&self, query: &str) -> Result<i64> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            let count: i64 = conn
                .prepare_cached("SELECT COUNT(*) FROM playlists")?
                .query_row([], |row| row.get(0))?;
            return Ok(count);
        }
        let index = self.search_index.lock();
        Ok(index.fuzzy_playlist_ids(query).len() as i64)
    }

    pub fn get_playlists(
        &self,
        query: &str,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<PlaylistListItem>> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            return collect_mapped::<PlaylistListRow, PlaylistListItem, _>(
                &conn,
                "SELECT p.id, p.name, p.image_id, COUNT(pt.id) AS song_count
                 FROM playlists p
                 LEFT JOIN playlist_songs pt ON pt.playlist_id = p.id
                 GROUP BY p.id, p.name, p.image_id
                 ORDER BY p.name COLLATE NOCASE ASC
                 LIMIT ?1 OFFSET ?2",
                params![limit, offset],
                PlaylistListRow::from_row,
            );
        }

        let page_ids: Vec<Cuid> = {
            let index = self.search_index.lock();
            index
                .fuzzy_playlist_ids(query)
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(|(_, id)| id)
                .collect()
        };
        let conn = self.conn.lock();
        fetch_playlists_by_ids(&conn, &page_ids)
    }

    pub fn upsert_image(&self, id: &str, data: &[u8]) -> Result<()> {
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
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
        let conn = self.conn.lock();
        conn.execute("DELETE FROM images WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_recently_added_items(&self, limit: i64) -> Result<Vec<RecentItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            r#"
            WITH recent_songs AS (
                SELECT s.id, s.title, s.album_id, s.image_id, s.date_added, s.date
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
                (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id
                 WHERE sa.song_id = s.id ORDER BY sa.position LIMIT 1) AS artist_name
            FROM image_groups ig
            JOIN songs s ON ig.first_song_id = s.id
            LEFT JOIN albums al ON ig.album_id = al.id
            ORDER BY ig.most_recent_date DESC
            "#,
        )?;
        let rows = stmt
            .query_map(params![limit], RecentItemRow::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|r| r.into_recent_item()).collect())
    }

    pub fn get_recently_played_items(&self, limit: i64) -> Result<Vec<RecentItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
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
                (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id
                 WHERE sa.song_id = s.id ORDER BY sa.position LIMIT 1) AS artist_name
            FROM recent_song_plays rsp
            JOIN songs s ON rsp.song_id = s.id
            LEFT JOIN albums al ON s.album_id = al.id
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
            let conn = self.conn.lock();
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

fn fetch_songs_by_ids(conn: &rusqlite::Connection, ids: &[Cuid]) -> Result<Vec<SongListItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT s.id, s.title,
                (SELECT GROUP_CONCAT(name, ', ') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artist_name,
                al.title AS album_title,
                s.album_id, s.duration, s.image_id,
                (SELECT GROUP_CONCAT(g.name, ', ') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
         FROM songs s
         LEFT JOIN albums al ON s.album_id = al.id
         WHERE s.id IN ({placeholders})"
    );
    let params: Vec<&dyn ToSql> = ids.iter().map(|id| id as &dyn ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let map: HashMap<Cuid, SongListItem> = stmt
        .query_map(params.as_slice(), SongListRow::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|r| {
            let item: SongListItem = r.into();
            (item.id.clone(), item)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

fn fetch_artists_by_ids(conn: &rusqlite::Connection, ids: &[Cuid]) -> Result<Vec<ArtistListItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT ar.id, ar.name, ar.image_id
         FROM artists ar
         WHERE ar.id IN ({placeholders})"
    );
    let params: Vec<&dyn ToSql> = ids.iter().map(|id| id as &dyn ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let map: HashMap<Cuid, ArtistListItem> = stmt
        .query_map(params.as_slice(), ArtistListRow::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|r| {
            let item: ArtistListItem = r.into();
            (item.id.clone(), item)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

fn fetch_albums_by_ids(conn: &rusqlite::Connection, ids: &[Cuid]) -> Result<Vec<AlbumListItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT al.id, al.title,
                (SELECT GROUP_CONCAT(name, ', ')
                 FROM (SELECT ar.name FROM albums_artists aa JOIN artists ar ON aa.artist_id = ar.id WHERE aa.album_id = al.id ORDER BY aa.position)) AS artist_name,
                al.image_id, MIN(s.date) AS year
         FROM albums al
         LEFT JOIN songs s ON s.album_id = al.id
         WHERE al.id IN ({placeholders})
         GROUP BY al.id"
    );
    let params: Vec<&dyn ToSql> = ids.iter().map(|id| id as &dyn ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let map: HashMap<Cuid, AlbumListItem> = stmt
        .query_map(params.as_slice(), AlbumListRow::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|r| {
            let item: AlbumListItem = r.into();
            (item.id.clone(), item)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

fn fetch_playlists_by_ids(
    conn: &rusqlite::Connection,
    ids: &[Cuid],
) -> Result<Vec<PlaylistListItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT p.id, p.name, p.image_id, COUNT(pt.id) AS song_count
         FROM playlists p
         LEFT JOIN playlist_songs pt ON pt.playlist_id = p.id
         WHERE p.id IN ({placeholders})
         GROUP BY p.id, p.name, p.image_id"
    );
    let params: Vec<&dyn ToSql> = ids.iter().map(|id| id as &dyn ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let map: HashMap<Cuid, PlaylistListItem> = stmt
        .query_map(params.as_slice(), PlaylistListRow::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|r| {
            let item: PlaylistListItem = r.into();
            (item.id.clone(), item)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

fn song_order(sort: SongSort, ascending: bool) -> &'static str {
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
        SongSort::Genre => {
            if ascending {
                "genres COLLATE NOCASE ASC, s.id ASC"
            } else {
                "genres COLLATE NOCASE DESC, s.id ASC"
            }
        }
        SongSort::Default => "s.date_added DESC, s.id ASC",
    }
}
