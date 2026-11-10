use super::{Database, placeholders};
use crate::data::models::{Cuid, Image};
use anyhow::Result;
use rusqlite::{OptionalExtension, params, params_from_iter};
use std::collections::HashSet;

pub(super) const ALBUM_IMAGE_ATTEMPTS: i64 = 3;

pub struct SongImageTarget {
    pub file_path: String,
    pub image_id: Option<String>,

    pub image_checked: bool,
}

pub enum AlbumImageTarget {
    Resolved(String),
    Candidates(Vec<Cuid>),
}

impl Database {
    pub fn get_image(&self, id: &str) -> Result<Option<Image>> {
        let conn = self.conn.lock();
        let row = conn
            .query_row(
                "SELECT * FROM images WHERE id = ?1",
                params![id],
                Image::from_row,
            )
            .optional()?;
        Ok(row)
    }

    pub fn get_image_blobs(&self, image_ids: &[String]) -> Result<Vec<Vec<u8>>> {
        let conn = self.image_conn.lock();
        let mut stmt = conn.prepare_cached("SELECT data FROM images WHERE id = ?1")?;
        let mut blobs = Vec::with_capacity(image_ids.len());
        for id in image_ids {
            if let Some(data) = stmt.query_row(params![id], |row| row.get(0)).optional()? {
                blobs.push(data);
            }
        }
        Ok(blobs)
    }

