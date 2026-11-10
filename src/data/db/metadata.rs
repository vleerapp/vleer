use super::scan::ARTIST_MERGE_SQL;
use super::{Database, placeholders};
use crate::data::models::Cuid;
use anyhow::Result;
use rusqlite::{OptionalExtension, params, params_from_iter};

pub struct SongBackfill<'a> {
    pub omm_id: &'a str,
    pub track_number: Option<i32>,
    pub year: Option<&'a str>,
    pub genres: &'a [String],
    pub image: Option<(&'a str, Option<&'a [u8]>)>,
    pub isrc: Option<&'a str>,
    pub upc: Option<&'a str>,
    pub album_image: Option<(&'a str, Option<&'a [u8]>)>,
    pub album_omm_id: Option<&'a str>,
}

pub struct AlbumLookup {
    pub id: Cuid,
    pub title: String,
    pub artist: String,
    pub artist_omm: Option<String>,
    pub upc: Option<String>,
}

pub struct AlbumMetadata<'a> {
    pub omm_id: &'a str,
    pub upc: Option<&'a str>,
    pub image: Option<(&'a str, Option<&'a [u8]>)>,
}

pub struct SongLookup {
    pub id: Cuid,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: i32,
    pub isrc: Option<String>,
    pub album_omm: Option<String>,
    pub artist_omm: Option<String>,
}

pub struct ArtistMetadata<'a> {
    pub omm_id: &'a str,
    pub name: &'a str,
    pub image: Option<(&'a str, Option<&'a [u8]>)>,
}

pub(super) const SONG_LOOKUP_SELECT: &str = "SELECT s.id, s.title, s.duration,
                        (SELECT title FROM albums WHERE id = s.album_id),
                        COALESCE((
                            SELECT a.name FROM songs_artists sa
                            JOIN artists a ON a.id = sa.artist_id
                            WHERE sa.song_id = s.id ORDER BY sa.position LIMIT 1
                        ), ''),
                        s.isrc,
                        (SELECT omm_id FROM albums WHERE id = s.album_id),
                        (SELECT ar.omm_id FROM songs_artists sa
                            JOIN artists ar ON ar.id = sa.artist_id
                            WHERE sa.song_id = s.id ORDER BY sa.position LIMIT 1)
                 FROM songs s";

pub(super) const ALBUM_LOOKUP_SELECT: &str = "SELECT al.id, al.title, al.upc,
                        COALESCE((
                            SELECT a.name FROM albums_artists aa
                            JOIN artists a ON a.id = aa.artist_id
                            WHERE aa.album_id = al.id ORDER BY aa.position LIMIT 1
                        ), ''),
                        (SELECT ar.omm_id FROM albums_artists aa
                            JOIN artists ar ON ar.id = aa.artist_id
                            WHERE aa.album_id = al.id ORDER BY aa.position LIMIT 1)
                 FROM albums al";

