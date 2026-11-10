use super::{Database, collect_mapped};
use crate::data::models::{Cuid, GenreListItem};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

impl Database {
    pub fn get_genres(&self, query: &str) -> Result<Vec<GenreListItem>> {
        let pattern = format!(
            "%{}%",
            query
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT g.id, g.name, COUNT(sg.song_id)
             FROM genres g
             JOIN songs_genres sg ON sg.genre_id = g.id
             WHERE g.name LIKE ?1 ESCAPE '\\'
             GROUP BY g.id
             ORDER BY g.name COLLATE NOCASE ASC",
        )?;
        let rows = stmt
            .query_map(params![pattern], |row| {
                Ok(GenreListItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    song_count: row.get::<_, i64>(2)?.max(0) as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_genre_song_ids(&self, genre_id: &Cuid) -> Result<Vec<Cuid>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT song_id FROM songs_genres WHERE genre_id = ?1 ORDER BY rowid",
        )?;
        let ids = stmt
            .query_map(params![genre_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<Cuid>>>()?;
        Ok(ids)
    }

    pub fn get_genre_image_id(&self, genre_id: &Cuid) -> Result<Option<String>> {
        let conn = self.conn.lock();
        let id = conn
            .prepare_cached("SELECT image_id FROM genres WHERE id = ?1")?
            .query_row(params![genre_id], |row| row.get(0))
            .optional()?
            .flatten();
        Ok(id)
    }

    pub fn get_genre_image_ids(&self, genre_id: &Cuid) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT s.image_id FROM songs s
             JOIN songs_genres sg ON sg.song_id = s.id
             WHERE sg.genre_id = ?1 AND s.image_id IS NOT NULL
             UNION
             SELECT a.image_id FROM albums a
             JOIN songs s ON s.album_id = a.id
             JOIN songs_genres sg ON sg.song_id = s.id
             WHERE sg.genre_id = ?1 AND a.image_id IS NOT NULL",
        )?;
        let ids = stmt
            .query_map(params![genre_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    pub fn get_genre_covers(&self) -> Result<Vec<(Cuid, Option<String>)>> {
        let conn = self.conn.lock();
        collect_mapped(
            &conn,
            "SELECT g.id, g.image_id
             FROM genres g
             WHERE EXISTS (SELECT 1 FROM songs_genres sg WHERE sg.genre_id = g.id)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    pub fn store_genre_cover(&self, genre_id: &Cuid, image_id: &str, data: &[u8]) -> Result<()> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;
        let previous: Option<String> = tx
            .prepare_cached("SELECT image_id FROM genres WHERE id = ?1")?
            .query_row(params![genre_id], |row| row.get(0))
            .optional()?
            .flatten();
        tx.prepare_cached(
            "INSERT INTO images (id, data) VALUES (?1, ?2)
             ON CONFLICT(id) DO NOTHING",
        )?
        .execute(params![image_id, data])?;
        tx.prepare_cached("UPDATE genres SET image_id = ?2 WHERE id = ?1")?
            .execute(params![genre_id, image_id])?;
        if let Some(previous) = previous.filter(|previous| previous != image_id) {
            tx.prepare_cached("DELETE FROM images WHERE id = ?1")?
                .execute(params![previous])?;
        }
        tx.commit()?;
        Ok(())
    }
}