    pub fn has_image(&self, image_id: &str) -> Result<bool> {
        let conn = self.image_write_conn.lock();
        let found: Option<i64> = conn
            .prepare_cached("SELECT 1 FROM images WHERE id = ?1")?
            .query_row(params![image_id], |row| row.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    pub fn get_song_image_target(&self, song_id: &Cuid) -> Result<Option<SongImageTarget>> {
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

    pub fn get_album_image_target(&self, album_id: &Cuid) -> Result<Option<AlbumImageTarget>> {
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

    pub fn get_song_album_image(&self, song_id: &Cuid) -> Result<Option<String>> {
        let conn = self.image_write_conn.lock();
        let image: Option<String> = conn
            .prepare_cached(
                "SELECT a.image_id FROM albums a
                 JOIN songs s ON s.album_id = a.id WHERE s.id = ?1",
            )?
            .query_row(params![song_id], |row| row.get(0))
            .optional()?
            .flatten();
        Ok(image)
    }

    pub fn album_needs_image(&self, album_id: &Cuid) -> Result<bool> {
        let conn = self.image_write_conn.lock();
        let missing: Option<bool> = conn
            .prepare_cached("SELECT image_id IS NULL FROM albums WHERE id = ?1")?
            .query_row(params![album_id], |row| row.get(0))
            .optional()?;
        Ok(missing.unwrap_or(false))
    }

    pub fn song_album_needs_image(&self, song_id: &Cuid) -> Result<bool> {
        let conn = self.image_write_conn.lock();
        let missing: Option<bool> = conn
            .prepare_cached(
                "SELECT a.image_id IS NULL FROM albums a
                 JOIN songs s ON s.album_id = a.id WHERE s.id = ?1",
            )?
            .query_row(params![song_id], |row| row.get(0))
            .optional()?;
        Ok(missing.unwrap_or(false))
    }

    pub fn song_artist_needs_image(&self, song_id: &Cuid) -> Result<bool> {
        let conn = self.image_write_conn.lock();
        let found: Option<i64> = conn
            .prepare_cached(
                "SELECT 1 FROM songs_artists sa
                 JOIN artists ar ON ar.id = sa.artist_id
                 WHERE sa.song_id = ?1 AND sa.position = 0 AND ar.image_id IS NULL",
            )?
            .query_row(params![song_id], |row| row.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    pub fn get_songs_without_image(&self, limit: i64) -> Result<Vec<Cuid>> {
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

    pub fn get_songs_without_image_in(&self, ids: &[Cuid]) -> Result<Vec<Cuid>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.image_write_conn.lock();
        let sql = format!(
            "SELECT id FROM songs WHERE image_checked = 0 AND image_id IS NULL AND id IN ({})",
            placeholders(ids.len())
        );
        let found: HashSet<Cuid> = conn
            .prepare(&sql)?
            .query_map(params_from_iter(ids), |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids
            .iter()
            .filter(|id| found.contains(*id))
            .cloned()
            .collect())
    }

    pub fn get_album_songs_without_image(&self, album_ids: &[Cuid]) -> Result<Vec<Cuid>> {
        if album_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.image_write_conn.lock();
        let sql = format!(
            "SELECT s.album_id, s.id FROM songs s
             JOIN albums a ON a.id = s.album_id
             WHERE s.image_checked = 0 AND s.image_id IS NULL AND a.image_id IS NULL
               AND s.album_id IN ({})
             ORDER BY s.track_number, s.id",
            placeholders(album_ids.len())
        );
        let rows: Vec<(Cuid, Cuid)> = conn
            .prepare(&sql)?
            .query_map(params_from_iter(album_ids), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut seen = HashSet::new();
        Ok(rows
            .into_iter()
            .filter(|(album_id, _)| seen.insert(album_id.clone()))
            .map(|(_, song_id)| song_id)
            .collect())
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

    pub fn store_song_artist_image(
        &self,
        song_id: &Cuid,
        image_id: &str,
        data: Option<&[u8]>,
    ) -> Result<bool> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;

        if let Some(data) = data {
            tx.prepare_cached(
                "INSERT INTO images (id, data) VALUES (?1, ?2)
                 ON CONFLICT(id) DO NOTHING",
            )?
            .execute(params![image_id, data])?;
        }

        let updated = tx
            .prepare_cached(
                "UPDATE artists SET image_id = ?2
                 WHERE image_id IS NULL
                   AND id = (SELECT artist_id FROM songs_artists WHERE song_id = ?1 AND position = 0)",
            )?
            .execute(params![song_id, image_id])?;

        if updated > 0 {
            tx.commit()?;
        }
        Ok(updated > 0)
    }

    pub fn mark_song_image_missing(&self, song_id: &Cuid) -> Result<()> {
        let conn = self.image_write_conn.lock();
        conn.prepare_cached("UPDATE songs SET image_checked = 1 WHERE id = ?1")?
            .execute(params![song_id])?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn delete_image(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM images WHERE id = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db::test_util::*;
    use crate::data::db::{ArtistMetadata, BatchTrack, ScanCache};
    use crate::data::models::Cuid;

    #[test]
    fn tag_artist_images_fill_gaps_but_never_replace_canonical_artwork() {
        let (db, path) = named_db("artist_tag_image");
        let mut cache = ScanCache::default();
        db.upsert_tracks(
            &[BatchTrack {
                artists: &["Lead", "Feature"],
                ..track("/music/1.flac", false)
            }],
            &mut cache,
        )
        .unwrap();
        let song = song_id_for(&db, "/music/1.flac");
        let lead = db.get_artist_by_name("Lead").unwrap().unwrap().id;
        let feature = db.get_artist_by_name("Feature").unwrap().unwrap().id;
        let image_of = |id: &Cuid| db.get_artist(id).unwrap().unwrap().image_id;
        let image_rows = || {
            db.conn
                .lock()
                .query_row("SELECT COUNT(*) FROM images", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        };

        assert!(db.song_artist_needs_image(&song).unwrap());
        db.store_song_artist_image(&song, "tag", Some(b"tag"))
            .unwrap();
        assert_eq!(image_of(&lead).as_deref(), Some("tag"));
        assert_eq!(image_of(&feature), None);
        assert!(!db.song_artist_needs_image(&song).unwrap());

        db.store_song_artist_image(&song, "other", Some(b"other"))
            .unwrap();
        assert_eq!(image_of(&lead).as_deref(), Some("tag"));
        assert_eq!(image_rows(), 1);

        db.store_artist_metadata(
            &lead,
            &ArtistMetadata {
                omm_id: "3456789012",
                name: "Lead",
                image: Some(("omm", Some(b"omm"))),
            },
        )
        .unwrap();
        assert_eq!(image_of(&lead).as_deref(), Some("omm"));
        assert_eq!(image_rows(), 1);
        cleanup(&path);
    }

    #[test]
    fn rescanning_keeps_a_resolved_image() {
        let (db, path) = named_db("image_keep");
        let mut cache = ScanCache::default();

        db.upsert_tracks(&[track("/music/a.flac", true)], &mut cache)
            .unwrap();
        let song = song_id_for(&db, "/music/a.flac");
        db.store_song_image(&song, "img1", Some(b"jpegbytes"))
            .unwrap();
        assert_eq!(
            image_state(&db, "/music/a.flac"),
            (Some("img1".to_string()), true)
        );

        db.upsert_tracks(&[track("/music/a.flac", false)], &mut cache)
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

        db.upsert_tracks(&[track("/music/b.flac", true)], &mut cache)
            .unwrap();
        let song = song_id_for(&db, "/music/b.flac");
        db.store_song_image(&song, "img1", Some(b"jpegbytes"))
            .unwrap();

        db.upsert_tracks(&[track("/music/b.flac", true)], &mut cache)
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

        db.upsert_tracks(&[track("/music/c.flac", true)], &mut cache)
            .unwrap();
        assert_eq!(image_state(&db, "/music/c.flac"), (None, false));

        let song = song_id_for(&db, "/music/c.flac");
        db.mark_song_image_missing(&song).unwrap();

        assert_eq!(image_state(&db, "/music/c.flac"), (None, true));
        assert!(
            db.get_songs_without_image(10).unwrap().is_empty(),
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
        db.upsert_tracks(&[first, second], &mut cache).unwrap();

        let d1 = song_id_for(&db, "/music/d1.flac");
        let d2 = song_id_for(&db, "/music/d2.flac");

        assert!(!db.has_image("shared").unwrap());
        db.store_song_image(&d1, "shared", Some(b"jpegbytes"))
            .unwrap();
        assert!(db.has_image("shared").unwrap());

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
            db.get_album_image_target(&album_id).unwrap(),
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
        db.upsert_tracks(&[only], &mut cache).unwrap();

        let album_id = db
            .get_song_by_path("/music/e.flac")
            .unwrap()
            .unwrap()
            .album_id
            .unwrap();
        assert!(matches!(
            db.get_album_image_target(&album_id).unwrap(),
            Some(AlbumImageTarget::Candidates(_))
        ));

        db.mark_song_image_missing(&song_id_for(&db, "/music/e.flac"))
            .unwrap();

        assert!(
            db.get_album_image_target(&album_id).unwrap().is_none(),
            "with every track checked the album must offer nothing"
        );
        cleanup(&path);
    }
}
