use super::{Database, collect_mapped};
use crate::data::models::{Cuid, Song, SongListItem, SongSort, StoredLyrics};
use anyhow::Result;
use rusqlite::{OptionalExtension, ToSql, params};
use std::collections::HashMap;

impl Database {
    pub fn get_song(&self, id: &Cuid) -> Result<Option<Song>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT s.*,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM songs s
             WHERE s.id = ?1",
        )?;
        let row = stmt.query_row(params![id], Song::from_row).optional()?;
        Ok(row)
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
            .query_row(params![file_path], Song::from_row)
            .optional()?;
        Ok(row)
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
            return collect_mapped::<SongListItem, _>(
                &conn,
                &sql,
                params![limit, offset],
                SongListItem::from_row,
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
        collect_mapped::<Song, _>(&conn, &sql, params.as_slice(), Song::from_row)
    }

    pub fn count_songs(&self, query: Option<&str>) -> Result<i64> {
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

    pub fn get_song_ids(
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

    pub fn get_song_omm_ids(&self, song_id: &Cuid) -> Result<(Option<String>, Option<String>)> {
        let conn = self.conn.lock();
        let ids = conn
            .prepare_cached(
                "SELECT 'omm:song:' || NULLIF(s.omm_id, ''),
                        (SELECT 'omm:artist:' || NULLIF(ar.omm_id, '') FROM songs_artists sa
                         JOIN artists ar ON ar.id = sa.artist_id
                         WHERE sa.song_id = s.id AND NULLIF(ar.omm_id, '') IS NOT NULL
                         ORDER BY sa.position LIMIT 1)
                 FROM songs s WHERE s.id = ?1",
            )?
            .query_row(params![song_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        Ok(ids.unwrap_or((None, None)))
    }

    pub fn get_lyrics(&self, song_id: &Cuid, retry_days: u32) -> Result<Option<StoredLyrics>> {
        let conn = self.conn.lock();
        let row = conn
            .prepare_cached(
                "SELECT source, synced, instrumental, content,
                        fetched_at < DATETIME('now', '-' || ?2 || ' days') AS stale
                 FROM lyrics WHERE song_id = ?1",
            )?
            .query_row(params![song_id, retry_days], |row| {
                Ok(StoredLyrics {
                    source: row.get("source")?,
                    synced: row.get("synced")?,
                    instrumental: row.get("instrumental")?,
                    content: row.get("content")?,
                    stale: row.get("stale")?,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn upsert_lyrics(
        &self,
        song_id: &Cuid,
        source: &str,
        synced: bool,
        instrumental: bool,
        content: &str,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.prepare_cached(
            "INSERT INTO lyrics (song_id, source, synced, instrumental, content, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, DATETIME('now'))
             ON CONFLICT(song_id) DO UPDATE SET
                source = excluded.source,
                synced = excluded.synced,
                instrumental = excluded.instrumental,
                content = excluded.content,
                fetched_at = excluded.fetched_at",
        )?
        .execute(params![song_id, source, synced, instrumental, content])?;
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

    pub fn delete_songs_in_dir(&self, dir: &str) -> Result<usize> {
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
}

pub(super) fn fetch_songs_by_ids(
    conn: &rusqlite::Connection,
    ids: &[Cuid],
) -> Result<Vec<SongListItem>> {
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
        .query_map(params.as_slice(), SongListItem::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

pub(super) fn song_order(sort: SongSort, ascending: bool) -> &'static str {
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

        SongSort::Default => "s.date_added DESC, s.rowid DESC",
    }
}

#[cfg(test)]
mod tests {

    use crate::data::db::test_util::*;
    use crate::data::db::{BatchTrack, ScanCache};
    use crate::data::models::{Cuid, SongSort};

    #[test]
    fn default_song_order_breaks_same_second_ties_by_insertion_order() {
        let (db, path) = named_db("song_recency");

        db.upsert_tracks(
            &[
                BatchTrack {
                    audio_hash: "1",
                    ..track("/music/1.flac", false)
                },
                BatchTrack {
                    audio_hash: "2",
                    ..track("/music/2.flac", false)
                },
                BatchTrack {
                    audio_hash: "3",
                    ..track("/music/3.flac", false)
                },
            ],
            &mut ScanCache::default(),
        )
        .unwrap();

        let songs = db.get_songs(None, SongSort::Default, false, 0, 10).unwrap();
        let ids: Vec<Cuid> = songs.into_iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                song_id_for(&db, "/music/3.flac"),
                song_id_for(&db, "/music/2.flac"),
                song_id_for(&db, "/music/1.flac"),
            ]
        );
        cleanup(&path);
    }
}
