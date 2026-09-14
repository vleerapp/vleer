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
use std::collections::{HashMap, HashSet};
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
         PRAGMA auto_vacuum = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA cache_size = -65536;"
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

pub struct BatchTrack<'a> {
    pub title: &'a str,
    pub artists: &'a [&'a str],
    pub genres: &'a [&'a str],
    pub album: Option<&'a str>,
    pub file_path: &'a str,
    pub duration: i32,
    pub track_number: Option<i32>,
    pub year: Option<i32>,

    pub recheck_image: bool,
    pub file_size: i64,
    pub file_modified: i64,
    pub lufs: Option<f32>,
}

struct AlbumCacheEntry {
    id: Cuid,
    artists: HashSet<String>,
    next_position: i64,
}

type SpellingVotes = HashMap<Cuid, HashMap<String, usize>>;

fn fold_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

pub mod write_profile {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    pub(super) static ALBUM_NS: AtomicU64 = AtomicU64::new(0);
    pub(super) static SONG_NS: AtomicU64 = AtomicU64::new(0);
    pub(super) static ARTIST_NS: AtomicU64 = AtomicU64::new(0);
    pub(super) static GENRE_NS: AtomicU64 = AtomicU64::new(0);
    pub(super) static COMMIT_NS: AtomicU64 = AtomicU64::new(0);
    pub(super) static LOCK_NS: AtomicU64 = AtomicU64::new(0);
    pub(super) static COMMITS: AtomicU64 = AtomicU64::new(0);
    pub(super) static WAL_BYTES: AtomicU64 = AtomicU64::new(0);

