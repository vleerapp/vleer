use super::Database;
use crate::data::{ids::fold_key, models::Cuid};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct BatchTrack<'a> {
    pub title: &'a str,
    pub artists: &'a [&'a str],
    pub genres: &'a [&'a str],
    pub album: Option<&'a str>,
    pub album_artist: Option<&'a str>,
    pub file_path: &'a str,
    pub audio_hash: &'a str,
    pub duration: i32,
    pub track_number: Option<i32>,
    pub year: Option<i32>,

    pub recheck_image: bool,
    pub file_size: i64,
    pub file_modified: i64,
    pub lufs: Option<f32>,
    pub isrc: Option<&'a str>,
    pub upc: Option<&'a str>,
    pub omm_song_id: Option<&'a str>,
    pub omm_album_id: Option<&'a str>,
    pub omm_artist_id: Option<&'a str>,
}

pub(super) struct AlbumCacheEntry {
    id: Cuid,
    artists: HashSet<String>,
    next_position: i64,
}

pub(super) type SpellingVotes = HashMap<Cuid, HashMap<String, usize>>;

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
    albums: HashMap<(String, String), AlbumCacheEntry>,
    artist_votes: SpellingVotes,
    genre_votes: SpellingVotes,
    album_votes: SpellingVotes,
}

pub(super) const ARTIST_MERGE_SQL: &[&str] = &[
    "INSERT OR IGNORE INTO songs_artists (song_id, artist_id, position)
     SELECT song_id, ?1, position FROM songs_artists WHERE artist_id = ?2",
    "INSERT OR IGNORE INTO albums_artists (album_id, artist_id, position)
     SELECT album_id, ?1, position FROM albums_artists WHERE artist_id = ?2",
    "UPDATE artists SET
        favorite = (SELECT MAX(COALESCE(favorite, 0)) FROM artists WHERE id IN (?1, ?2)),
        pinned = (SELECT MAX(COALESCE(pinned, 0)) FROM artists WHERE id IN (?1, ?2)),
        image_id = COALESCE(image_id, (SELECT image_id FROM artists WHERE id = ?2)),
        omm_id = CASE
            WHEN NULLIF(omm_id, '') IS NOT NULL THEN omm_id
            WHEN NULLIF((SELECT omm_id FROM artists WHERE id = ?2), '') IS NOT NULL
                THEN (SELECT omm_id FROM artists WHERE id = ?2)
            WHEN omm_id IS NOT NULL OR (SELECT omm_id FROM artists WHERE id = ?2) IS NOT NULL
                THEN ''
            ELSE NULL
        END
     WHERE id = ?1",
    "UPDATE artist_aliases SET artist_id = ?1 WHERE artist_id = ?2",
    "DELETE FROM songs_artists WHERE artist_id = ?2",
    "DELETE FROM albums_artists WHERE artist_id = ?2",
    "DELETE FROM artists WHERE id = ?2",
];

