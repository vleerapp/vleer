use std::time::Instant;
use vleer::data::{db::repo::Database, models::SongSort};

fn temp_db() -> (Database, std::path::PathBuf) {
    let path = std::path::PathBuf::from(format!("/tmp/vleer_bench_{}.db", std::process::id()));
    let db = Database::new(&path).expect("failed to create test db");
    (db, path)
}

fn cleanup(path: &std::path::PathBuf) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn bench_sqlite_operations() {
    let (db, path) = temp_db();

    const SONGS: usize = 10_000;
    const ARTISTS: usize = 100;
    const ALBUMS: usize = 500;

    let t = Instant::now();
    for i in 0..ARTISTS {
        db.upsert_artist(&format!("Artist {i}")).unwrap();
    }
    println!("insert {ARTISTS} artists:       {:>10?}", t.elapsed());

    let artists = db.get_artists("", 0, ARTISTS as i64).unwrap();

    let t = Instant::now();
    for i in 0..ALBUMS {
        let artist_id = artists.get(i % artists.len()).map(|a| &a.id);
        db.upsert_album(&format!("Album {i}"), artist_id, None)
            .unwrap();
    }
    println!("insert {ALBUMS} albums:         {:>10?}", t.elapsed());

    let albums = db.get_albums("", 0, ALBUMS as i64).unwrap();

    let t = Instant::now();
    for i in 0..SONGS {
        let album = &albums[i % albums.len()];
        let artist_id = artists.get(i % artists.len()).map(|a| &a.id);
        db.upsert_song(
            &format!("Song Title {i}"),
            artist_id,
            Some(&album.id),
            &format!("/music/song_{i}.mp3"),
            180 + (i as i32 % 300),
            Some((i as i32 % 20) + 1),
            Some(2000 + (i as i32 % 24)),
            None,
            None,
            1_000_000,
            i as i64,
            None,
        )
        .unwrap();
    }
    println!("insert {SONGS} songs:         {:>10?}", t.elapsed());

    let t = Instant::now();
    for _ in 0..100 {
        db.get_songs_count(None).unwrap();
    }
    println!("get_songs_count       x100:  {:>10?}", t.elapsed());

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
        db.get_albums_count("").unwrap();
    }
    println!("get_albums_count      x100:  {:>10?}", t.elapsed());

    let paths: Vec<String> = (0..1000).map(|i| format!("/music/song_{i}.mp3")).collect();
    let t = Instant::now();
    db.delete_songs_by_paths(&paths).unwrap();
    println!("delete 1000 songs (batch):   {:>10?}", t.elapsed());

    cleanup(&path);
}