    pub(super) fn add(counter: &AtomicU64, started: std::time::Instant) {
        counter.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn reset() {
        for counter in [
            &ALBUM_NS, &SONG_NS, &ARTIST_NS, &GENRE_NS, &COMMIT_NS, &LOCK_NS,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
    }

    pub fn commit_stats() -> (u64, u64) {
        (
            COMMITS.load(Ordering::Relaxed),
            WAL_BYTES.load(Ordering::Relaxed) / 1_048_576,
        )
    }

    pub fn snapshot() -> (Duration, Duration, Duration, Duration, Duration, Duration) {
        let ns = |c: &AtomicU64| Duration::from_nanos(c.load(Ordering::Relaxed));
        (
            ns(&LOCK_NS),
            ns(&ALBUM_NS),
            ns(&SONG_NS),
            ns(&ARTIST_NS),
            ns(&GENRE_NS),
            ns(&COMMIT_NS),
        )
    }
}

#[derive(Default)]
pub struct ScanCache {
    artists: HashMap<String, Cuid>,
    genres: HashMap<String, Cuid>,
    albums: HashMap<String, AlbumCacheEntry>,
    artist_votes: SpellingVotes,
    genre_votes: SpellingVotes,
    album_votes: SpellingVotes,
}

fn find_or_insert(
    tx: &rusqlite::Transaction<'_>,
    find_sql: &str,
    insert_sql: &str,
    name: &str,
) -> Result<Cuid> {
    let existing: Option<Cuid> = tx
        .prepare_cached(find_sql)?
        .query_row(params![name], |row| row.get(0))
        .optional()?;
    match existing {
        Some(id) => Ok(id),
        None => Ok(tx
            .prepare_cached(insert_sql)?
            .query_row(params![Cuid::new(), name], |row| row.get(0))?),
    }
}

fn artist_id_cached(
    tx: &rusqlite::Transaction<'_>,
    cache: &mut ScanCache,
    name: &str,
) -> Result<Cuid> {
    let key = fold_name(name);
    if let Some(id) = cache.artists.get(&key) {
        return Ok(id.clone());
    }
    let id = find_or_insert(
        tx,
        "SELECT id FROM artists WHERE name = ?1 COLLATE NOCASE ORDER BY name LIMIT 1",
        "INSERT INTO artists (id, name) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name
         RETURNING id",
        name,
    )?;
    cache.artists.insert(key, id.clone());
    Ok(id)
}

fn genre_id_cached(
    tx: &rusqlite::Transaction<'_>,
    cache: &mut ScanCache,
    name: &str,
) -> Result<Cuid> {
    let key = fold_name(name);
    if let Some(id) = cache.genres.get(&key) {
        return Ok(id.clone());
    }
    let id = find_or_insert(
        tx,
        "SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE ORDER BY name LIMIT 1",
        "INSERT INTO genres (id, name) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name
         RETURNING id",
        name,
    )?;
    cache.genres.insert(key, id.clone());
    Ok(id)
}

fn upsert_album_cached(
    tx: &rusqlite::Transaction<'_>,
    cache: &mut ScanCache,
    title: &str,
    artists: &[&str],
) -> Result<Cuid> {
    let title_key = fold_name(title);
    if !cache.albums.contains_key(&title_key) {
        let album_id = find_or_insert(
            tx,
            "SELECT id FROM albums WHERE title = ?1 COLLATE NOCASE ORDER BY title LIMIT 1",
            "INSERT INTO albums (id, title) VALUES (?1, ?2)
             ON CONFLICT(title) DO UPDATE SET title = excluded.title
             RETURNING id",
            title,
        )?;

        let existing: HashSet<String> = {
            let mut stmt = tx.prepare_cached(
                "SELECT a.name FROM artists a
                 JOIN albums_artists aa ON a.id = aa.artist_id
                 WHERE aa.album_id = ?1",
            )?;
            stmt.query_map(params![album_id], |row| row.get::<_, String>(0))?
                .map(|name| name.map(|name| fold_name(&name)))
                .collect::<rusqlite::Result<_>>()?
        };

        let max_position: i64 = tx
            .prepare_cached(
                "SELECT COALESCE(MAX(position), -1) FROM albums_artists WHERE album_id = ?1",
            )?
            .query_row(params![album_id], |row| row.get(0))
            .unwrap_or(-1);

        cache.albums.insert(
            title_key.clone(),
            AlbumCacheEntry {
                id: album_id,
                artists: existing,
                next_position: max_position + 1,
            },
        );
    }

    let (album_id, missing): (Cuid, Vec<&str>) = {
        let entry = &cache.albums[&title_key];
        let mut seen = HashSet::new();
        (
            entry.id.clone(),
            artists
                .iter()
                .copied()
                .filter(|name| {
                    let key = fold_name(name);
                    !entry.artists.contains(&key) && seen.insert(key)
                })
                .collect(),
        )
    };

    for name in missing {
        let artist_id = artist_id_cached(tx, cache, name)?;
        let position = {
            let entry = cache
                .albums
                .get_mut(&title_key)
                .expect("album cache entry inserted above");
            let position = entry.next_position;
            entry.next_position += 1;
            entry.artists.insert(fold_name(name));
            position
        };
        tx.prepare_cached(
            "INSERT INTO albums_artists (album_id, artist_id, position) VALUES (?1, ?2, ?3)
             ON CONFLICT(album_id, artist_id) DO UPDATE SET position = excluded.position",
        )?
        .execute(params![album_id, artist_id, position])?;
    }

    Ok(album_id)
}

fn vote(votes: &mut SpellingVotes, id: &Cuid, spelling: &str) {
    *votes
        .entry(id.clone())
        .or_default()
        .entry(spelling.to_string())
        .or_default() += 1;
}

fn apply_majority_spellings(
    tx: &rusqlite::Transaction<'_>,
    votes: &SpellingVotes,
    touched: &HashSet<Cuid>,
    state_sql: &str,
    rename_sql: &str,
) -> Result<()> {
    for id in touched {
        let Some(counts) = votes.get(id) else {
            continue;
        };
        let Some((winner, &winner_votes)) = counts
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        else {
            continue;
        };
        let Some((current, total)): Option<(String, i64)> = tx
            .prepare_cached(state_sql)?
            .query_row(params![id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?
        else {
            continue;
        };
        if current == *winner {
            continue;
        }
        let seen: usize = counts.values().sum();
        let unseen = (total.max(0) as usize).saturating_sub(seen);
        let current_votes = counts.get(&current).copied().unwrap_or(0);
        if winner_votes > current_votes + unseen {
            tx.prepare_cached(rename_sql)?
                .execute(params![winner, id])?;
        }
    }
    Ok(())
}

struct CaseDuplicates {
    candidates_sql: &'static str,
    merge_sql: &'static [&'static str],
}

const CASE_DUPLICATES: &[CaseDuplicates] = &[
    CaseDuplicates {
        candidates_sql: "SELECT id, name FROM artists
             ORDER BY name COLLATE NOCASE,
                      (SELECT COUNT(*) FROM songs_artists WHERE artist_id = artists.id) DESC,
                      (SELECT COUNT(*) FROM albums_artists WHERE artist_id = artists.id) DESC,
                      name",
        merge_sql: &[
            "INSERT OR IGNORE INTO songs_artists (song_id, artist_id, position)
             SELECT song_id, ?1, position FROM songs_artists WHERE artist_id = ?2",
            "INSERT OR IGNORE INTO albums_artists (album_id, artist_id, position)
             SELECT album_id, ?1, position FROM albums_artists WHERE artist_id = ?2",
            "UPDATE artists SET
                favorite = (SELECT MAX(COALESCE(favorite, 0)) FROM artists WHERE id IN (?1, ?2)),
                pinned = (SELECT MAX(COALESCE(pinned, 0)) FROM artists WHERE id IN (?1, ?2)),
                image_id = COALESCE(image_id, (SELECT image_id FROM artists WHERE id = ?2))
             WHERE id = ?1",
            "DELETE FROM songs_artists WHERE artist_id = ?2",
            "DELETE FROM albums_artists WHERE artist_id = ?2",
            "DELETE FROM artists WHERE id = ?2",
        ],
    },
    CaseDuplicates {
        candidates_sql: "SELECT id, title FROM albums
             ORDER BY title COLLATE NOCASE,
                      (SELECT COUNT(*) FROM songs WHERE album_id = albums.id) DESC,
                      title",
        merge_sql: &[
            "UPDATE songs SET album_id = ?1 WHERE album_id = ?2",
            "INSERT OR IGNORE INTO albums_artists (album_id, artist_id, position)
             SELECT ?1, artist_id,
                    position + (SELECT COALESCE(MAX(position), -1) + 1 FROM albums_artists WHERE album_id = ?1)
             FROM albums_artists WHERE album_id = ?2",
            "UPDATE albums SET
                favorite = (SELECT MAX(COALESCE(favorite, 0)) FROM albums WHERE id IN (?1, ?2)),
                pinned = (SELECT MAX(COALESCE(pinned, 0)) FROM albums WHERE id IN (?1, ?2)),
                image_id = COALESCE(image_id, (SELECT image_id FROM albums WHERE id = ?2))
             WHERE id = ?1",
            "DELETE FROM albums_artists WHERE album_id = ?2",
            "DELETE FROM albums WHERE id = ?2",
        ],
    },
    CaseDuplicates {
        candidates_sql: "SELECT id, name FROM genres
             ORDER BY name COLLATE NOCASE,
                      (SELECT COUNT(*) FROM songs_genres WHERE genre_id = genres.id) DESC,
                      name",
        merge_sql: &[
            "INSERT OR IGNORE INTO songs_genres (song_id, genre_id)
             SELECT song_id, ?1 FROM songs_genres WHERE genre_id = ?2",
            "DELETE FROM songs_genres WHERE genre_id = ?2",
            "DELETE FROM genres WHERE id = ?2",
        ],
    },
];

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    pub image_conn: Arc<Mutex<Connection>>,
    image_write_conn: Arc<Mutex<Connection>>,
    search_index: Arc<Mutex<SearchIndex>>,
    path: Arc<std::path::PathBuf>,
}

const ALBUM_IMAGE_ATTEMPTS: i64 = 3;

pub struct SongImageTarget {
    pub file_path: String,
    pub image_id: Option<String>,

    pub image_checked: bool,
}

pub enum AlbumImageTarget {
    Resolved(String),
    Candidates(Vec<Cuid>),
}

impl Global for Database {}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let mut bootstrap = Connection::open(path)?;
        run_migrations(&mut bootstrap)?;
        drop(bootstrap);

        let conn = open_connection(path, 3000)?;
        let image_conn = open_connection(path, 5000)?;

        let image_write_conn = open_connection(path, 15000)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            image_conn: Arc::new(Mutex::new(image_conn)),
            image_write_conn: Arc::new(Mutex::new(image_write_conn)),
            search_index: Arc::new(Mutex::new(SearchIndex::default())),
            path: Arc::new(path.to_path_buf()),
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

    pub fn song_image_target(&self, song_id: &Cuid) -> Result<Option<SongImageTarget>> {
        let conn = self.image_write_conn.lock();
        let row = conn
            .prepare_cached("SELECT file_path, image_id, image_checked FROM songs WHERE id = ?1")?
            .query_row(params![song_id], |row| {
                Ok(SongImageTarget {
                    file_path: row.get(0)?,
                    image_id: row.get(1)?,
                    image_checked: row.get::<_, i64>(2)? != 0,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn album_image_target(&self, album_id: &Cuid) -> Result<Option<AlbumImageTarget>> {
        let conn = self.image_write_conn.lock();

        let existing: Option<String> = conn
            .prepare_cached("SELECT image_id FROM albums WHERE id = ?1")?
            .query_row(params![album_id], |row| row.get(0))
            .optional()?
            .flatten();
        if let Some(image_id) = existing {
            return Ok(Some(AlbumImageTarget::Resolved(image_id)));
        }

        let from_track: Option<String> = conn
            .prepare_cached(
                "SELECT image_id FROM songs
                 WHERE album_id = ?1 AND image_id IS NOT NULL LIMIT 1",
            )?
            .query_row(params![album_id], |row| row.get(0))
            .optional()?
            .flatten();
        if let Some(image_id) = from_track {
            return Ok(Some(AlbumImageTarget::Resolved(image_id)));
        }

        let candidates: Vec<Cuid> = conn
            .prepare_cached(
                "SELECT id FROM songs
                 WHERE album_id = ?1 AND image_checked = 0
                 ORDER BY track_number, id LIMIT ?2",
            )?
            .query_map(params![album_id, ALBUM_IMAGE_ATTEMPTS], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        Ok(if candidates.is_empty() {
            None
        } else {
            Some(AlbumImageTarget::Candidates(candidates))
        })
    }

    pub fn image_exists(&self, image_id: &str) -> Result<bool> {
        let conn = self.image_write_conn.lock();
        let found: Option<i64> = conn
            .prepare_cached("SELECT 1 FROM images WHERE id = ?1")?
            .query_row(params![image_id], |row| row.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    pub fn store_song_image(
        &self,
        song_id: &Cuid,
        image_id: &str,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;

        if let Some(data) = data {
            tx.prepare_cached(
                "INSERT INTO images (id, data) VALUES (?1, ?2)
                 ON CONFLICT(id) DO NOTHING",
            )?
            .execute(params![image_id, data])?;
        }

        tx.prepare_cached("UPDATE songs SET image_id = ?2, image_checked = 1 WHERE id = ?1")?
            .execute(params![song_id, image_id])?;

        tx.prepare_cached(
            "UPDATE albums SET image_id = ?2
             WHERE id = (SELECT album_id FROM songs WHERE id = ?1) AND image_id IS NULL",
        )?
        .execute(params![song_id, image_id])?;

        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn write_image_row(&self, image_id: &str, data: &[u8]) -> Result<()> {
        let conn = self.image_write_conn.lock();
        conn.prepare_cached(
            "INSERT INTO images (id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        )?
        .execute(params![image_id, data])?;
        Ok(())
    }

    pub fn mark_song_image_missing(&self, song_id: &Cuid) -> Result<()> {
        let conn = self.image_write_conn.lock();
        conn.prepare_cached("UPDATE songs SET image_checked = 1 WHERE id = ?1")?
            .execute(params![song_id])?;
        Ok(())
    }

    pub fn songs_needing_image(&self, limit: i64) -> Result<Vec<Cuid>> {
        let conn = self.image_write_conn.lock();
        let rows = conn
            .prepare_cached(
                "SELECT id FROM songs
                 WHERE image_checked = 0 AND image_id IS NULL
                 LIMIT ?1",
            )?
            .query_map(params![limit], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn upsert_tracks_batch(
        &self,
        tracks: &[BatchTrack<'_>],
        cache: &mut ScanCache,
    ) -> Result<usize> {
        if tracks.is_empty() {
            return Ok(0);
        }

        let lock_started = std::time::Instant::now();
        let mut conn = self.conn.lock();
        write_profile::add(&write_profile::LOCK_NS, lock_started);
        let tx = conn.transaction()?;
        let mut written = 0usize;
        let mut touched_artists = HashSet::new();
        let mut touched_genres = HashSet::new();
        let mut touched_albums = HashSet::new();

        for track in tracks {
            let album_started = std::time::Instant::now();
            let album_id = match track.album {
                Some(album_title) => {
                    Some(upsert_album_cached(&tx, cache, album_title, track.artists)?)
                }
                None => None,
            };
            if let (Some(id), Some(title)) = (&album_id, track.album) {
                vote(&mut cache.album_votes, id, title);
                touched_albums.insert(id.clone());
            }
            write_profile::add(&write_profile::ALBUM_NS, album_started);
            let song_started = std::time::Instant::now();

            let year_str = track.year.map(|y| y.to_string());

            let song_id: Cuid = tx
                .prepare_cached(
                    "INSERT INTO songs (id, title, album_id, file_path, file_size, file_modified, date, duration, track_number, lufs, image_checked)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)
                     ON CONFLICT(file_path) DO UPDATE SET
                        title = excluded.title,
                        album_id = excluded.album_id,
                        file_size = excluded.file_size,
                        file_modified = excluded.file_modified,
                        date = excluded.date,
                        duration = excluded.duration,
                        track_number = excluded.track_number,
                        lufs = excluded.lufs,
                        image_checked = CASE WHEN ?11 THEN 0 ELSE songs.image_checked END
                     RETURNING id",
                )?
                .query_row(
                    params![
                        Cuid::new(),
                        track.title,
                        album_id,
                        track.file_path,
                        track.file_size,
                        track.file_modified,
                        year_str,
                        track.duration,
                        track.track_number,
                        track.lufs,
                        track.recheck_image
                    ],
                    |row| row.get(0),
                )?;

            write_profile::add(&write_profile::SONG_NS, song_started);

            let artists_started = std::time::Instant::now();
            tx.prepare_cached("DELETE FROM songs_artists WHERE song_id = ?1")?
                .execute(params![song_id])?;
            let mut credited = HashSet::new();
            for (position, artist_name) in track.artists.iter().enumerate() {
                let artist_id = artist_id_cached(&tx, cache, artist_name)?;
                if credited.insert(artist_id.clone()) {
                    vote(&mut cache.artist_votes, &artist_id, artist_name);
                    touched_artists.insert(artist_id.clone());
                }
                tx.prepare_cached(
                    "INSERT INTO songs_artists (song_id, artist_id, position) VALUES (?1, ?2, ?3)
                     ON CONFLICT(song_id, artist_id) DO UPDATE SET position = excluded.position",
                )?
                .execute(params![song_id, artist_id, position as i64])?;
            }

            write_profile::add(&write_profile::ARTIST_NS, artists_started);

            let genres_started = std::time::Instant::now();
            tx.prepare_cached("DELETE FROM songs_genres WHERE song_id = ?1")?
                .execute(params![song_id])?;
            let mut tagged = HashSet::new();
            for genre_name in track.genres {
                let genre_id = genre_id_cached(&tx, cache, genre_name)?;
                if tagged.insert(genre_id.clone()) {
                    vote(&mut cache.genre_votes, &genre_id, genre_name);
                    touched_genres.insert(genre_id.clone());
                }
                tx.prepare_cached(
                    "INSERT INTO songs_genres (song_id, genre_id) VALUES (?1, ?2)
                     ON CONFLICT(song_id, genre_id) DO NOTHING",
                )?
                .execute(params![song_id, genre_id])?;
            }

            write_profile::add(&write_profile::GENRE_NS, genres_started);
            written += 1;
        }

        apply_majority_spellings(
            &tx,
            &cache.artist_votes,
            &touched_artists,
            "SELECT name, (SELECT COUNT(*) FROM songs_artists WHERE artist_id = ?1)
             FROM artists WHERE id = ?1",
            "UPDATE OR IGNORE artists SET name = ?1 WHERE id = ?2",
        )?;
        apply_majority_spellings(
            &tx,
            &cache.genre_votes,
            &touched_genres,
            "SELECT name, (SELECT COUNT(*) FROM songs_genres WHERE genre_id = ?1)
             FROM genres WHERE id = ?1",
            "UPDATE OR IGNORE genres SET name = ?1 WHERE id = ?2",
        )?;
        apply_majority_spellings(
            &tx,
            &cache.album_votes,
            &touched_albums,
            "SELECT title, (SELECT COUNT(*) FROM songs WHERE album_id = ?1)
             FROM albums WHERE id = ?1",
            "UPDATE OR IGNORE albums SET title = ?1 WHERE id = ?2",
        )?;

        let commit_started = std::time::Instant::now();
        tx.commit()?;
        write_profile::add(&write_profile::COMMIT_NS, commit_started);
        write_profile::COMMITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(meta) = std::fs::metadata(format!("{}-wal", self.path.display())) {
            write_profile::WAL_BYTES.store(meta.len(), std::sync::atomic::Ordering::Relaxed);
        }
        Ok(written)
    }

    pub fn merge_case_duplicates(&self) -> Result<usize> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut merged = 0;

        for kind in CASE_DUPLICATES {
            let rows: Vec<(Cuid, String)> = tx
                .prepare_cached(kind.candidates_sql)?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;

            let mut keeper: Option<(String, Cuid)> = None;
            for (id, name) in rows {
                let key = fold_name(&name);
                match &keeper {
                    Some((keeper_key, keeper_id)) if *keeper_key == key => {
                        for sql in kind.merge_sql {
                            tx.prepare_cached(sql)?.execute(params![keeper_id, id])?;
                        }
                        merged += 1;
                    }
                    _ => keeper = Some((key, id)),
                }
            }
        }

        tx.commit()?;
        Ok(merged)
    }

    pub fn delete_song(&self, id: &Cuid) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM songs WHERE id = ?1", params![id])?;
        }
        self.rebuild_search_index();
        Ok(())
    }

    pub fn delete_songs_under_path(&self, dir: &str) -> Result<usize> {
        let dir = dir.trim_end_matches('/');
        let escaped = dir
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let prefix = format!("{escaped}/%");
        let deleted = {
            let conn = self.conn.lock();
            conn.execute(
                "DELETE FROM songs WHERE file_path = ?1 OR file_path LIKE ?2 ESCAPE '\\'",
                params![dir, prefix],
            )?
        };
        if deleted > 0 {
            self.rebuild_search_index();
        }
        Ok(deleted)
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
                        al.image_id,
                        (SELECT MIN(s.date) FROM songs s WHERE s.album_id = al.id) AS year
                 FROM (SELECT id, title, image_id FROM albums
                       ORDER BY title COLLATE NOCASE ASC
                       LIMIT ?1 OFFSET ?2) al",
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
                "SELECT p.id, p.name, p.image_id,
                        (SELECT COUNT(*) FROM playlist_songs pt WHERE pt.playlist_id = p.id) AS song_count
                 FROM (SELECT id, name, image_id FROM playlists
                       ORDER BY name COLLATE NOCASE ASC
                       LIMIT ?1 OFFSET ?2) p",
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
                    -- Falls through to the album before the song: grouping on
                    -- image_id alone put every track of a not-yet-resolved
                    -- album in its own group, turning the album tiles here into
                    -- a flat list of songs.
                    COALESCE(rs.image_id, 'album_' || rs.album_id, 'no_image_' || rs.id) AS group_key,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn temp_db() -> (Database, std::path::PathBuf) {
        let path = std::path::PathBuf::from(format!("/tmp/vleer_bench_{}.db", std::process::id()));
        let db = Database::new(&path).expect("failed to create test db");
        (db, path)
    }

    fn named_db(name: &str) -> (Database, std::path::PathBuf) {
        let path = std::path::PathBuf::from(format!("/tmp/vleer_{name}_{}.db", std::process::id()));
        cleanup(&path);
        let db = Database::new(&path).expect("failed to create test db");
        (db, path)
    }

    fn cleanup(path: &std::path::PathBuf) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    fn track<'a>(file_path: &'a str, recheck_image: bool) -> BatchTrack<'a> {
        BatchTrack {
            title: "Title",
            artists: &[],
            genres: &[],
            album: None,
            file_path,
            duration: 180,
            track_number: None,
            year: None,
            recheck_image,
            file_size: 1_000,
            file_modified: 42,
            lufs: None,
        }
    }

    fn song_id_for(db: &Database, file_path: &str) -> Cuid {
        db.get_song_by_path(file_path)
            .unwrap()
            .expect("song should exist")
            .id
    }

    fn image_state(db: &Database, file_path: &str) -> (Option<String>, bool) {
        let target = db
            .song_image_target(&song_id_for(db, file_path))
            .unwrap()
            .expect("target should exist");
        (target.image_id, target.image_checked)
    }

    fn artist_names(db: &Database) -> Vec<String> {
        let conn = db.conn.lock();
        let mut stmt = conn
            .prepare("SELECT name FROM artists ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn differently_cased_names_merge_into_the_majority_spelling() {
        let (db, path) = named_db("case_majority");
        let mut cache = ScanCache::default();

        let lower = ["ellis"];
        let upper = ["Ellis"];
        let tracks = [
            BatchTrack {
                artists: &lower,
                album: Some("night"),
                ..track("/music/1.flac", true)
            },
            BatchTrack {
                artists: &upper,
                album: Some("Night"),
                ..track("/music/2.flac", true)
            },
            BatchTrack {
                artists: &upper,
                album: Some("Night"),
                ..track("/music/3.flac", true)
            },
        ];
        db.upsert_tracks_batch(&tracks, &mut cache).unwrap();

        assert_eq!(artist_names(&db), vec!["Ellis".to_string()]);
        let album_id = db
            .get_song_by_path("/music/1.flac")
            .unwrap()
            .unwrap()
            .album_id;
        assert_eq!(
            db.get_song_by_path("/music/3.flac")
                .unwrap()
                .unwrap()
                .album_id,
            album_id
        );
        assert_eq!(
            db.get_album(album_id.as_ref().unwrap())
                .unwrap()
                .unwrap()
                .title,
            "Night"
        );

        let mut incremental = ScanCache::default();
        db.upsert_tracks_batch(
            &[BatchTrack {
                artists: &lower,
                ..track("/music/4.flac", true)
            }],
            &mut incremental,
        )
        .unwrap();
        assert_eq!(
            artist_names(&db),
            vec!["Ellis".to_string()],
            "one changed file must not outvote songs it did not read"
        );
        cleanup(&path);
    }

    #[test]
    fn existing_case_duplicates_merge_into_the_most_used_row() {
        let (db, path) = named_db("case_merge");
        {
            let conn = db.conn.lock();
            conn.execute_batch(
                "INSERT INTO artists (id, name, favorite) VALUES ('a_upper', 'Ellis', 0), ('a_lower', 'ellis', 1);
                 INSERT INTO albums (id, title) VALUES ('al_upper', 'Night'), ('al_lower', 'night');
                 INSERT INTO genres (id, name) VALUES ('g_upper', 'Pop'), ('g_lower', 'pop');
                 INSERT INTO songs (id, title, album_id, file_path, duration) VALUES
                    ('s1', 'One', 'al_upper', '/music/1.flac', 1),
                    ('s2', 'Two', 'al_upper', '/music/2.flac', 1),
                    ('s3', 'Three', 'al_lower', '/music/3.flac', 1);
                 INSERT INTO songs_artists (song_id, artist_id, position) VALUES
                    ('s1', 'a_upper', 0), ('s2', 'a_upper', 0), ('s3', 'a_lower', 0);
                 INSERT INTO albums_artists (album_id, artist_id, position) VALUES
                    ('al_upper', 'a_upper', 0), ('al_lower', 'a_lower', 0);
                 INSERT INTO songs_genres (song_id, genre_id) VALUES
                    ('s1', 'g_upper'), ('s2', 'g_upper'), ('s3', 'g_lower');",
            )
            .unwrap();
        }

        assert_eq!(db.merge_case_duplicates().unwrap(), 3);
        assert_eq!(db.merge_case_duplicates().unwrap(), 0);

        assert_eq!(artist_names(&db), vec!["Ellis".to_string()]);
        let conn = db.conn.lock();
        let count = |sql: &str| conn.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(count("SELECT COUNT(*) FROM albums"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM genres"), 1);
        assert_eq!(
            count("SELECT COUNT(*) FROM songs WHERE album_id = 'al_upper'"),
            3
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM songs_artists WHERE artist_id = 'a_upper'"),
            3
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM songs_genres WHERE genre_id = 'g_upper'"),
            3
        );
        assert_eq!(
            count("SELECT favorite FROM artists WHERE id = 'a_upper'"),
            1
        );
        drop(conn);
        cleanup(&path);
    }

    #[test]
    fn rescanning_keeps_a_resolved_image() {
        let (db, path) = named_db("image_keep");
        let mut cache = ScanCache::default();

        db.upsert_tracks_batch(&[track("/music/a.flac", true)], &mut cache)
            .unwrap();
        let song = song_id_for(&db, "/music/a.flac");
        db.store_song_image(&song, "img1", Some(b"jpegbytes"))
            .unwrap();
        assert_eq!(
            image_state(&db, "/music/a.flac"),
            (Some("img1".to_string()), true)
        );

        db.upsert_tracks_batch(&[track("/music/a.flac", false)], &mut cache)
            .unwrap();

        assert_eq!(
            image_state(&db, "/music/a.flac"),
            (Some("img1".to_string()), true),
            "a forced rescan must not drop a resolved cover"
        );
        cleanup(&path);
    }

    #[test]
    fn a_changed_file_is_rechecked_but_keeps_its_image() {
        let (db, path) = named_db("image_recheck");
        let mut cache = ScanCache::default();

        db.upsert_tracks_batch(&[track("/music/b.flac", true)], &mut cache)
            .unwrap();
        let song = song_id_for(&db, "/music/b.flac");
        db.store_song_image(&song, "img1", Some(b"jpegbytes"))
            .unwrap();

        db.upsert_tracks_batch(&[track("/music/b.flac", true)], &mut cache)
            .unwrap();

        let (image_id, checked) = image_state(&db, "/music/b.flac");
        assert_eq!(
            image_id,
            Some("img1".to_string()),
            "the old cover stays on screen until a new one resolves"
        );
        assert!(
            !checked,
            "image_checked must reset on a changed file -- ON CONFLICT DO UPDATE \
             never re-applies the column default, so this has to be written explicitly"
        );
        cleanup(&path);
    }

    #[test]
    fn a_file_without_an_image_is_remembered_as_checked() {
        let (db, path) = named_db("image_missing");
        let mut cache = ScanCache::default();

        db.upsert_tracks_batch(&[track("/music/c.flac", true)], &mut cache)
            .unwrap();
        assert_eq!(image_state(&db, "/music/c.flac"), (None, false));

        let song = song_id_for(&db, "/music/c.flac");
        db.mark_song_image_missing(&song).unwrap();

        assert_eq!(image_state(&db, "/music/c.flac"), (None, true));
        assert!(
            db.songs_needing_image(10).unwrap().is_empty(),
            "a checked song must not come back round on the warm pass"
        );
        cleanup(&path);
    }

    #[test]
    fn identical_images_are_stored_once_and_fill_the_album() {
        let (db, path) = named_db("image_dedupe");
        let mut cache = ScanCache::default();

        let mut first = track("/music/d1.flac", true);
        first.album = Some("An Album");
        let mut second = track("/music/d2.flac", true);
        second.album = Some("An Album");
        db.upsert_tracks_batch(&[first, second], &mut cache)
            .unwrap();

        let d1 = song_id_for(&db, "/music/d1.flac");
        let d2 = song_id_for(&db, "/music/d2.flac");

        assert!(!db.image_exists("shared").unwrap());
        db.store_song_image(&d1, "shared", Some(b"jpegbytes"))
            .unwrap();
        assert!(db.image_exists("shared").unwrap());

        db.store_song_image(&d2, "shared", None).unwrap();

        let count: i64 = {
            let conn = db.conn.lock();
            conn.query_row("SELECT COUNT(*) FROM images", [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(count, 1, "one picture, one row");

        let album_id = db
            .get_song_by_path("/music/d1.flac")
            .unwrap()
            .unwrap()
            .album_id
            .expect("song should have an album");
        assert!(matches!(
            db.album_image_target(&album_id).unwrap(),
            Some(AlbumImageTarget::Resolved(id)) if id == "shared"
        ));
        cleanup(&path);
    }

    #[test]
    fn an_album_with_no_images_stops_offering_candidates() {
        let (db, path) = named_db("album_negative");
        let mut cache = ScanCache::default();

        let mut only = track("/music/e.flac", true);
        only.album = Some("Imageless");
        db.upsert_tracks_batch(&[only], &mut cache).unwrap();

        let album_id = db
            .get_song_by_path("/music/e.flac")
            .unwrap()
            .unwrap()
            .album_id
            .unwrap();
        assert!(matches!(
            db.album_image_target(&album_id).unwrap(),
            Some(AlbumImageTarget::Candidates(_))
        ));

        db.mark_song_image_missing(&song_id_for(&db, "/music/e.flac"))
            .unwrap();

        assert!(
            db.album_image_target(&album_id).unwrap().is_none(),
            "with every track checked the album must offer nothing"
        );
        cleanup(&path);
    }

    #[test]
    #[ignore]
    fn bench_write_contention() {
        let (db, path) = named_db("contention");
        let songs: usize = std::env::var("VLEER_BENCH_SONGS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);

        let titles: Vec<String> = (0..songs).map(|i| format!("Song {i}")).collect();
        let paths: Vec<String> = (0..songs).map(|i| format!("/music/c_{i}.flac")).collect();

        let artist_names: Vec<String> = (0..songs / 10).map(|i| format!("Artist {i}")).collect();
        let album_names: Vec<String> = (0..songs / 4).map(|i| format!("Album {i}")).collect();

        let insert_all = |db: &Database| {
            let mut cache = ScanCache::default();
            for chunk in (0..songs).collect::<Vec<_>>().chunks(256) {
                let artists: Vec<[&str; 1]> = chunk
                    .iter()
                    .map(|&i| [artist_names[i % artist_names.len()].as_str()])
                    .collect();
                let batch: Vec<BatchTrack<'_>> = chunk
                    .iter()
                    .enumerate()
                    .map(|(slot, &i)| BatchTrack {
                        title: &titles[i],
                        artists: &artists[slot],
                        genres: &[],
                        album: Some(&album_names[i % album_names.len()]),
                        file_path: &paths[i],
                        duration: 180,
                        track_number: None,
                        year: None,
                        recheck_image: true,
                        file_size: 1_000,
                        file_modified: 42,
                        lufs: None,
                    })
                    .collect();
                db.upsert_tracks_batch(&batch, &mut cache).unwrap();
            }
        };

        write_profile::reset();
        let t = Instant::now();
        insert_all(&db);
        let alone = t.elapsed();
        let (l, a, so, ar, g, c) = write_profile::snapshot();
        let wal = std::fs::metadata(format!("{}-wal", path.display()))
            .map(|m| m.len())
            .unwrap_or(0);
        println!(
            "  alone:        lock {l:?}, albums {a:?}, songs {so:?}, artists {ar:?}, \
genres {g:?}, commit {c:?}, wal {}MB",
            wal / 1_048_576
        );

        let (db2, path2) = named_db("contention_b");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = {
            let db2 = db2.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut n = 0u64;
                let blob = vec![0u8; 32 * 1024];
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if db2.write_image_row(&format!("contend{n}"), &blob).is_ok() {
                        n += 1;
                    }
                }
                n
            })
        };

        let t = Instant::now();
        insert_all(&db2);
        let contended = t.elapsed();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let writes = writer.join().unwrap();

        let (db3, path3) = named_db("contention_c");
        let stop_r = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let db3 = db3.clone();
            let stop_r = stop_r.clone();
            std::thread::spawn(move || {
                let mut n = 0u64;
                while !stop_r.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = db3.get_songs(None, SongSort::Default, true, 0, 100);
                    let _ = db3.get_albums("", 0, 60);
                    let _ = db3.get_recently_added_items(100);
                    n += 1;
                }
                n
            })
        };

        write_profile::reset();
        let t = Instant::now();
        insert_all(&db3);
        let read_contended = t.elapsed();
        stop_r.store(true, std::sync::atomic::Ordering::Relaxed);
        let reads = reader.join().unwrap();
        let (l, a, so, ar, g, c) = write_profile::snapshot();
        let wal = std::fs::metadata(format!("{}-wal", path3.display()))
            .map(|m| m.len())
            .unwrap_or(0);
        println!(
            "  with readers: lock {l:?}, albums {a:?}, songs {so:?}, artists {ar:?}, \
genres {g:?}, commit {c:?}, wal {}MB",
            wal / 1_048_576
        );
        println!(
            "insert {songs} songs + UI reads: {read_contended:>12.2?}  ({reads} query rounds)"
        );
        println!(
            "read-contention slowdown: {:.1}x",
            read_contended.as_secs_f64() / alone.as_secs_f64()
        );
        cleanup(&path3);

        let (db4, path4) = named_db("contention_d");
        let stop_c = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let burners: Vec<_> = (0..std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8))
            .map(|_| {
                let stop_c = stop_c.clone();
                std::thread::spawn(move || {
                    let mut x = 0u64;
                    while !stop_c.load(std::sync::atomic::Ordering::Relaxed) {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                        std::hint::black_box(x);
                    }
                })
            })
            .collect();

        let t = Instant::now();
        insert_all(&db4);
        let cpu_contended = t.elapsed();
        stop_c.store(true, std::sync::atomic::Ordering::Relaxed);
        for b in burners {
            let _ = b.join();
        }
        println!("insert {songs} songs + busy cores: {cpu_contended:>12.2?}");
        println!(
            "cpu-contention slowdown: {:.1}x",
            cpu_contended.as_secs_f64() / alone.as_secs_f64()
        );
        cleanup(&path4);

        let (db5, path5) = named_db("contention_e");
        let blob = vec![7u8; 40 * 1024];
        for i in 0..1_000 {
            db5.write_image_row(&format!("bloat{i}"), &blob).unwrap();
        }
        let t = Instant::now();
        insert_all(&db5);
        let bloated = t.elapsed();
        let size = std::fs::metadata(&path5).map(|m| m.len()).unwrap_or(0);
        println!(
            "insert {songs} songs, {}MB store: {bloated:>12.2?}",
            size / 1_048_576
        );
        println!(
            "bloat slowdown: {:.1}x",
            bloated.as_secs_f64() / alone.as_secs_f64()
        );
        cleanup(&path5);

        let t = Instant::now();
        insert_all(&db);
        let reupsert = t.elapsed();

        println!("insert {songs} songs alone:      {alone:>12.2?}");
        println!("re-upsert same {songs} songs:    {reupsert:>12.2?}");
        println!(
            "re-upsert vs cold: {:.1}x",
            reupsert.as_secs_f64() / alone.as_secs_f64()
        );
        println!(
            "insert {songs} songs contended:  {contended:>12.2?}  ({writes} competing writes)"
        );
        println!(
            "slowdown: {:.1}x",
            contended.as_secs_f64() / alone.as_secs_f64()
        );
        cleanup(&path);
        cleanup(&path2);
    }

    #[test]
    fn bench_sqlite_operations() {
        let (db, path) = temp_db();

        fn scaled(name: &str, default: usize) -> usize {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        let songs = scaled("VLEER_BENCH_SONGS", 10_000);
        let artists_count = scaled("VLEER_BENCH_ARTISTS", 100);
        let albums_count = scaled("VLEER_BENCH_ALBUMS", 500);

        let artist_names: Vec<String> = (0..artists_count).map(|i| format!("Artist {i}")).collect();

        let titles: Vec<String> = (0..songs).map(|i| format!("Song Title {i}")).collect();
        let paths: Vec<String> = (0..songs).map(|i| format!("/music/song_{i}.mp3")).collect();
        let albums: Vec<String> = (0..albums_count).map(|i| format!("Album {i}")).collect();

        write_profile::reset();
        let t = Instant::now();
        let mut cache = ScanCache::default();
        for chunk in (0..songs).collect::<Vec<_>>().chunks(256) {
            let artists: Vec<[&str; 1]> = chunk
                .iter()
                .map(|i| [artist_names[i % artists_count].as_str()])
                .collect();
            let batch: Vec<BatchTrack<'_>> = chunk
                .iter()
                .enumerate()
                .map(|(slot, &i)| BatchTrack {
                    title: &titles[i],
                    artists: &artists[slot],
                    genres: &[],
                    album: Some(&albums[i % albums_count]),
                    file_path: &paths[i],
                    duration: 180 + (i as i32 % 300),
                    track_number: Some((i as i32 % 20) + 1),
                    year: Some(2000 + (i as i32 % 24)),
                    recheck_image: true,
                    file_size: 1_000_000,
                    file_modified: i as i64,
                    lufs: None,
                })
                .collect();
            db.upsert_tracks_batch(&batch, &mut cache).unwrap();
        }
        println!("insert {songs} songs:         {:>10?}", t.elapsed());
        let (lock, albums, song_rows, artists, genres, commit) = write_profile::snapshot();
        println!(
            "  breakdown: lock {lock:?}, albums {albums:?}, songs {song_rows:?}, \
artists {artists:?}, genres {genres:?}, commit {commit:?}"
        );

        let t = Instant::now();
        db.rebuild_search_index();
        println!("rebuild search index:        {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..100 {
            db.get_songs_count(None).unwrap();
        }
        println!("get_songs_count       x100:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for i in 0..100 {
            db.get_songs(None, SongSort::Default, true, i * 50, 50)
                .unwrap();
        }
        println!("get_songs paginated   x100:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..50 {
            db.get_songs(Some("Song"), SongSort::Default, true, 0, 20)
                .unwrap();
        }
        println!("get_songs FTS search   x50:  {:>10?}", t.elapsed());

        let songs = db.get_songs(None, SongSort::Default, true, 0, 1).unwrap();
        let song_id = songs[0].id.clone();
        let t = Instant::now();
        for _ in 0..1000 {
            db.get_song(&song_id).unwrap();
        }
        println!("get_song by id       x1000:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for i in 0..100 {
            db.get_albums("", i * 5, 5).unwrap();
        }
        println!("get_albums paginated  x100:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..100 {
            db.get_albums_count("").unwrap();
        }
        println!("get_albums_count      x100:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..10 {
            db.get_recently_added_items(100).unwrap();
        }
        println!("get_recently_added     x10:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..10 {
            db.get_recently_played_items(100).unwrap();
        }
        println!("get_recently_played    x10:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..10 {
            db.get_artists("", 0, 60).unwrap();
        }
        println!("get_artists paginated  x10:  {:>10?}", t.elapsed());

        cleanup(&path);
    }
}