impl Database {
    pub fn get_pending_songs(&self, limit: i64, checked_only: bool) -> Result<Vec<SongLookup>> {
        let conn = self.image_write_conn.lock();
        let rows = conn
            .prepare_cached(&format!(
                "{SONG_LOOKUP_SELECT} WHERE s.omm_id IS NULL AND (?2 = 0 OR s.image_checked = 1) LIMIT ?1"
            ))?
            .query_map(params![limit, checked_only as i64], song_lookup_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_pending_songs_in(&self, ids: &[Cuid]) -> Result<Vec<SongLookup>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.image_write_conn.lock();
        let sql = format!(
            "{SONG_LOOKUP_SELECT} WHERE s.omm_id IS NULL AND s.id IN ({})",
            placeholders(ids.len())
        );
        let mut rows: Vec<SongLookup> = conn
            .prepare(&sql)?
            .query_map(params_from_iter(ids), song_lookup_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        rows.sort_by_key(|row| ids.iter().position(|id| *id == row.id));
        Ok(rows)
    }

    pub fn get_pending_albums(&self, limit: i64) -> Result<Vec<AlbumLookup>> {
        let conn = self.image_write_conn.lock();
        let rows = conn
            .prepare_cached(&format!(
                "{ALBUM_LOOKUP_SELECT} WHERE al.omm_id IS NULL LIMIT ?1"
            ))?
            .query_map(params![limit], album_lookup_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_pending_albums_in(&self, ids: &[Cuid]) -> Result<Vec<AlbumLookup>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.image_write_conn.lock();
        let sql = format!(
            "{ALBUM_LOOKUP_SELECT} WHERE al.omm_id IS NULL AND al.id IN ({})",
            placeholders(ids.len())
        );
        let mut rows: Vec<AlbumLookup> = conn
            .prepare(&sql)?
            .query_map(params_from_iter(ids), album_lookup_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        rows.sort_by_key(|row| ids.iter().position(|id| *id == row.id));
        Ok(rows)
    }

    pub fn get_pending_artists(&self, limit: i64) -> Result<Vec<(Cuid, String)>> {
        let conn = self.image_write_conn.lock();
        let rows = conn
            .prepare_cached("SELECT id, name FROM artists WHERE omm_id IS NULL LIMIT ?1")?
            .query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_pending_song_albums(&self, song_ids: &[Cuid]) -> Result<Vec<AlbumLookup>> {
        if song_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.image_write_conn.lock();
        let sql = format!(
            "{ALBUM_LOOKUP_SELECT} WHERE al.omm_id IS NULL AND al.id IN (
                SELECT album_id FROM songs WHERE id IN ({}) AND album_id IS NOT NULL)",
            placeholders(song_ids.len())
        );
        let rows = conn
            .prepare(&sql)?
            .query_map(params_from_iter(song_ids), album_lookup_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_pending_song_artists(&self, song_ids: &[Cuid]) -> Result<Vec<(Cuid, String)>> {
        if song_ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.image_write_conn.lock();
        let sql = format!(
            "SELECT DISTINCT ar.id, ar.name FROM songs_artists sa
             JOIN artists ar ON ar.id = sa.artist_id
             WHERE sa.position = 0 AND ar.omm_id IS NULL AND sa.song_id IN ({})",
            placeholders(song_ids.len())
        );
        let rows = conn
            .prepare(&sql)?
            .query_map(params_from_iter(song_ids), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn has_stale_omm_ids(&self) -> Result<bool> {
        let conn = self.image_write_conn.lock();
        let stale = |table: &str| {
            format!(
                "SELECT 1 FROM {table}
                 WHERE NULLIF(omm_id, '') IS NOT NULL
                   AND NOT (omm_id GLOB '[1-9]*'
                            AND omm_id NOT GLOB '*[^0-9]*'
                            AND length(omm_id) <= 18)"
            )
        };
        let sql = format!(
            "SELECT EXISTS({}) OR EXISTS({}) OR EXISTS({})",
            stale("songs"),
            stale("albums"),
            stale("artists")
        );
        Ok(conn.query_row(&sql, [], |row| row.get(0))?)
    }

    pub fn store_song_metadata(&self, song_id: &Cuid, metadata: &SongBackfill<'_>) -> Result<bool> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;

        if let Some((image_id, Some(data))) = metadata.image {
            tx.prepare_cached(
                "INSERT INTO images (id, data) VALUES (?1, ?2)
                 ON CONFLICT(id) DO NOTHING",
            )?
            .execute(params![image_id, data])?;
        }

        let updated = tx
            .prepare_cached(
                "UPDATE songs SET
                    omm_id = ?2,
                    track_number = COALESCE(track_number, ?3),
                    date = CASE WHEN date IS NULL OR date = '' THEN COALESCE(?4, date) ELSE date END,
                    isrc = COALESCE(isrc, ?5)
                 WHERE id = ?1",
            )?
            .execute(params![
                song_id,
                metadata.omm_id,
                metadata.track_number,
                metadata.year,
                metadata.isrc
            ])?;

        if let Some(upc) = metadata.upc {
            tx.prepare_cached(
                "UPDATE albums SET upc = ?2
                 WHERE id = (SELECT album_id FROM songs WHERE id = ?1) AND upc IS NULL",
            )?
            .execute(params![song_id, upc])?;
        }

        if let Some(album_omm_id) = metadata.album_omm_id {
            tx.prepare_cached(
                "UPDATE albums SET omm_id = ?2
                 WHERE id = (SELECT album_id FROM songs WHERE id = ?1)
                   AND NULLIF(omm_id, '') IS NULL",
            )?
            .execute(params![song_id, album_omm_id])?;
        }

        if let Some((image_id, data)) = metadata.album_image {
            if let Some(data) = data {
                tx.prepare_cached(
                    "INSERT INTO images (id, data) VALUES (?1, ?2)
                     ON CONFLICT(id) DO NOTHING",
                )?
                .execute(params![image_id, data])?;
            }
            tx.prepare_cached(
                "UPDATE albums SET image_id = ?2
                 WHERE id = (SELECT album_id FROM songs WHERE id = ?1) AND image_id IS NULL
                   AND EXISTS (SELECT 1 FROM images WHERE id = ?2)",
            )?
            .execute(params![song_id, image_id])?;
        }

        if let Some((image_id, _)) = metadata.image {
            tx.prepare_cached(
                "UPDATE songs SET image_id = ?2, image_checked = 1
                 WHERE id = ?1 AND image_id IS NULL",
            )?
            .execute(params![song_id, image_id])?;
        }

        let has_genres: bool = tx
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM songs_genres WHERE song_id = ?1)")?
            .query_row(params![song_id], |row| row.get(0))?;
        if updated > 0 && !has_genres {
            for name in metadata
                .genres
                .iter()
                .map(|g| g.trim())
                .filter(|g| !g.is_empty())
            {
                let genre_id = Cuid::for_genre(name);
                tx.prepare_cached(
                    "INSERT INTO genres (id, name) VALUES (?1, ?2) ON CONFLICT(id) DO NOTHING",
                )?
                .execute(params![genre_id, name])?;
                tx.prepare_cached(
                    "INSERT INTO songs_genres (song_id, genre_id) VALUES (?1, ?2)
                     ON CONFLICT(song_id, genre_id) DO NOTHING",
                )?
                .execute(params![song_id, genre_id])?;
            }
        }

        tx.commit()?;
        Ok(updated > 0)
    }

    pub fn store_album_metadata(
        &self,
        album_id: &Cuid,
        metadata: &AlbumMetadata<'_>,
    ) -> Result<bool> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;

        if let Some((image_id, Some(data))) = metadata.image {
            tx.prepare_cached(
                "INSERT INTO images (id, data) VALUES (?1, ?2)
                 ON CONFLICT(id) DO NOTHING",
            )?
            .execute(params![image_id, data])?;
        }

        let updated = tx
            .prepare_cached("UPDATE albums SET omm_id = ?2, upc = COALESCE(upc, ?3) WHERE id = ?1")?
            .execute(params![album_id, metadata.omm_id, metadata.upc])?;

        if let Some((image_id, _)) = metadata.image {
            tx.prepare_cached(
                "UPDATE albums SET image_id = ?2
                 WHERE id = ?1 AND image_id IS NULL
                   AND EXISTS (SELECT 1 FROM images WHERE id = ?2)",
            )?
            .execute(params![album_id, image_id])?;
        }

        tx.commit()?;
        Ok(updated > 0)
    }

    pub fn store_artist_metadata(
        &self,
        artist_id: &Cuid,
        metadata: &ArtistMetadata<'_>,
    ) -> Result<Option<Cuid>> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;

        let Some(current_name): Option<String> = tx
            .prepare_cached("SELECT name FROM artists WHERE id = ?1")?
            .query_row(params![artist_id], |row| row.get(0))
            .optional()?
        else {
            return Ok(None);
        };

        let keeper: Option<(Cuid, String)> = tx
            .prepare_cached(
                "SELECT id, name FROM artists
                 WHERE id <> ?1 AND (omm_id = ?2 OR name = ?3 COLLATE NOCASE)
                 ORDER BY omm_id = ?2 DESC LIMIT 1",
            )?
            .query_row(params![artist_id, metadata.omm_id, metadata.name], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;

        if let Some((image_id, Some(data))) = metadata.image {
            tx.prepare_cached(
                "INSERT INTO images (id, data) VALUES (?1, ?2)
                 ON CONFLICT(id) DO NOTHING",
            )?
            .execute(params![image_id, data])?;
        }
        let image_owner = keeper.as_ref().map_or(artist_id, |(id, _)| id);
        if let Some((image_id, _)) = metadata.image {
            tx.prepare_cached("UPDATE artists SET image_id = ?2 WHERE id = ?1")?
                .execute(params![image_owner, image_id])?;
        }

        let target = match keeper {
            Some((keeper_id, keeper_name)) => {
                for sql in ARTIST_MERGE_SQL {
                    tx.prepare_cached(sql)?
                        .execute(params![keeper_id, artist_id])?;
                }
                insert_artist_alias(&tx, &keeper_name, &keeper_id, metadata.name)?;
                keeper_id
            }
            None => artist_id.clone(),
        };
        insert_artist_alias(&tx, &current_name, &target, metadata.name)?;

        tx.prepare_cached(
            "UPDATE artists SET
                omm_id = ?2,
                name = ?3,
                image_id = COALESCE(?4, image_id)
             WHERE id = ?1",
        )?
        .execute(params![
            target,
            metadata.omm_id,
            metadata.name,
            metadata.image.map(|(image_id, _)| image_id)
        ])?;

        tx.commit()?;
        Ok(Some(target))
    }

    pub fn mark_song_unmatched(&self, song_id: &Cuid) -> Result<()> {
        let conn = self.image_write_conn.lock();
        conn.prepare_cached("UPDATE songs SET omm_id = '' WHERE id = ?1 AND omm_id IS NULL")?
            .execute(params![song_id])?;
        Ok(())
    }

    pub fn mark_album_unmatched(&self, album_id: &Cuid) -> Result<()> {
        let conn = self.image_write_conn.lock();
        conn.prepare_cached("UPDATE albums SET omm_id = '' WHERE id = ?1 AND omm_id IS NULL")?
            .execute(params![album_id])?;
        Ok(())
    }

    pub fn mark_artist_unmatched(&self, artist_id: &Cuid) -> Result<()> {
        let conn = self.image_write_conn.lock();
        conn.prepare_cached("UPDATE artists SET omm_id = '' WHERE id = ?1 AND omm_id IS NULL")?
            .execute(params![artist_id])?;
        Ok(())
    }

    pub fn reset_omm_ids(&self) -> Result<()> {
        let mut conn = self.image_write_conn.lock();
        let tx = conn.transaction()?;
        for table in ["songs", "albums", "artists"] {
            tx.execute(
                &format!("UPDATE {table} SET omm_id = NULL WHERE omm_id IS NOT NULL"),
                [],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn song_lookup_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SongLookup> {
    Ok(SongLookup {
        id: row.get(0)?,
        title: row.get(1)?,
        duration: row.get(2)?,
        album: row.get(3)?,
        artist: row.get(4)?,
        isrc: row.get(5)?,
        album_omm: row.get(6)?,
        artist_omm: row.get(7)?,
    })
}

pub(super) fn album_lookup_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AlbumLookup> {
    Ok(AlbumLookup {
        id: row.get(0)?,
        title: row.get(1)?,
        upc: row.get(2)?,
        artist: row.get(3)?,
        artist_omm: row.get(4)?,
    })
}

pub(super) fn insert_artist_alias(
    tx: &rusqlite::Transaction<'_>,
    alias: &str,
    artist_id: &Cuid,
    canonical: &str,
) -> Result<()> {
    if alias.eq_ignore_ascii_case(canonical) {
        return Ok(());
    }
    tx.prepare_cached(
        "INSERT INTO artist_aliases (name, artist_id) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET artist_id = excluded.artist_id",
    )?
    .execute(params![alias, artist_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {

    use crate::data::db::test_util::*;
    use crate::data::db::{ArtistMetadata, BatchTrack, ScanCache, SongBackfill};

    use rusqlite::params;

    #[test]
    fn song_metadata_backfills_isrc_and_album_upc_and_image() {
        let (db, path) = named_db("album_backfill");
        db.upsert_tracks(
            &[BatchTrack {
                album: Some("Album"),
                ..track("/a.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let song = db.get_song_by_path("/a.flac").unwrap().unwrap();
        let album_id = song.album_id.clone().unwrap();
        assert!(db.song_album_needs_image(&song.id).unwrap());

        db.store_song_metadata(
            &song.id,
            &SongBackfill {
                omm_id: "1234567890",
                track_number: None,
                year: None,
                genres: &[],
                image: None,
                isrc: Some("USRC17607839"),
                upc: Some("0602577812345"),
                album_image: Some(("cover", Some(&[1, 2, 3]))),
                album_omm_id: Some("2345678901"),
            },
        )
        .unwrap();

        let album = db.get_album(&album_id).unwrap().unwrap();
        assert_eq!(album.upc.as_deref(), Some("0602577812345"));
        assert_eq!(album.image_id.as_deref(), Some("cover"));
        assert!(db.get_pending_albums(10).unwrap().is_empty());
        assert!(!db.song_album_needs_image(&song.id).unwrap());
        let song = db.get_song(&song.id).unwrap().unwrap();
        assert_eq!(song.isrc.as_deref(), Some("USRC17607839"));
        cleanup(&path);
    }

    #[test]
    fn tagged_omm_ids_are_stored_on_scan() {
        let (db, path) = named_db("tagged_omm_ids");
        let mut cache = ScanCache::default();
        let artists = ["Ellis", "Other"];
        let tagged = BatchTrack {
            artists: &artists,
            album: Some("Night"),
            omm_song_id: Some("1024303286"),
            omm_album_id: Some("1028859113"),
            omm_artist_id: Some("1005090652"),
            ..track("/music/1.flac", true)
        };
        db.upsert_tracks(&[tagged], &mut cache).unwrap();

        let ids = |sql: &str| -> Vec<Option<String>> {
            let conn = db.conn.lock();
            let mut stmt = conn.prepare(sql).unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(ids("SELECT omm_id FROM songs"), [Some("1024303286".into())]);
        assert_eq!(
            ids("SELECT omm_id FROM albums"),
            [Some("1028859113".into())]
        );
        assert_eq!(
            ids("SELECT omm_id FROM artists ORDER BY name"),
            [Some("1005090652".into()), None]
        );

        let untagged = BatchTrack {
            artists: &artists,
            album: Some("Night"),
            ..track("/music/1.flac", true)
        };
        db.upsert_tracks(&[untagged], &mut cache).unwrap();
        assert_eq!(ids("SELECT omm_id FROM songs"), [None]);
        cleanup(&path);
    }

    #[test]
    fn stale_omm_ids_are_detected_and_reset() {
        let (db, path) = named_db("stale_omm_ids");
        db.upsert_tracks(&[track("/music/1.flac", false)], &mut ScanCache::default())
            .unwrap();
        let check = |value: Option<&str>| {
            db.image_write_conn
                .lock()
                .execute("UPDATE songs SET omm_id = ?1", params![value])
                .unwrap();
            db.has_stale_omm_ids().unwrap()
        };
        assert!(!check(None));
        assert!(!check(Some("")));
        assert!(!check(Some("1234567890")));
        assert!(!check(Some("123456789012345678")));
        assert!(check(Some("1234567890123456789")));
        assert!(check(Some("0123456789")));
        assert!(check(Some("0a1b2c3d4e5f6g7h")));
        assert!(check(Some("omm:song:1234567890")));

        db.reset_omm_ids().unwrap();
        assert!(!db.has_stale_omm_ids().unwrap());
        let nulls: i64 = db
            .image_write_conn
            .lock()
            .query_row("SELECT COUNT(*) FROM songs WHERE omm_id IS NULL", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(nulls, 1);
        cleanup(&path);
    }

    #[test]
    fn identified_artists_take_the_canonical_name_and_keep_matching_tags() {
        let (db, path) = named_db("artist_metadata");
        let mut cache = ScanCache::default();
        db.upsert_tracks(
            &[
                BatchTrack {
                    artists: &["Beyonce"],
                    ..track("/music/1.flac", false)
                },
                BatchTrack {
                    artists: &["Beyoncé"],
                    ..track("/music/2.flac", false)
                },
            ],
            &mut cache,
        )
        .unwrap();

        let by_name = |name: &str| db.get_artist_by_name(name).unwrap().unwrap().id;
        let plain = by_name("Beyonce");
        let accented = by_name("Beyoncé");
        let metadata = ArtistMetadata {
            omm_id: "3456789012",
            name: "Beyoncé",
            image: Some(("img", Some(b"jpeg"))),
        };

        assert_eq!(
            db.store_artist_metadata(&plain, &metadata).unwrap(),
            Some(accented.clone())
        );
        assert_eq!(artist_names(&db), vec!["Beyoncé".to_string()]);
        assert!(db.get_pending_artists(10).unwrap().is_empty());

        let mut cache = ScanCache::default();
        db.upsert_tracks(
            &[
                BatchTrack {
                    artists: &["Beyonce"],
                    ..track("/music/1.flac", false)
                },
                BatchTrack {
                    artists: &["BEYONCE"],
                    ..track("/music/3.flac", false)
                },
            ],
            &mut cache,
        )
        .unwrap();

        assert_eq!(artist_names(&db), vec!["Beyoncé".to_string()]);
        let artist = db.get_artist(&accented).unwrap().unwrap();
        assert_eq!(artist.image_id.as_deref(), Some("img"));
        cleanup(&path);
    }
}
