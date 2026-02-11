use crate::data::{
    db::models::*,
    models::{Cuid, Event, EventContext, EventType, SongSort},
};
use sqlx::SqlitePool;

pub async fn get_song(pool: &SqlitePool, id: Cuid) -> sqlx::Result<Option<SongRow>> {
    sqlx::query_as::<_, SongRow>("SELECT * FROM songs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_song_by_path(pool: &SqlitePool, file_path: &str) -> sqlx::Result<Option<SongRow>> {
    sqlx::query_as::<_, SongRow>("SELECT * FROM songs WHERE file_path = ?")
        .bind(file_path)
        .fetch_optional(pool)
        .await
}

pub async fn upsert_song(
    pool: &SqlitePool,
    title: &str,
    artist_id: Option<&Cuid>,
    album_id: Option<&Cuid>,
    file_path: &str,
    duration: i32,
    track_number: Option<i32>,
    year: Option<i32>,
    genre: Option<&str>,
    image_id: Option<&str>,
    file_size: i64,
    file_modified: i64,
    lufs: Option<f32>,
) -> sqlx::Result<()> {
    let year_str = year.map(|y| y.to_string());

    sqlx::query(
        "INSERT INTO songs (id, title, artist_id, album_id, file_path, file_size, file_modified, genre, date, duration, image_id, track_number, lufs)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(file_path) DO UPDATE SET
            title = excluded.title,
            artist_id = excluded.artist_id,
            album_id = excluded.album_id,
            file_size = excluded.file_size,
            file_modified = excluded.file_modified,
            genre = excluded.genre,
            date = excluded.date,
            duration = excluded.duration,
            image_id = excluded.image_id,
            track_number = excluded.track_number,
            lufs = excluded.lufs"
    )
    .bind(Cuid::new())
    .bind(title)
    .bind(artist_id)
    .bind(album_id)
    .bind(file_path)
    .bind(file_size)
    .bind(file_modified)
    .bind(genre)
    .bind(year_str)
    .bind(duration)
    .bind(image_id)
    .bind(track_number)
    .bind(lufs)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete_song(pool: &SqlitePool, id: &Cuid) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM songs WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_song_by_path(pool: &SqlitePool, file_path: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM songs WHERE file_path = ?")
        .bind(file_path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_songs_paged(
    pool: &SqlitePool,
    offset: i64,
    limit: i64,
) -> sqlx::Result<Vec<SongRow>> {
    sqlx::query_as::<_, SongRow>("SELECT * FROM songs ORDER BY date_added DESC LIMIT ? OFFSET ?")
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

pub async fn get_songs_count(pool: &SqlitePool) -> sqlx::Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM songs")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

pub async fn get_artist(pool: &SqlitePool, id: Cuid) -> sqlx::Result<Option<ArtistRow>> {
    sqlx::query_as::<_, ArtistRow>("SELECT * FROM artists WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn upsert_artist(pool: &SqlitePool, name: &str) -> sqlx::Result<Cuid> {
    sqlx::query_scalar(
        "INSERT INTO artists (id, name) VALUES (?, ?)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name
         RETURNING id",
    )
    .bind(Cuid::new())
    .bind(name)
    .fetch_one(pool)
    .await
}

pub async fn get_album(pool: &SqlitePool, id: Cuid) -> sqlx::Result<Option<AlbumRow>> {
    sqlx::query_as::<_, AlbumRow>("SELECT * FROM albums WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_albums_count_filtered(pool: &SqlitePool, query: &str) -> sqlx::Result<i64> {
    let query_lower = query.to_lowercase();

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(DISTINCT al.id)
        FROM albums al
        LEFT JOIN artists ar ON al.artist_id = ar.id
        WHERE
            ?1 = ''
            OR LOWER(al.title) LIKE '%' || ?1 || '%'
            OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || ?1 || '%')
        "#,
    )
    .bind(&query_lower)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

pub async fn get_albums_paged_filtered(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<AlbumListRow>> {
    let query_lower = query.to_lowercase();

    sqlx::query_as::<_, AlbumListRow>(
        r#"
        SELECT
            al.id,
            al.title,
            ar.name AS artist_name,
            al.image_id,
            MIN(s.date) AS year
        FROM albums al
        LEFT JOIN artists ar ON al.artist_id = ar.id
        LEFT JOIN songs s ON s.album_id = al.id
        WHERE
            ?1 = ''
            OR LOWER(al.title) LIKE '%' || ?1 || '%'
            OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || ?1 || '%')
        GROUP BY al.id, al.title, ar.name, al.image_id
        ORDER BY LOWER(al.title) ASC
        LIMIT ?2 OFFSET ?3
        "#,
    )
    .bind(&query_lower)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn get_albums_with_artist(pool: &SqlitePool) -> sqlx::Result<Vec<AlbumListRow>> {
    sqlx::query_as::<_, AlbumListRow>(
        r#"
        SELECT
            al.id,
            al.title,
            ar.name AS artist_name,
            al.image_id,
            MIN(s.date) AS year
        FROM albums al
        LEFT JOIN artists ar ON al.artist_id = ar.id
        LEFT JOIN songs s ON s.album_id = al.id
        GROUP BY al.id, al.title, ar.name, al.image_id
        ORDER BY LOWER(al.title) ASC
        "#,
    )
    .fetch_all(pool)
    .await
}

pub async fn upsert_album(
    pool: &SqlitePool,
    title: &str,
    artist_id: Option<&Cuid>,
    image_id: Option<&str>,
) -> sqlx::Result<Cuid> {
    sqlx::query_scalar(
        "INSERT INTO albums (id, title, artist_id, image_id)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(title, artist_id) DO UPDATE SET
            image_id = excluded.image_id
         RETURNING id",
    )
    .bind(Cuid::new())
    .bind(title)
    .bind(artist_id)
    .bind(image_id)
    .fetch_one(pool)
    .await
}

pub async fn delete_album(pool: &SqlitePool, id: &Cuid) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM albums WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_songs_count_filtered(pool: &SqlitePool, query: &str) -> sqlx::Result<i64> {
    let query_lower = query.to_lowercase();

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(DISTINCT s.id)
        FROM songs s
        LEFT JOIN artists ar ON s.artist_id = ar.id
        LEFT JOIN albums al ON s.album_id = al.id
        WHERE
            ?1 = ''
            OR LOWER(s.title) LIKE '%' || ?1 || '%'
            OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || ?1 || '%')
            OR (al.id IS NOT NULL AND LOWER(al.title) LIKE '%' || ?1 || '%')
        "#,
    )
    .bind(&query_lower)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

fn song_order_clause(sort: SongSort, ascending: bool) -> &'static str {
    match sort {
        SongSort::Title => {
            if ascending {
                "LOWER(s.title) ASC, s.id ASC"
            } else {
                "LOWER(s.title) DESC, s.id ASC"
            }
        }
        SongSort::Album => {
            if ascending {
                "LOWER(COALESCE(al.title, '')) ASC, s.id ASC"
            } else {
                "LOWER(COALESCE(al.title, '')) DESC, s.id ASC"
            }
        }
        SongSort::Duration => {
            if ascending {
                "s.duration ASC, s.id ASC"
            } else {
                "s.duration DESC, s.id ASC"
            }
        }
        SongSort::Default => "s.date_added DESC, s.id ASC",
    }
}

pub async fn get_songs_paged_filtered(
    pool: &SqlitePool,
    query: &str,
    sort: SongSort,
    ascending: bool,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<SongListRow>> {
    let query_lower = query.to_lowercase();
    let order_clause = song_order_clause(sort, ascending);

    let sql = format!(
        r#"
        SELECT
            s.id,
            s.title,
            ar.name AS artist_name,
            al.title AS album_title,
            s.duration,
            s.image_id
        FROM songs s
        LEFT JOIN artists ar ON s.artist_id = ar.id
        LEFT JOIN albums al ON s.album_id = al.id
        WHERE
            ?1 = ''
            OR LOWER(s.title) LIKE '%' || ?1 || '%'
            OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || ?1 || '%')
            OR (al.id IS NOT NULL AND LOWER(al.title) LIKE '%' || ?1 || '%')
        ORDER BY {order_clause}
        LIMIT ?2 OFFSET ?3
        "#
    );

    sqlx::query_as::<_, SongListRow>(&sql)
        .bind(&query_lower)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

pub async fn get_song_ids_from_offset_filtered(
    pool: &SqlitePool,
    query: &str,
    sort: SongSort,
    ascending: bool,
    offset: i64,
) -> sqlx::Result<Vec<Cuid>> {
    let query_lower = query.to_lowercase();
    let order_clause = song_order_clause(sort, ascending);

    let sql = format!(
        r#"
        SELECT s.id
        FROM songs s
        LEFT JOIN artists ar ON s.artist_id = ar.id
        LEFT JOIN albums al ON s.album_id = al.id
        WHERE
            ?1 = ''
            OR LOWER(s.title) LIKE '%' || ?1 || '%'
            OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || ?1 || '%')
            OR (al.id IS NOT NULL AND LOWER(al.title) LIKE '%' || ?1 || '%')
        ORDER BY {order_clause}
        LIMIT -1 OFFSET ?2
        "#
    );

    let rows: Vec<(Cuid,)> = sqlx::query_as(&sql)
        .bind(&query_lower)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn get_album_songs(pool: &SqlitePool, album_id: &Cuid) -> sqlx::Result<Vec<SongRow>> {
    sqlx::query_as::<_, SongRow>("SELECT * FROM songs WHERE album_id = ? ORDER BY track_number ASC")
        .bind(album_id)
        .fetch_all(pool)
        .await
}

pub async fn get_artists_count_filtered(pool: &SqlitePool, query: &str) -> sqlx::Result<i64> {
    let query_lower = query.to_lowercase();

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(DISTINCT ar.id)
        FROM artists ar
        WHERE
            ?1 = ''
            OR LOWER(ar.name) LIKE '%' || ?1 || '%'
        "#,
    )
    .bind(&query_lower)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

pub async fn get_artists_paged_filtered(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<ArtistListRow>> {
    let query_lower = query.to_lowercase();

    sqlx::query_as::<_, ArtistListRow>(
        r#"
        SELECT
            ar.id,
            ar.name,
            ar.image_id
        FROM artists ar
        WHERE
            ?1 = ''
            OR LOWER(ar.name) LIKE '%' || ?1 || '%'
        ORDER BY LOWER(ar.name) ASC
        LIMIT ?2 OFFSET ?3
        "#,
    )
    .bind(&query_lower)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn get_playlist(pool: &SqlitePool, id: &Cuid) -> sqlx::Result<Option<PlaylistRow>> {
    sqlx::query_as::<_, PlaylistRow>("SELECT * FROM playlists WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn upsert_playlist(
    pool: &SqlitePool,
    id: &Cuid,
    name: &str,
    description: Option<&str>,
    image_id: Option<&str>,
    pinned: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO playlists (id, name, description, image_id, pinned)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            image_id = excluded.image_id,
            pinned = excluded.pinned,
            date_updated = DATETIME('now')",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(image_id)
    .bind(pinned)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete_playlist(pool: &SqlitePool, id: &Cuid) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM playlists WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn upsert_playlist_song(
    pool: &SqlitePool,
    playlist_id: &Cuid,
    song_id: &Cuid,
    position: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO playlist_tracks (id, playlist_id, song_id, position)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(playlist_id, song_id) DO UPDATE SET
            position = excluded.position",
    )
    .bind(Cuid::new())
    .bind(playlist_id)
    .bind(song_id)
    .bind(position)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete_playlist_song(
    pool: &SqlitePool,
    playlist_id: &Cuid,
    song_id: &Cuid,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM playlist_tracks WHERE playlist_id = ? AND song_id = ?")
        .bind(playlist_id)
        .bind(song_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_playlist_songs(
    pool: &SqlitePool,
    playlist_id: &Cuid,
) -> sqlx::Result<Vec<PlaylistTrackRow>> {
    sqlx::query(
        r#"
        SELECT
            pt.id,
            pt.playlist_id,
            pt.position,
            s.*
        FROM playlist_tracks pt
        JOIN songs s ON s.id = pt.song_id
        WHERE pt.playlist_id = ?
        ORDER BY pt.position ASC
        "#,
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| PlaylistTrackRow::from_row(&row))
    .collect()
}

pub async fn get_event(pool: &SqlitePool, id: &Cuid) -> sqlx::Result<Option<Event>> {
    let row: Option<EventRow> = sqlx::query_as::<_, EventRow>("SELECT * FROM events WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|r| r.into_event()))
}

pub async fn insert_event(
    pool: &SqlitePool,
    event_type: EventType,
    context_id: Option<&Cuid>,
) -> sqlx::Result<Cuid> {
    let id = Cuid::new();
    let event_type_str = match event_type {
        EventType::Play => "PLAY",
        EventType::Stop => "STOP",
        EventType::Pause => "PAUSE",
        EventType::Resume => "RESUME",
    };

    sqlx::query("INSERT INTO events (id, event_type, context_id) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(event_type_str)
        .bind(context_id)
        .execute(pool)
        .await?;

    Ok(id)
}

pub async fn get_events_by_type(
    pool: &SqlitePool,
    event_type: EventType,
) -> sqlx::Result<Vec<Event>> {
    let event_type_str = match event_type {
        EventType::Play => "PLAY",
        EventType::Stop => "STOP",
        EventType::Pause => "PAUSE",
        EventType::Resume => "RESUME",
    };

    let rows: Vec<EventRow> = sqlx::query_as::<_, EventRow>(
        "SELECT * FROM events WHERE event_type = ? ORDER BY timestamp DESC",
    )
    .bind(event_type_str)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.into_event()).collect())
}

pub async fn insert_event_context(
    pool: &SqlitePool,
    song_id: Option<&Cuid>,
    playlist_id: Option<&Cuid>,
) -> sqlx::Result<Cuid> {
    let id = Cuid::new();
    sqlx::query("INSERT INTO event_contexts (id, song_id, playlist_id) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(song_id)
        .bind(playlist_id)
        .execute(pool)
        .await?;

    Ok(id)
}

pub async fn get_event_context(pool: &SqlitePool, id: &Cuid) -> sqlx::Result<Option<EventContext>> {
    let row: Option<EventContextRow> =
        sqlx::query_as::<_, EventContextRow>("SELECT * FROM event_contexts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;

    Ok(row.map(|r| EventContext {
        id: r.id,
        song_id: r.song_id,
        playlist_id: r.playlist_id,
        date_created: r.date_created,
    }))
}

pub async fn get_event_context_by_song(
    pool: &SqlitePool,
    song_id: &Cuid,
) -> sqlx::Result<Vec<EventContext>> {
    let rows: Vec<EventContextRow> =
        sqlx::query_as::<_, EventContextRow>("SELECT * FROM event_contexts WHERE song_id = ?")
            .bind(song_id)
            .fetch_all(pool)
            .await?;

    Ok(rows
        .into_iter()
        .map(|r| EventContext {
            id: r.id,
            song_id: r.song_id,
            playlist_id: r.playlist_id,
            date_created: r.date_created,
        })
        .collect())
}

pub async fn get_event_context_by_playlist(
    pool: &SqlitePool,
    playlist_id: &Cuid,
) -> sqlx::Result<Vec<EventContext>> {
    let rows: Vec<EventContextRow> =
        sqlx::query_as::<_, EventContextRow>("SELECT * FROM event_contexts WHERE playlist_id = ?")
            .bind(playlist_id)
            .fetch_all(pool)
            .await?;

    Ok(rows
        .into_iter()
        .map(|r| EventContext {
            id: r.id,
            song_id: r.song_id,
            playlist_id: r.playlist_id,
            date_created: r.date_created,
        })
        .collect())
}

pub async fn set_favorite<T: Toggleable>(
    pool: &SqlitePool,
    id: &Cuid,
    favorite: bool,
) -> sqlx::Result<()> {
    let query = format!(
        "UPDATE {} SET favorite = ? WHERE {} = ?",
        T::TABLE,
        T::ID_COL
    );
    sqlx::query(&query)
        .bind(favorite)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_pinned<T: Toggleable>(
    pool: &SqlitePool,
    id: &Cuid,
    pinned: bool,
) -> sqlx::Result<()> {
    let query = format!("UPDATE {} SET pinned = ? WHERE {} = ?", T::TABLE, T::ID_COL);
    sqlx::query(&query)
        .bind(pinned)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn search_library(pool: &SqlitePool, query: &str) -> sqlx::Result<Vec<SearchResultRow>> {
    let query_lower = query.to_lowercase();
    let query_normalized: String = query_lower
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();

    sqlx::query_as::<_, SearchResultRow>(
        r#"
        WITH RECURSIVE
        search_params AS (
            SELECT 
                ?1 AS original_query,
                ?2 AS query_lower,
                ?3 AS query_normalized
        ),
        album_song_counts AS (
            SELECT 
                album_id,
                COUNT(*) AS song_count
            FROM songs
            WHERE album_id IS NOT NULL
            GROUP BY album_id
        ),
        song_matches AS (
            SELECT DISTINCT
                s.id,
                s.title AS name,
                s.image_id AS image,
                'Song' AS item_type,
                s.title AS original_title,
                (CASE 
                    WHEN LOWER(s.title) = sp.query_lower 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(s.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') = sp.query_normalized 
                    THEN 1000 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(s.title) LIKE '%' || sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(s.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%' 
                    THEN 100 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(s.title) LIKE sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(s.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE sp.query_normalized || '%' 
                    THEN 50 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LENGTH(s.title) <= 30 
                    THEN 10 
                    ELSE 0 
                END) AS score
            FROM songs s
            CROSS JOIN search_params sp
            LEFT JOIN artists ar ON s.artist_id = ar.id
            LEFT JOIN albums al ON s.album_id = al.id
            WHERE 
                LOWER(s.title) LIKE '%' || sp.query_lower || '%'
                OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(s.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
                OR (ar.id IS NOT NULL AND (
                    LOWER(ar.name) LIKE '%' || sp.query_lower || '%'
                    OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(ar.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
                ))
                OR (al.id IS NOT NULL AND (
                    LOWER(al.title) LIKE '%' || sp.query_lower || '%'
                    OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(al.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
                ))
        ),
        album_matches AS (
            SELECT DISTINCT
                al.id,
                al.title AS name,
                al.image_id AS image,
                'Album' AS item_type,
                al.title AS original_title,
                (CASE 
                    WHEN LOWER(al.title) = sp.query_lower 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(al.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') = sp.query_normalized 
                    THEN 1000 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(al.title) LIKE '%' || sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(al.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%' 
                    THEN 100 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(al.title) LIKE sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(al.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE sp.query_normalized || '%' 
                    THEN 50 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LENGTH(al.title) <= 30 
                    THEN 10 
                    ELSE 0 
                END) AS score
            FROM albums al
            CROSS JOIN search_params sp
            LEFT JOIN artists ar ON al.artist_id = ar.id
            LEFT JOIN album_song_counts asc ON al.id = asc.album_id
            WHERE 
                (asc.song_count IS NULL OR asc.song_count > 1)
                AND (
                    LOWER(al.title) LIKE '%' || sp.query_lower || '%'
                    OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(al.title), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
                    OR (ar.id IS NOT NULL AND (
                        LOWER(ar.name) LIKE '%' || sp.query_lower || '%'
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(ar.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
                    ))
                )
        ),
        artist_matches AS (
            SELECT DISTINCT
                ar.id,
                ar.name AS name,
                ar.image_id AS image,
                'Artist' AS item_type,
                ar.name AS original_title,
                (CASE 
                    WHEN LOWER(ar.name) = sp.query_lower 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(ar.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') = sp.query_normalized 
                    THEN 1000 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(ar.name) LIKE '%' || sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(ar.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%' 
                    THEN 100 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(ar.name) LIKE sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(ar.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE sp.query_normalized || '%' 
                    THEN 50 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LENGTH(ar.name) <= 30 
                    THEN 10 
                    ELSE 0 
                END) AS score
            FROM artists ar
            CROSS JOIN search_params sp
            WHERE 
                LOWER(ar.name) LIKE '%' || sp.query_lower || '%'
                OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(ar.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
        ),
        playlist_matches AS (
            SELECT DISTINCT
                p.id,
                p.name AS name,
                p.image_id AS image,
                'Playlist' AS item_type,
                p.name AS original_title,
                (CASE 
                    WHEN LOWER(p.name) = sp.query_lower 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(p.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') = sp.query_normalized 
                    THEN 1000 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(p.name) LIKE '%' || sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(p.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%' 
                    THEN 100 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LOWER(p.name) LIKE sp.query_lower || '%' 
                        OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(p.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE sp.query_normalized || '%' 
                    THEN 50 
                    ELSE 0 
                END) +
                (CASE 
                    WHEN LENGTH(p.name) <= 30 
                    THEN 10 
                    ELSE 0 
                END) AS score
            FROM playlists p
            CROSS JOIN search_params sp
            WHERE 
                LOWER(p.name) LIKE '%' || sp.query_lower || '%'
                OR REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(LOWER(p.name), '-', ''), '_', ''), '.', ''), '!', ''), '?', '') LIKE '%' || sp.query_normalized || '%'
        ),
        all_matches AS (
            SELECT * FROM song_matches
            UNION ALL
            SELECT * FROM album_matches
            UNION ALL
            SELECT * FROM artist_matches
            UNION ALL
            SELECT * FROM playlist_matches
        )
        SELECT 
            id,
            name,
            image,
            item_type
        FROM all_matches
        GROUP BY LOWER(REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(name, '-', ''), '_', ''), '.', ''), '!', ''), '?', ''))
        ORDER BY 
            MAX(score) DESC,
            LOWER(name) ASC
        "#,
    )
    .bind(query)
    .bind(&query_lower)
    .bind(&query_normalized)
    .fetch_all(pool)
    .await
}

pub async fn get_search_match_counts(
    pool: &SqlitePool,
    query: &str,
) -> sqlx::Result<SearchCountsRow> {
    let query_lower = query.to_lowercase();

    sqlx::query_as::<_, SearchCountsRow>(
        r#"
        WITH search_params AS (
            SELECT ?1 AS query_lower
        )
        SELECT 
            (
                SELECT COUNT(DISTINCT s.id)
                FROM songs s
                CROSS JOIN search_params sp
                LEFT JOIN artists ar ON s.artist_id = ar.id
                LEFT JOIN albums al ON s.album_id = al.id
                WHERE 
                    LOWER(s.title) LIKE '%' || sp.query_lower || '%'
                    OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || sp.query_lower || '%')
                    OR (al.id IS NOT NULL AND LOWER(al.title) LIKE '%' || sp.query_lower || '%')
            ) AS song_count,
            (
                SELECT COUNT(DISTINCT al.id)
                FROM albums al
                CROSS JOIN search_params sp
                LEFT JOIN artists ar ON al.artist_id = ar.id
                WHERE 
                    LOWER(al.title) LIKE '%' || sp.query_lower || '%'
                    OR (ar.id IS NOT NULL AND LOWER(ar.name) LIKE '%' || sp.query_lower || '%')
            ) AS album_count,
            (
                SELECT COUNT(DISTINCT ar.id)
                FROM artists ar
                CROSS JOIN search_params sp
                WHERE LOWER(ar.name) LIKE '%' || sp.query_lower || '%'
            ) AS artist_count,
            (
                SELECT COUNT(DISTINCT p.id)
                FROM playlists p
                CROSS JOIN search_params sp
                WHERE LOWER(p.name) LIKE '%' || sp.query_lower || '%'
            ) AS playlist_count
        "#,
    )
    .bind(&query_lower)
    .fetch_one(pool)
    .await
}

pub async fn upsert_image(pool: &SqlitePool, id: &str, data: &[u8]) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO images (id, data)
         VALUES (?, ?)
         ON CONFLICT(id) DO UPDATE SET
            data = excluded.data,
            date_updated = DATETIME('now')",
    )
    .bind(id)
    .bind(data)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_image(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<ImageRow>> {
    sqlx::query_as::<_, ImageRow>("SELECT * FROM images WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_image(pool: &SqlitePool, id: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM images WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_recently_added_items(
    pool: &SqlitePool,
    limit: i64,
) -> sqlx::Result<Vec<RecentItemRow>> {
    sqlx::query_as::<_, RecentItemRow>(
        r#"
        WITH recent_songs AS (
            SELECT 
                s.id,
                s.title,
                s.artist_id,
                s.album_id,
                s.image_id,
                s.date_added,
                s.date
            FROM songs s
            ORDER BY s.date_added DESC
            LIMIT ?
        ),
        image_groups AS (
            SELECT 
                COALESCE(rs.image_id, 'no_image_' || rs.id) as group_key,
                rs.image_id,
                rs.album_id,
                MAX(rs.date_added) as most_recent_date,
                COUNT(*) as song_count,
                MIN(rs.id) as first_song_id
            FROM recent_songs rs
            GROUP BY group_key, rs.image_id, rs.album_id
        )
        SELECT 
            ig.most_recent_date,
            ig.song_count,
            ig.first_song_id,
            s.title as first_song_title,
            s.artist_id as first_artist_id,
            ig.image_id,
            s.date as first_year,
            ig.album_id,
            al.title as album_title,
            ar.name as artist_name
        FROM image_groups ig
        JOIN songs s ON ig.first_song_id = s.id
        LEFT JOIN albums al ON ig.album_id = al.id
        LEFT JOIN artists ar ON s.artist_id = ar.id
        LEFT JOIN images img ON ig.image_id = img.id
        ORDER BY ig.most_recent_date DESC
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_pinned_items(
    pool: &SqlitePool,
) -> sqlx::Result<Vec<(Cuid, String, Option<String>, String)>> {
    let rows = sqlx::query_as::<_, PinnedItemRow>(
        r#"
        SELECT id, title AS name, image_id, 'Song' AS item_type
        FROM songs
        WHERE pinned = TRUE

        UNION ALL

        SELECT id, title AS name, image_id, 'Album' AS item_type
        FROM albums
        WHERE pinned = TRUE

        UNION ALL

        SELECT id, name AS name, image_id, 'Artist' AS item_type
        FROM artists
        WHERE pinned = TRUE

        UNION ALL

        SELECT id, name AS name, image_id, 'Playlist' AS item_type
        FROM playlists
        WHERE pinned = TRUE
        ORDER BY name COLLATE NOCASE
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.id, r.name, r.image_id, r.item_type))
        .collect())
}