impl Database {
    pub fn upsert_tracks(&self, tracks: &[BatchTrack<'_>], cache: &mut ScanCache) -> Result<usize> {
        if tracks.is_empty() {
            return Ok(0);
        }

        let lock_started = std::time::Instant::now();
        let mut conn = self.conn.lock();
        write_profile::add(&write_profile::LOCK_NS, lock_started);
        let tx = conn.transaction()?;
        let mut inserted = 0usize;
        let mut touched_artists = HashSet::new();
        let mut touched_genres = HashSet::new();
        let mut touched_albums = HashSet::new();

        for track in tracks {
            let album_started = std::time::Instant::now();
            let album_id = match track.album {
                Some(album_title) => Some(upsert_album_cached(
                    &tx,
                    cache,
                    album_title,
                    track.album_artist,
                    track.artists,
                )?),
                None => None,
            };
            if let (Some(id), Some(title)) = (&album_id, track.album) {
                vote(&mut cache.album_votes, id, title);
                touched_albums.insert(id.clone());
            }
            if let (Some(id), Some(upc)) = (&album_id, track.upc) {
                tx.prepare_cached("UPDATE albums SET upc = ?2 WHERE id = ?1 AND upc IS NOT ?2")?
                    .execute(params![id, upc])?;
            }
            if let (Some(id), Some(omm_id)) = (&album_id, track.omm_album_id) {
                tx.prepare_cached(
                    "UPDATE albums SET omm_id = ?2 WHERE id = ?1 AND omm_id IS NOT ?2",
                )?
                .execute(params![id, omm_id])?;
            }
            write_profile::add(&write_profile::ALBUM_NS, album_started);
            let song_started = std::time::Instant::now();

            let year_str = track.year.map(|y| y.to_string());

            let song_id = Cuid::for_song(track.audio_hash, album_id.as_ref());
            let at_path: Option<Cuid> = tx
                .prepare_cached("SELECT id FROM songs WHERE file_path = ?1")?
                .query_row(params![track.file_path], |row| row.get(0))
                .optional()?;
            match at_path {
                Some(old_id) if old_id != song_id => {
                    let canonical_path: Option<String> = tx
                        .prepare_cached("SELECT file_path FROM songs WHERE id = ?1")?
                        .query_row(params![song_id], |row| row.get(0))
                        .optional()?;
                    match canonical_path {
                        Some(canonical_path) if Path::new(&canonical_path).exists() => {
                            continue;
                        }
                        Some(_) => {
                            tx.prepare_cached(
                                "UPDATE songs SET
                                    favorite = MAX(favorite, (SELECT favorite FROM songs WHERE id = ?2)),
                                    pinned = MAX(pinned, (SELECT pinned FROM songs WHERE id = ?2))
                                 WHERE id = ?1",
                            )?
                            .execute(params![song_id, old_id])?;
                            tx.prepare_cached("UPDATE events SET song_id = ?1 WHERE song_id = ?2")?
                                .execute(params![song_id, old_id])?;
                            tx.prepare_cached("DELETE FROM songs WHERE id = ?1")?
                                .execute(params![old_id])?;
                            tx.prepare_cached("UPDATE songs SET file_path = ?1 WHERE id = ?2")?
                                .execute(params![track.file_path, song_id])?;
                        }
                        None => {
                            tx.prepare_cached("UPDATE songs SET id = ?1 WHERE id = ?2")?
                                .execute(params![song_id, old_id])?;
                            tx.prepare_cached("UPDATE events SET song_id = ?1 WHERE song_id = ?2")?
                                .execute(params![song_id, old_id])?;
                        }
                    }
                }
                Some(_) => {}
                None => {
                    let existing_path: Option<String> = tx
                        .prepare_cached("SELECT file_path FROM songs WHERE id = ?1")?
                        .query_row(params![song_id], |row| row.get(0))
                        .optional()?;
                    match existing_path {
                        Some(existing_path) if Path::new(&existing_path).exists() => {
                            continue;
                        }
                        Some(_) => {
                            tx.prepare_cached("UPDATE songs SET file_path = ?1 WHERE id = ?2")?
                                .execute(params![track.file_path, song_id])?;
                        }
                        None => {
                            let orphans = tx
                                .prepare_cached(
                                    "SELECT id, file_path FROM songs
                                     WHERE audio_hash = ?1 ORDER BY date_added",
                                )?
                                .query_map(params![track.audio_hash], |row| {
                                    Ok((row.get::<_, Cuid>(0)?, row.get::<_, String>(1)?))
                                })?
                                .collect::<rusqlite::Result<Vec<_>>>()?;
                            let orphan = orphans
                                .into_iter()
                                .find(|(_, path)| !Path::new(path).exists());
                            match orphan {
                                Some((orphan_id, _)) => {
                                    tx.prepare_cached(
                                        "UPDATE songs SET id = ?1, file_path = ?2 WHERE id = ?3",
                                    )?
                                    .execute(params![
                                        song_id,
                                        track.file_path,
                                        orphan_id
                                    ])?;
                                    tx.prepare_cached(
                                        "UPDATE events SET song_id = ?1 WHERE song_id = ?2",
                                    )?
                                    .execute(params![song_id, orphan_id])?;
                                }
                                None => inserted += 1,
                            }
                        }
                    }
                }
            }

            tx.prepare_cached(
                    "INSERT INTO songs (id, title, album_id, file_path, file_size, file_modified, date, duration, track_number, lufs, image_checked, audio_hash, isrc, omm_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?12, ?13, ?14)
                     ON CONFLICT(id) DO UPDATE SET
                        audio_hash = excluded.audio_hash,
                        omm_id = excluded.omm_id,
                        isrc = COALESCE(excluded.isrc, songs.isrc),
                        title = excluded.title,
                        album_id = excluded.album_id,
                        file_size = excluded.file_size,
                        file_modified = excluded.file_modified,
                        date = excluded.date,
                        duration = excluded.duration,
                        track_number = excluded.track_number,
                        lufs = excluded.lufs,
                        image_checked = CASE WHEN ?11 THEN 0 ELSE songs.image_checked END",
                )?
                .execute(
                    params![
                        song_id,
                        track.title,
                        album_id,
                        track.file_path,
                        track.file_size,
                        track.file_modified,
                        year_str,
                        track.duration,
                        track.track_number,
                        track.lufs,
                        track.recheck_image,
                        track.audio_hash,
                        track.isrc,
                        track.omm_song_id
                    ],
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
                if position == 0
                    && let Some(omm_id) = track.omm_artist_id
                {
                    tx.prepare_cached(
                        "UPDATE artists SET omm_id = ?2 WHERE id = ?1 AND omm_id IS NOT ?2",
                    )?
                    .execute(params![artist_id, omm_id])?;
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
        }

        apply_majority_spellings(
            &tx,
            &cache.artist_votes,
            &touched_artists,
            "SELECT name, (SELECT COUNT(*) FROM songs_artists WHERE artist_id = ?1)
             FROM artists WHERE id = ?1",
            "UPDATE OR IGNORE artists SET name = ?1 WHERE id = ?2 AND COALESCE(omm_id, '') = ''",
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
        Ok(inserted)
    }

    pub fn relink_unhashed_songs(&self) -> Result<usize> {
        const ARTIST_SET: &str = "(SELECT group_concat(artist_id) FROM (
            SELECT artist_id FROM songs_artists WHERE song_id = {} ORDER BY artist_id))";
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let stale: Vec<(Cuid, String, i32, String)> = tx
            .prepare_cached(
                "SELECT id, title, duration, file_path FROM songs WHERE audio_hash IS NULL",
            )?
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;

        let find = format!(
            "SELECT s.id, s.file_path FROM songs s
             WHERE s.audio_hash IS NOT NULL AND s.title = ?2 COLLATE NOCASE AND s.duration = ?3
               AND {} IS {}",
            ARTIST_SET.replace("{}", "s.id"),
            ARTIST_SET.replace("{}", "?1"),
        );

        let mut merged = 0;
        for (stale_id, title, duration, path) in stale {
            if Path::new(&path).exists() {
                continue;
            }
            let live: Vec<(Cuid, String)> = tx
                .prepare_cached(&find)?
                .query_map(params![stale_id, title, duration], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let mut live = live
                .into_iter()
                .filter(|(_, path)| Path::new(path).exists());
            let (Some((live_id, _)), None) = (live.next(), live.next()) else {
                continue;
            };

            tx.prepare_cached(
                "UPDATE songs SET
                    favorite = MAX(favorite, (SELECT favorite FROM songs WHERE id = ?2)),
                    pinned = MAX(pinned, (SELECT pinned FROM songs WHERE id = ?2)),
                    date_added = MIN(date_added, (SELECT date_added FROM songs WHERE id = ?2))
                 WHERE id = ?1",
            )?
            .execute(params![live_id, stale_id])?;
            for table in ["playlist_songs", "events", "lyrics"] {
                tx.prepare_cached(&format!(
                    "UPDATE OR IGNORE {table} SET song_id = ?1 WHERE song_id = ?2"
                ))?
                .execute(params![live_id, stale_id])?;
            }
            tx.prepare_cached("DELETE FROM songs WHERE id = ?1")?
                .execute(params![stale_id])?;
            merged += 1;
        }

        tx.commit()?;
        drop(conn);
        if merged > 0 {
            self.rebuild_search_index();
        }
        Ok(merged)
    }
}

pub(super) fn artist_id_cached(
    tx: &rusqlite::Transaction<'_>,
    cache: &mut ScanCache,
    name: &str,
) -> Result<Cuid> {
    let key = fold_key(name);
    if let Some(id) = cache.artists.get(&key) {
        return Ok(id.clone());
    }
    let existing: Option<Cuid> = tx
        .prepare_cached(
            "SELECT id FROM (
                 SELECT id, 0 AS rank FROM artists WHERE name = ?1 COLLATE NOCASE
                 UNION ALL
                 SELECT artist_id, 1 FROM artist_aliases WHERE name = ?1
             ) ORDER BY rank LIMIT 1",
        )?
        .query_row(params![name], |row| row.get(0))
        .optional()?;
    let id = match existing {
        Some(id) => id,
        None => {
            let insert = |id: Cuid| -> Result<Option<Cuid>> {
                Ok(tx
                    .prepare_cached(
                        "INSERT INTO artists (id, name) VALUES (?1, ?2)
                         ON CONFLICT(name) DO UPDATE SET name = excluded.name
                         ON CONFLICT(id) DO NOTHING
                         RETURNING id",
                    )?
                    .query_row(params![id, name], |row| row.get(0))
                    .optional()?)
            };
            match insert(Cuid::for_artist(name))? {
                Some(id) => id,
                None => insert(Cuid::new())?.expect("a random artist id cannot collide"),
            }
        }
    };
    cache.artists.insert(key, id.clone());
    Ok(id)
}

pub(super) fn genre_id_cached(
    tx: &rusqlite::Transaction<'_>,
    cache: &mut ScanCache,
    name: &str,
) -> Result<Cuid> {
    let key = fold_key(name);
    if let Some(id) = cache.genres.get(&key) {
        return Ok(id.clone());
    }
    let id = Cuid::for_genre(name);
    tx.prepare_cached("INSERT INTO genres (id, name) VALUES (?1, ?2) ON CONFLICT(id) DO NOTHING")?
        .execute(params![id, name])?;
    cache.genres.insert(key, id.clone());
    Ok(id)
}

pub(super) fn upsert_album_cached(
    tx: &rusqlite::Transaction<'_>,
    cache: &mut ScanCache,
    title: &str,
    album_artist: Option<&str>,
    artists: &[&str],
) -> Result<Cuid> {
    let credited: Vec<&str> = album_artist
        .into_iter()
        .chain(artists.iter().copied())
        .collect();
    let key_artist = credited.first().copied().unwrap_or_default();
    let cache_key = (fold_key(title), fold_key(key_artist));
    if !cache.albums.contains_key(&cache_key) {
        let album_id = Cuid::for_album(title, key_artist);
        tx.prepare_cached(
            "INSERT INTO albums (id, title) VALUES (?1, ?2) ON CONFLICT(id) DO NOTHING",
        )?
        .execute(params![album_id, title])?;

        let existing: HashSet<String> = {
            let mut stmt = tx.prepare_cached(
                "SELECT a.name FROM artists a
                 JOIN albums_artists aa ON a.id = aa.artist_id
                 WHERE aa.album_id = ?1",
            )?;
            stmt.query_map(params![album_id], |row| row.get::<_, String>(0))?
                .map(|name| name.map(|name| fold_key(&name)))
                .collect::<rusqlite::Result<_>>()?
        };

        let max_position: i64 = tx
            .prepare_cached(
                "SELECT COALESCE(MAX(position), -1) FROM albums_artists WHERE album_id = ?1",
            )?
            .query_row(params![album_id], |row| row.get(0))
            .unwrap_or(-1);

        cache.albums.insert(
            cache_key.clone(),
            AlbumCacheEntry {
                id: album_id,
                artists: existing,
                next_position: max_position + 1,
            },
        );
    }

    let (album_id, missing): (Cuid, Vec<&str>) = {
        let entry = &cache.albums[&cache_key];
        let mut seen = HashSet::new();
        (
            entry.id.clone(),
            credited
                .iter()
                .copied()
                .filter(|name| {
                    let key = fold_key(name);
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
                .get_mut(&cache_key)
                .expect("album cache entry inserted above");
            let position = entry.next_position;
            entry.next_position += 1;
            entry.artists.insert(fold_key(name));
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

pub(super) fn vote(votes: &mut SpellingVotes, id: &Cuid, spelling: &str) {
    *votes
        .entry(id.clone())
        .or_default()
        .entry(spelling.to_string())
        .or_default() += 1;
}

pub(super) fn apply_majority_spellings(
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

#[cfg(test)]
mod tests {

    use crate::data::db::test_util::*;
    use crate::data::db::{BatchTrack, ScanCache};
    use crate::data::models::{Cuid, EventType, Song};

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
        db.upsert_tracks(&tracks, &mut cache).unwrap();

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
        db.upsert_tracks(
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
    fn a_moved_file_keeps_its_history() {
        let (db, path) = named_db("song_move");
        let playlist = Cuid::new();
        db.upsert_playlist(&playlist, "Mix", None, false).unwrap();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "audio-1",
                ..track("/music/old/1.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let old_id = song_id_for(&db, "/music/old/1.flac");
        db.upsert_playlist_song(&playlist, &old_id).unwrap();
        db.set_favorite::<Song>(&old_id, true).unwrap();
        db.insert_event(EventType::Play, &old_id, None, Some(0), 0.0)
            .unwrap();

        let added = db
            .upsert_tracks(
                &[BatchTrack {
                    audio_hash: "audio-1",
                    ..track("/music/new/1.flac", false)
                }],
                &mut ScanCache::default(),
            )
            .unwrap();

        assert_eq!(added, 0, "a relink is an update, not an addition");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        assert!(db.get_song_by_path("/music/old/1.flac").unwrap().is_none());
        let song = db.get_song_by_path("/music/new/1.flac").unwrap().unwrap();
        assert_eq!(song.id, Cuid::for_song("audio-1", None));
        assert!(song.favorite);
        let tracks = db.get_playlist_songs(&playlist).unwrap();
        assert_eq!(tracks.len(), 1);
        let events = db.get_events_by_type(EventType::Play).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].song_id, song.id);
        cleanup(&path);
    }

    #[test]
    fn a_moved_and_retagged_file_keeps_its_history() {
        let (db, path) = named_db("song_move_retag");
        let dir = std::env::temp_dir().join(format!("vleer_move_retag_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let new_path = dir.join("1.flac");
        std::fs::write(&new_path, b"x").unwrap();
        let new_path = new_path.to_string_lossy().into_owned();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "audio-mr",
                album: Some("Loose"),
                artists: &["Artist"],
                ..track("/gone/1.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let old_id = song_id_for(&db, "/gone/1.flac");
        db.set_favorite::<Song>(&old_id, true).unwrap();

        let added = db
            .upsert_tracks(
                &[BatchTrack {
                    audio_hash: "audio-mr",
                    album: Some("Proper"),
                    artists: &["Artist"],
                    ..track(&new_path, false)
                }],
                &mut ScanCache::default(),
            )
            .unwrap();

        assert_eq!(added, 0);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        let song = db.get_song_by_path(&new_path).unwrap().unwrap();
        assert!(song.favorite);
        assert_eq!(
            song.id,
            Cuid::for_song("audio-mr", Some(&Cuid::for_album("Proper", "Artist")))
        );
        std::fs::remove_dir_all(&dir).unwrap();
        cleanup(&path);
    }

    #[test]
    fn an_unhashed_stale_row_merges_into_its_live_twin() {
        let (db, path) = named_db("song_relink_unhashed");
        let dir = std::env::temp_dir().join(format!("vleer_relink_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let live_path = dir.join("1.flac");
        std::fs::write(&live_path, b"x").unwrap();
        let live_path = live_path.to_string_lossy().into_owned();

        db.upsert_tracks(
            &[
                BatchTrack {
                    audio_hash: "old",
                    album: Some("Loose"),
                    artists: &["Artist"],
                    ..track("/gone/1.flac", false)
                },
                BatchTrack {
                    audio_hash: "new",
                    album: Some("Proper"),
                    artists: &["Artist"],
                    ..track(&live_path, false)
                },
            ],
            &mut ScanCache::default(),
        )
        .unwrap();
        db.conn
            .lock()
            .execute(
                "UPDATE songs SET audio_hash = NULL WHERE file_path = '/gone/1.flac'",
                [],
            )
            .unwrap();
        let stale = song_id_for(&db, "/gone/1.flac");
        let playlist = Cuid::new();
        db.upsert_playlist(&playlist, "Mix", None, false).unwrap();
        db.upsert_playlist_song(&playlist, &stale).unwrap();
        db.set_favorite::<Song>(&stale, true).unwrap();

        assert_eq!(db.relink_unhashed_songs().unwrap(), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        let song = db.get_song_by_path(&live_path).unwrap().unwrap();
        assert!(song.favorite);
        assert_eq!(
            db.get_playlist_songs(&playlist).unwrap()[0].song.id,
            song.id
        );
        std::fs::remove_dir_all(&dir).unwrap();
        cleanup(&path);
    }

    #[test]
    fn retagging_keeps_the_id() {
        let (db, path) = named_db("song_identity");
        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "audio-2",
                ..track("/music/a/2.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let id = song_id_for(&db, "/music/a/2.flac");

        db.upsert_tracks(
            &[BatchTrack {
                title: "Retitled",
                audio_hash: "audio-2",
                ..track("/music/a/2.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        assert_eq!(song_id_for(&db, "/music/a/2.flac"), id);
        cleanup(&path);
    }

    #[test]
    fn the_same_recording_on_two_albums_keeps_a_row_per_album() {
        let (db, path) = named_db("song_shared_across_albums");
        let dir = std::env::temp_dir().join(format!("vleer_shared_{}", std::process::id()));
        for sub in ["best-of", "comp2"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            std::fs::write(dir.join(sub).join("1.flac"), b"x").unwrap();
        }
        let best_of_path = dir.join("best-of/1.flac").to_string_lossy().into_owned();
        let comp2_path = dir.join("comp2/1.flac").to_string_lossy().into_owned();
        db.upsert_tracks(
            &[
                BatchTrack {
                    audio_hash: "shared",
                    album: Some("Greatest Hits"),
                    artists: &["Artist"],
                    ..track(&best_of_path, false)
                },
                BatchTrack {
                    audio_hash: "shared",
                    album: Some("Welcome to the Jungle, Vol. 2"),
                    artists: &["Artist"],
                    ..track(&comp2_path, false)
                },
            ],
            &mut ScanCache::default(),
        )
        .unwrap();

        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM songs"),
            2,
            "same recording, two different albums -- two rows"
        );
        let best_of = db
            .get_song_by_path(&best_of_path)
            .unwrap()
            .unwrap()
            .album_id
            .unwrap();
        let comp2 = db
            .get_song_by_path(&comp2_path)
            .unwrap()
            .unwrap()
            .album_id
            .unwrap();
        assert_ne!(best_of, comp2);
        assert_eq!(db.get_album_songs(&best_of).unwrap().len(), 1);
        assert_eq!(db.get_album_songs(&comp2).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
        cleanup(&path);
    }

    #[test]
    fn a_copy_with_identical_audio_in_the_same_album_still_collapses() {
        let (db, path) = named_db("song_copy_same_album");
        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "dup",
                album: Some("An Album"),
                ..track("/music/a/1.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        let added = db
            .upsert_tracks(
                &[BatchTrack {
                    audio_hash: "dup",
                    album: Some("An Album"),
                    ..track("/music/a/1-copy.flac", false)
                }],
                &mut ScanCache::default(),
            )
            .unwrap();

        assert_eq!(added, 0, "same audio, same album -- still one song");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        cleanup(&path);
    }

    #[test]
    fn a_copy_with_identical_audio_collapses_onto_one_row() {
        let (db, path) = named_db("song_copy");
        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "audio-3",
                ..track("/music/a/2.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        let added = db
            .upsert_tracks(
                &[BatchTrack {
                    audio_hash: "audio-3",
                    ..track("/music/b/2.flac", false)
                }],
                &mut ScanCache::default(),
            )
            .unwrap();

        assert_eq!(added, 0, "identical audio is the same song, not a new one");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        assert!(db.get_song_by_path("/music/a/2.flac").unwrap().is_none());
        assert!(db.get_song_by_path("/music/b/2.flac").unwrap().is_some());
        cleanup(&path);
    }

    #[test]
    fn a_copy_does_not_steal_the_row_while_the_original_still_exists_on_disk() {
        let (db, path) = named_db("song_copy_original_survives");
        let dir = std::env::temp_dir().join(format!("vleer_dup_survive_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.flac");
        let copy = dir.join("copy.flac");
        std::fs::write(&original, b"x").unwrap();
        std::fs::write(&copy, b"x").unwrap();
        let original = original.to_str().unwrap();
        let copy = copy.to_str().unwrap();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "dup-survive",
                ..track(original, false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        let added = db
            .upsert_tracks(
                &[BatchTrack {
                    audio_hash: "dup-survive",
                    ..track(copy, false)
                }],
                &mut ScanCache::default(),
            )
            .unwrap();

        assert_eq!(added, 0, "the copy is a duplicate, not a new song");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        assert!(
            db.get_song_by_path(original).unwrap().is_some(),
            "original file still on disk -- row must stay pointed at it"
        );
        assert!(
            db.get_song_by_path(copy).unwrap().is_none(),
            "the copy must not steal the row while the original survives"
        );

        std::fs::remove_dir_all(&dir).unwrap();
        cleanup(&path);
    }

    #[test]
    fn a_copy_takes_over_the_row_once_the_original_is_gone() {
        let (db, path) = named_db("song_copy_original_gone");
        let dir = std::env::temp_dir().join(format!("vleer_dup_gone_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.flac");
        let copy = dir.join("copy.flac");
        std::fs::write(&original, b"x").unwrap();
        std::fs::write(&copy, b"x").unwrap();
        let original_str = original.to_str().unwrap();
        let copy_str = copy.to_str().unwrap();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "dup-gone",
                ..track(original_str, false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        std::fs::remove_file(&original).unwrap();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "dup-gone",
                ..track(copy_str, false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        assert!(db.get_song_by_path(original_str).unwrap().is_none());
        assert!(db.get_song_by_path(copy_str).unwrap().is_some());

        std::fs::remove_dir_all(&dir).unwrap();
        cleanup(&path);
    }

    #[test]
    fn replaced_audio_at_the_same_path_gets_a_new_id_and_keeps_playlists() {
        let (db, path) = named_db("song_replace");
        let playlist = Cuid::new();
        db.upsert_playlist(&playlist, "Mix", None, false).unwrap();
        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "before",
                ..track("/music/3.flac", false)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let before = song_id_for(&db, "/music/3.flac");
        db.upsert_playlist_song(&playlist, &before).unwrap();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "after",
                ..track("/music/3.flac", true)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();
        let after = song_id_for(&db, "/music/3.flac");
        assert_ne!(before, after);
        assert_eq!(after, Cuid::for_song("after", None));
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM playlist_songs"), 1);
        assert_eq!(db.get_playlist_songs(&playlist).unwrap()[0].song.id, after);
        cleanup(&path);
    }

    #[test]
    fn audio_replaced_to_match_another_cataloged_song_merges_into_it() {
        let (db, path) = named_db("song_replace_collision");
        db.upsert_tracks(
            &[
                BatchTrack {
                    audio_hash: "shared",
                    ..track("/music/target.flac", false)
                },
                BatchTrack {
                    audio_hash: "before",
                    ..track("/music/source.flac", false)
                },
            ],
            &mut ScanCache::default(),
        )
        .unwrap();
        let target = song_id_for(&db, "/music/target.flac");
        db.set_favorite::<Song>(&target, false).unwrap();
        let source = song_id_for(&db, "/music/source.flac");
        db.set_favorite::<Song>(&source, true).unwrap();

        db.upsert_tracks(
            &[BatchTrack {
                audio_hash: "shared",
                ..track("/music/source.flac", true)
            }],
            &mut ScanCache::default(),
        )
        .unwrap();

        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        assert!(db.get_song_by_path("/music/target.flac").unwrap().is_none());
        let merged = db.get_song_by_path("/music/source.flac").unwrap().unwrap();
        assert_eq!(merged.id, target);
        assert!(merged.favorite, "favorite carries over from the merged row");
        cleanup(&path);
    }

    #[test]
    fn duplicates_within_a_single_batch_collapse_too() {
        let (db, path) = named_db("song_copy_same_batch");
        let added = db
            .upsert_tracks(
                &[
                    BatchTrack {
                        audio_hash: "dup",
                        ..track("/music/a/x.flac", false)
                    },
                    BatchTrack {
                        audio_hash: "dup",
                        ..track("/music/b/x.flac", false)
                    },
                ],
                &mut ScanCache::default(),
            )
            .unwrap();
        assert_eq!(added, 1, "one real song, one duplicate, in the same batch");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), 1);
        cleanup(&path);
    }

    #[test]
    fn duplicates_scattered_through_a_large_batch_still_collapse() {
        let (db, path) = named_db("song_copy_large_batch");
        let n = 600;
        let paths: Vec<String> = (0..n).map(|i| format!("/music/{i}.flac")).collect();
        let hashes: Vec<String> = (0..n)
            .map(|i| match i {
                5 => "dup".to_string(),
                595 => "dup".to_string(),
                _ => format!("hash-{i}"),
            })
            .collect();
        let batch: Vec<BatchTrack<'_>> = (0..n)
            .map(|i| BatchTrack {
                audio_hash: &hashes[i],
                ..track(&paths[i], false)
            })
            .collect();
        let added = db.upsert_tracks(&batch, &mut ScanCache::default()).unwrap();
        assert_eq!(added, n - 1, "one pair should collapse into a single song");
        assert_eq!(count(&db, "SELECT COUNT(*) FROM songs"), (n - 1) as i64);
        cleanup(&path);
    }
}
