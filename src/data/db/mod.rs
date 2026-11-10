use crate::data::search::SearchIndex;
use anyhow::Result;
use gpui::Global;
use parking_lot::Mutex;
use rusqlite::Connection;
use rust_embed::RustEmbed;
use std::path::Path;
use std::sync::Arc;

mod albums;
mod artists;
mod genres;
mod images;
mod library;
mod metadata;
mod playlists;
mod scan;
mod search;
mod songs;

pub use images::AlbumImageTarget;
pub use metadata::{AlbumLookup, AlbumMetadata, ArtistMetadata, SongBackfill, SongLookup};
pub use scan::{BatchTrack, ScanCache, write_profile};

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

    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current as usize > files.len() {
        anyhow::bail!(
            "database schema version {current} is newer than known migrations ({})",
            files.len()
        );
    }

    let tx = conn.transaction()?;
    for (i, (name, sql)) in files.iter().enumerate().skip(current as usize) {
        tx.execute_batch(sql)
            .map_err(|e| anyhow::anyhow!("migration {name} failed: {e}"))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
    }
    tx.commit()?;
    Ok(())
}

fn collect_mapped<T, F>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    mapper: F,
) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt
        .query_map(params, mapper)?
        .collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(rows)
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    pub image_conn: Arc<Mutex<Connection>>,
    image_write_conn: Arc<Mutex<Connection>>,

    index_conn: Arc<Mutex<Connection>>,
    search_index: Arc<Mutex<SearchIndex>>,
    path: Arc<std::path::PathBuf>,
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(",")
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
        let index_conn = open_connection(path, 5000)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
            image_conn: Arc::new(Mutex::new(image_conn)),
            image_write_conn: Arc::new(Mutex::new(image_write_conn)),
            index_conn: Arc::new(Mutex::new(index_conn)),
            search_index: Arc::new(Mutex::new(SearchIndex::default())),
            path: Arc::new(path.to_path_buf()),
        };
        db.rebuild_search_index();
        Ok(db)
    }
}

#[cfg(test)]
mod test_util {
    use crate::data::db::{BatchTrack, Database};
    use crate::data::models::Cuid;

    pub(super) fn temp_db() -> (Database, std::path::PathBuf) {
        let path = std::path::PathBuf::from(format!("/tmp/vleer_bench_{}.db", std::process::id()));
        let db = Database::new(&path).expect("failed to create test db");
        (db, path)
    }

    pub(super) fn named_db(name: &str) -> (Database, std::path::PathBuf) {
        let path = std::path::PathBuf::from(format!("/tmp/vleer_{name}_{}.db", std::process::id()));
        cleanup(&path);
        let db = Database::new(&path).expect("failed to create test db");
        (db, path)
    }

    pub(super) fn cleanup(path: &std::path::PathBuf) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    pub(super) fn track<'a>(file_path: &'a str, recheck_image: bool) -> BatchTrack<'a> {
        BatchTrack {
            title: "Title",
            artists: &[],
            genres: &[],
            album: None,
            album_artist: None,
            file_path,
            audio_hash: file_path,
            duration: 180_000,
            track_number: None,
            year: None,
            recheck_image,
            file_size: 1_000,
            file_modified: 42,
            lufs: None,
            isrc: None,
            upc: None,
            omm_song_id: None,
            omm_album_id: None,
            omm_artist_id: None,
        }
    }

    pub(super) fn song_id_for(db: &Database, file_path: &str) -> Cuid {
        db.get_song_by_path(file_path)
            .unwrap()
            .expect("song should exist")
            .id
    }

    pub(super) fn image_state(db: &Database, file_path: &str) -> (Option<String>, bool) {
        let target = db
            .get_song_image_target(&song_id_for(db, file_path))
            .unwrap()
            .expect("target should exist");
        (target.image_id, target.image_checked)
    }

    pub(super) fn artist_names(db: &Database) -> Vec<String> {
        let conn = db.conn.lock();
        let mut stmt = conn
            .prepare("SELECT name FROM artists ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    pub(super) fn count(db: &Database, sql: &str) -> i64 {
        db.conn.lock().query_row(sql, [], |row| row.get(0)).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db::test_util::*;
    use crate::data::db::{BatchTrack, ScanCache};
    use crate::data::models::SongSort;

    use std::time::Instant;

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
                        album_artist: None,
                        file_path: &paths[i],
                        audio_hash: &paths[i],
                        duration: 180_000,
                        track_number: None,
                        year: None,
                        recheck_image: true,
                        file_size: 1_000,
                        file_modified: 42,
                        lufs: None,
                        isrc: None,
                        upc: None,
                        omm_song_id: None,
                        omm_album_id: None,
                        omm_artist_id: None,
                    })
                    .collect();
                db.upsert_tracks(&batch, &mut cache).unwrap();
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
                    let _ = db3.get_recently_added(100);
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
                    album_artist: None,
                    file_path: &paths[i],
                    audio_hash: &paths[i],
                    duration: (180 + (i as i32 % 300)) * 1000,
                    track_number: Some((i as i32 % 20) + 1),
                    year: Some(2000 + (i as i32 % 24)),
                    recheck_image: true,
                    file_size: 1_000_000,
                    file_modified: i as i64,
                    lufs: None,
                    isrc: None,
                    upc: None,
                    omm_song_id: None,
                    omm_album_id: None,
                    omm_artist_id: None,
                })
                .collect();
            db.upsert_tracks(&batch, &mut cache).unwrap();
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
            db.count_songs(None).unwrap();
        }
        println!("count_songs       x100:  {:>10?}", t.elapsed());

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
            db.count_albums("").unwrap();
        }
        println!("count_albums      x100:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..10 {
            db.get_recently_added(100).unwrap();
        }
        println!("get_recently_added     x10:  {:>10?}", t.elapsed());

        let t = Instant::now();
        for _ in 0..10 {
            db.get_recently_played(100).unwrap();
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
