use super::{Database, collect_mapped};
use crate::data::models::{
    Cuid, Event, EventType, PinnedItem, RecentItem, RecentItemRow, Toggleable,
};
use anyhow::Result;
use rusqlite::params;

impl Database {
    pub fn get_recently_added(&self, limit: i64) -> Result<Vec<RecentItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            r#"
            WITH recent_songs AS (
                SELECT s.rowid AS rowid, s.id, s.title, s.album_id, s.image_id, s.date_added, s.date
                FROM songs s
                ORDER BY s.date_added DESC, s.rowid DESC
                LIMIT ?1
            ),
            image_groups AS (
                SELECT
                    COALESCE(rs.image_id, 'album_' || rs.album_id, 'no_image_' || rs.id) AS group_key,
                    rs.image_id,
                    rs.album_id,
                    MAX(rs.rowid) AS latest_rowid,
                    COUNT(*) AS song_count,
                    rs.id AS first_song_id
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
            ORDER BY ig.latest_rowid DESC
            "#,
        )?;
        let rows = stmt
            .query_map(params![limit], RecentItemRow::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().map(|r| r.into_recent_item()).collect())
    }

    pub fn get_recently_played(&self, limit: i64) -> Result<Vec<RecentItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            r#"
            WITH recent_song_plays AS (
                SELECT song_id, MAX(timestamp) AS most_recent_date
                FROM events
                WHERE event_type = ?1
                GROUP BY song_id
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

    pub fn get_pinned(&self) -> Vec<PinnedItem> {
        let run = || -> Result<Vec<PinnedItem>> {
            let conn = self.conn.lock();
            collect_mapped::<PinnedItem, _>(
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
                PinnedItem::from_row,
            )
        };
        run().unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn get_events_by_type(&self, event_type: EventType) -> Result<Vec<Event>> {
        let conn = self.conn.lock();
        collect_mapped::<Event, _>(
            &conn,
            "SELECT * FROM events WHERE event_type = ?1 ORDER BY timestamp DESC, rowid DESC",
            params![event_type.as_str()],
            Event::from_row,
        )
    }

    pub fn insert_event(
        &self,
        event_type: EventType,
        song_id: &Cuid,
        playlist_id: Option<&Cuid>,
        position: Option<i64>,
        unix_seconds: f64,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO events (id, event_type, song_id, playlist_id, position, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, STRFTIME('%Y-%m-%d %H:%M:%f', ?6, 'unixepoch'))",
            params![
                Cuid::new(),
                event_type.as_str(),
                song_id,
                playlist_id,
                position,
                unix_seconds
            ],
        )?;
        Ok(())
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
}

#[cfg(test)]
mod tests {

    use crate::data::db::test_util::*;
    use crate::data::db::{BatchTrack, ScanCache};
    use crate::data::models::RecentItem;

    #[test]
    fn recently_added_albums_come_first_by_default() {
        let (db, path) = named_db("album_recency");
        db.upsert_tracks(
            &[
                BatchTrack {
                    audio_hash: "1",
                    album: Some("Old"),
                    ..track("/music/old.flac", false)
                },
                BatchTrack {
                    audio_hash: "2",
                    album: Some("New"),
                    ..track("/music/new.flac", false)
                },
            ],
            &mut ScanCache::default(),
        )
        .unwrap();

        let albums = db.get_albums("", 0, 10).unwrap();
        let titles: Vec<&str> = albums.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["New", "Old"]);
        cleanup(&path);
    }

    #[test]
    fn recently_added_items_surfaces_the_newest_song_of_each_group() {
        let (db, path) = named_db("recent_items");
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
            ],
            &mut ScanCache::default(),
        )
        .unwrap();

        let items = db.get_recently_added(10).unwrap();
        assert_eq!(items.len(), 2, "each song has its own image group here");
        let RecentItem::Song { id: first, .. } = &items[0] else {
            panic!("expected a song item");
        };
        let RecentItem::Song { id: second, .. } = &items[1] else {
            panic!("expected a song item");
        };
        assert_eq!(*first, song_id_for(&db, "/music/2.flac"));
        assert_eq!(*second, song_id_for(&db, "/music/1.flac"));
        cleanup(&path);
    }
}
