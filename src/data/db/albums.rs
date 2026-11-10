use super::{Database, collect_mapped};
use crate::data::models::{Album, AlbumListItem, Cuid, Song};
use anyhow::Result;
use rusqlite::{OptionalExtension, ToSql, params};
use std::collections::HashMap;

impl Database {
    pub fn get_album(&self, id: &Cuid) -> Result<Option<Album>> {
        let conn = self.conn.lock();
        let row = conn
            .prepare_cached(
                "SELECT al.id, al.title, al.upc, al.image_id, al.favorite, al.pinned,
                        (SELECT GROUP_CONCAT(name, ',')
                         FROM (SELECT ar.name FROM albums_artists aa JOIN artists ar ON aa.artist_id = ar.id WHERE aa.album_id = al.id ORDER BY aa.position)) AS artists
                 FROM albums al WHERE al.id = ?1",
            )?
            .query_row(params![id], Album::from_row)
            .optional()?;
        Ok(row)
    }

    pub fn get_albums(&self, query: &str, offset: i64, limit: i64) -> Result<Vec<AlbumListItem>> {
        let query = query.trim();
        if query.is_empty() {
            let conn = self.conn.lock();
            return collect_mapped::<AlbumListItem, _>(
                &conn,
                "SELECT al.id, al.title,
                        (SELECT GROUP_CONCAT(name, ', ')
                         FROM (SELECT ar.name FROM albums_artists aa JOIN artists ar ON aa.artist_id = ar.id WHERE aa.album_id = al.id ORDER BY aa.position)) AS artist_name,
                        al.image_id,
                        (SELECT MIN(s.date) FROM songs s WHERE s.album_id = al.id) AS year
                 FROM (SELECT a.id, a.title, a.image_id
                       FROM albums a
                       ORDER BY (SELECT MAX(s.rowid) FROM songs s WHERE s.album_id = a.id) DESC
                       LIMIT ?1 OFFSET ?2) al",
                params![limit, offset],
                AlbumListItem::from_row,
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

    pub fn count_albums(&self, query: &str) -> Result<usize> {
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

    pub fn get_album_songs(&self, album_id: &Cuid) -> Result<Vec<Song>> {
        let conn = self.conn.lock();
        collect_mapped::<Song, _>(
            &conn,
            "SELECT s.*,
                    (SELECT GROUP_CONCAT(name, ',') FROM (SELECT ar.name FROM songs_artists sa JOIN artists ar ON sa.artist_id = ar.id WHERE sa.song_id = s.id ORDER BY sa.position)) AS artists,
                    (SELECT GROUP_CONCAT(g.name, ',') FROM songs_genres sg JOIN genres g ON sg.genre_id = g.id WHERE sg.song_id = s.id) AS genres
             FROM songs s
             WHERE s.album_id = ?1
             ORDER BY s.track_number ASC",
            params![album_id],
            Song::from_row,
        )
    }

    pub fn delete_album(&self, id: &Cuid) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM albums WHERE id = ?1", params![id])?;
        }
        self.rebuild_search_index();
        Ok(())
    }
}

pub(super) fn fetch_albums_by_ids(
    conn: &rusqlite::Connection,
    ids: &[Cuid],
) -> Result<Vec<AlbumListItem>> {
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
        .query_map(params.as_slice(), AlbumListItem::from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();
    Ok(ids.iter().filter_map(|id| map.get(id).cloned()).collect())
}

#[cfg(test)]
mod tests {

    use crate::data::db::test_util::*;
    use crate::data::db::{BatchTrack, ScanCache};
    use crate::data::models::{Album, Cuid};

    #[test]
    fn retitling_an_album_drops_the_old_row_and_keeps_its_state() {
        let (db, path) = named_db("album_retitle");
        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "1",
                album: Some("Song - Single"),
                ..track("/music/x.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let song = song_id_for(&db, "/music/x.flac");
        let old_album = db
            .get_song_by_path("/music/x.flac")
            .unwrap()
            .unwrap()
            .album_id
            .unwrap();
        db.store_song_image(&song, "cover", Some(b"jpegbytes"))
            .unwrap();
        db.set_favorite::<Album>(&old_album, true).unwrap();
        db.set_pinned::<Album>(&old_album, true).unwrap();
        assert_eq!(
            db.get_album(&old_album)
                .unwrap()
                .unwrap()
                .image_id
                .as_deref(),
            Some("cover")
        );

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "1",
                album: Some("Song"),
                ..track("/music/x.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        assert_eq!(count(&db, "SELECT COUNT(*) FROM albums"), 1);
        assert!(db.get_album(&old_album).unwrap().is_none());
        let new_album = db
            .get_song_by_path("/music/x.flac")
            .unwrap()
            .unwrap()
            .album_id
            .unwrap();
        assert_ne!(new_album, old_album);
        let album = db.get_album(&new_album).unwrap().unwrap();
        assert_eq!(album.title, "Song");
        assert_eq!(album.image_id.as_deref(), Some("cover"));
        assert!(album.favorite, "favorite carries over from the renamed row");
        assert!(album.pinned, "pinned carries over from the renamed row");
        cleanup(&path);
    }

    #[test]
    fn albums_with_the_same_title_split_by_album_artist() {
        let (db, path) = named_db("album_artist");
        db.upsert_tracks(
            &[
                BatchTrack {
                    album: Some("Greatest Hits"),
                    artists: &["Queen"],
                    ..track("/music/q.flac", false)
                },
                BatchTrack {
                    album: Some("Greatest Hits"),
                    artists: &["ABBA"],
                    ..track("/music/a.flac", false)
                },
                BatchTrack {
                    album: Some("greatest hits"),
                    album_artist: Some("Queen"),
                    artists: &["Queen", "David Bowie"],
                    ..track("/music/q2.flac", false)
                },
            ],
            &mut ScanCache::default(),
        )
        .unwrap();

        let album_of = |p: &str| db.get_song_by_path(p).unwrap().unwrap().album_id.unwrap();
        assert_ne!(album_of("/music/q.flac"), album_of("/music/a.flac"));
        assert_eq!(album_of("/music/q.flac"), album_of("/music/q2.flac"));
        assert_eq!(
            album_of("/music/q.flac"),
            Cuid::for_album("Greatest Hits", "Queen")
        );
        assert_eq!(count(&db, "SELECT COUNT(*) FROM albums"), 2);
        cleanup(&path);
    }
}
