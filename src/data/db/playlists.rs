use super::{Database, collect_mapped};
use crate::data::models::{Cuid, Playlist, PlaylistListItem, PlaylistTrack};
use anyhow::Result;
use rusqlite::{OptionalExtension, ToSql, params};
use std::collections::HashMap;

impl Database {
    pub fn get_playlist(&self, id: &Cuid) -> Result<Option<Playlist>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT * FROM playlists WHERE id = ?1",
                params![id],
                Playlist::from_row,
            )
            .optional()?;
        Ok(row)
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
            return collect_mapped::<PlaylistListItem, _>(
                &conn,
                "SELECT p.id, p.name, p.image_id,
                        (SELECT COUNT(*) FROM playlist_songs pt WHERE pt.playlist_id = p.id) AS song_count
                 FROM (SELECT id, name, image_id FROM playlists
                       ORDER BY name COLLATE NOCASE ASC
                       LIMIT ?1 OFFSET ?2) p",
                params![limit, offset],
                PlaylistListItem::from_row,
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

    pub fn count_playlists(&self, query: &str) -> Result<i64> {
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

    pub fn get_playlist_songs(&self, playlist_id: &Cuid) -> Result<Vec<PlaylistTrack>> {
        let conn = self.conn.lock();
        collect_mapped::<PlaylistTrack, _>(
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
            PlaylistTrack::from_row,
        )
    }

    pub fn upsert_playlist(
        &self,
        id: &Cuid,
        name: &str,
        image_id: Option<&str>,
        pinned: bool,
    ) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute(
                "INSERT INTO playlists (id, name, image_id, pinned)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    image_id = excluded.image_id,
                    pinned = excluded.pinned,
                    date_updated = DATETIME('now')",
                params![id, name, image_id, pinned],
            )?;
        }
        let mut index = self.search_index.lock();
        index.upsert_playlist(id.clone(), name.to_string(), image_id.map(String::from));
        Ok(())
    }

    pub fn upsert_playlist_song(&self, playlist_id: &Cuid, song_id: &Cuid) -> Result<()> {
        let id = Cuid::for_playlist_song(playlist_id, song_id);
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO playlist_songs (id, playlist_id, song_id, position)
             VALUES (?1, ?2, ?3, COALESCE((SELECT MAX(position) FROM playlist_songs WHERE playlist_id = ?2), -1) + 1)
             ON CONFLICT(playlist_id, song_id) DO NOTHING",
            params![id, playlist_id, song_id],
        )?;
        Ok(())
    }

    pub fn set_playlist_order(&self, playlist_id: &Cuid, song_ids: &[Cuid]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for (position, song_id) in song_ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlist_songs SET position = ?1 WHERE playlist_id = ?2 AND song_id = ?3",
                params![position as i64, playlist_id, song_id],
            )?;
        }
        tx.commit()?;
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
}

pub(super) fn fetch_playlists_by_ids(
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
        .query_map(params.as_slice(), PlaylistListItem::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}
