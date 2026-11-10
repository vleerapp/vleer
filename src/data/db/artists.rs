use super::{Database, collect_mapped};
use crate::data::models::{AlbumListItem, Artist, ArtistListItem, Cuid};
use anyhow::Result;
use rusqlite::{OptionalExtension, ToSql, params};
use std::collections::HashMap;

impl Database {
    pub fn get_artist(&self, id: &Cuid) -> Result<Option<Artist>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT * FROM artists WHERE id = ?1",
                params![id],
                Artist::from_row,
            )
            .optional()?;
        Ok(row)
    }

    pub fn get_artist_by_name(&self, name: &str) -> Result<Option<Artist>> {
        let conn = self.conn.lock();
        let row = conn
            .prepare_cached("SELECT * FROM artists WHERE name = ?1")?
            .query_row(params![name], Artist::from_row)
            .optional()?;
        Ok(row)
    }

    pub fn get_artists(&self, query: &str, offset: i64, limit: i64) -> Result<Vec<ArtistListItem>> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            return collect_mapped::<ArtistListItem, _>(
                &conn,
                "SELECT ar.id, ar.name, ar.image_id
                 FROM artists ar
                 ORDER BY ar.name COLLATE NOCASE ASC
                 LIMIT ?1 OFFSET ?2",
                params![limit, offset],
                ArtistListItem::from_row,
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

    pub fn count_artists(&self, query: &str) -> Result<usize> {
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

    pub fn get_artist_albums(
        &self,
        artist_id: &Cuid,
        appears_on: bool,
    ) -> Result<Vec<AlbumListItem>> {
        let membership = if appears_on {
            "al.id IN (SELECT s.album_id FROM songs s JOIN songs_artists sa ON sa.song_id = s.id
                       WHERE sa.artist_id = ?1 AND s.album_id IS NOT NULL)
             AND al.id NOT IN (SELECT album_id FROM albums_artists WHERE artist_id = ?1)"
        } else {
            "al.id IN (SELECT album_id FROM albums_artists WHERE artist_id = ?1)"
        };
        let conn = self.conn.lock();
        collect_mapped::<AlbumListItem, _>(
            &conn,
            &format!(
                "SELECT al.id, al.title,
                        (SELECT GROUP_CONCAT(name, ', ')
                         FROM (SELECT ar.name FROM albums_artists aa JOIN artists ar ON aa.artist_id = ar.id WHERE aa.album_id = al.id ORDER BY aa.position)) AS artist_name,
                        al.image_id,
                        (SELECT MIN(s.date) FROM songs s WHERE s.album_id = al.id) AS year
                 FROM albums al
                 WHERE {membership}
                 ORDER BY year DESC, al.title COLLATE NOCASE"
            ),
            params![artist_id],
            AlbumListItem::from_row,
        )
    }

    pub fn get_artist_song_ids(&self, artist_id: &Cuid) -> Result<Vec<Cuid>> {
        let conn = self.conn.lock();
        collect_mapped::<Cuid, _>(
            &conn,
            "SELECT s.id
             FROM songs s
             JOIN songs_artists sa ON sa.song_id = s.id
             LEFT JOIN albums al ON al.id = s.album_id
             WHERE sa.artist_id = ?1
             ORDER BY
                CASE
                    WHEN s.album_id IS NULL THEN 1
                    WHEN EXISTS (SELECT 1 FROM albums_artists aa
                                 WHERE aa.album_id = s.album_id AND aa.artist_id = ?1) THEN 0
                    ELSE 2
                END,
                (SELECT MIN(s2.date) FROM songs s2 WHERE s2.album_id = s.album_id) DESC,
                al.title COLLATE NOCASE, s.track_number, s.title COLLATE NOCASE",
            params![artist_id],
            |row| row.get(0),
        )
    }
}

pub(super) fn fetch_artists_by_ids(
    conn: &rusqlite::Connection,
    ids: &[Cuid],
) -> Result<Vec<ArtistListItem>> {
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
        .query_map(params.as_slice(), ArtistListItem::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

#[cfg(test)]
mod tests {

    use crate::data::db::test_util::*;
    use crate::data::db::{ArtistMetadata, BatchTrack, ScanCache};
    use crate::data::models::Cuid;
    use rusqlite::params;

    #[test]
    fn merging_an_artist_keeps_its_already_stored_image() {
        let (db, path) = named_db("artist_merge_image");
        db.upsert_tracks(
            &[BatchTrack {
                artists: &["Fish", "Fish & Chips"],
                ..track("/music/1.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let fish = db.get_artist_by_name("Fish").unwrap().unwrap().id;
        let combined = db.get_artist_by_name("Fish & Chips").unwrap().unwrap().id;
        db.store_artist_metadata(
            &fish,
            &ArtistMetadata {
                omm_id: "1003132717",
                name: "Fish",
                image: Some(("old", Some(b"old"))),
            },
        )
        .unwrap();
        db.write_image_row("shared", b"shared").unwrap();
        db.image_write_conn
            .lock()
            .execute(
                "UPDATE artists SET image_id = 'shared' WHERE id = ?1",
                params![combined],
            )
            .unwrap();

        let merged = db
            .store_artist_metadata(
                &combined,
                &ArtistMetadata {
                    omm_id: "1003132717",
                    name: "Fish",
                    image: Some(("shared", None)),
                },
            )
            .unwrap();
        assert_eq!(merged, Some(fish.clone()));
        let artist = db.get_artist(&fish).unwrap().unwrap();
        assert_eq!(artist.image_id.as_deref(), Some("shared"));
        assert!(db.has_image("shared").unwrap());
        cleanup(&path);
    }

    #[test]
    fn genres_and_artists_get_derived_ids() {
        let (db, path) = named_db("derived_ids");
        db.upsert_tracks(
            &[BatchTrack {
                artists: &["Ellis"],
                genres: &["Pop"],
                ..track("/music/g.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        assert_eq!(
            db.get_artist_by_name("Ellis").unwrap().unwrap().id,
            Cuid::for_artist("ellis")
        );
        let genre: String = db
            .conn
            .lock()
            .query_row("SELECT id FROM genres", [], |row| row.get(0))
            .unwrap();
        assert_eq!(genre, Cuid::for_genre("POP").into_string());
        cleanup(&path);
    }
}
