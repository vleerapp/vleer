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

pub async fn delete_songs_by_paths(pool: &SqlitePool, file_paths: &[String]) -> sqlx::Result<u64> {
    if file_paths.is_empty() {
        return Ok(0);
    }

    let placeholders = std::iter::repeat_n("?", file_paths.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("DELETE FROM songs WHERE file_path IN ({placeholders})");

    let mut query = sqlx::query(&sql);
    for path in file_paths {
        query = query.bind(path);
    }

    let result = query.execute(pool).await?;
    Ok(result.rows_affected())
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
    let query = query.trim();
    if query.is_empty() {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM albums")
            .fetch_one(pool)
            .await?;
        return Ok(row.0);
    }

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM albums al
        WHERE
            al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            OR EXISTS (
                SELECT 1
                FROM artists ar
                WHERE
                    ar.id = al.artist_id
                    AND ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
        "#,
    )
    .bind(query)
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
    let query = query.trim();
    if query.is_empty() {
        return sqlx::query_as::<_, AlbumListRow>(
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
            ORDER BY al.title COLLATE NOCASE ASC
            LIMIT ?1 OFFSET ?2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await;
    }

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
            al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            OR EXISTS (
                SELECT 1
                FROM artists ar2
                WHERE
                    ar2.id = al.artist_id
                    AND ar2.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
        GROUP BY al.id, al.title, ar.name, al.image_id
        ORDER BY al.title COLLATE NOCASE ASC
        LIMIT ?2 OFFSET ?3
        "#,
    )
    .bind(query)
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
        ORDER BY al.title COLLATE NOCASE ASC
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
            image_id = COALESCE(excluded.image_id, albums.image_id)
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
    let query = query.trim();
    if query.is_empty() {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM songs")
            .fetch_one(pool)
            .await?;
        return Ok(row.0);
    }

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM songs s
        WHERE
            s.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            OR EXISTS (
                SELECT 1
                FROM artists ar
                WHERE
                    ar.id = s.artist_id
                    AND ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
            OR EXISTS (
                SELECT 1
                FROM albums al
                WHERE
                    al.id = s.album_id
                    AND al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
        "#,
    )
    .bind(query)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

fn song_order_clause(sort: SongSort, ascending: bool) -> &'static str {
    match sort {
        SongSort::Title => {
            if ascending {
                "s.title COLLATE NOCASE ASC, s.id ASC"
            } else {
                "s.title COLLATE NOCASE DESC, s.id ASC"
            }
        }
        SongSort::Album => {
            if ascending {
                "COALESCE(al.title, '') COLLATE NOCASE ASC, s.id ASC"
            } else {
                "COALESCE(al.title, '') COLLATE NOCASE DESC, s.id ASC"
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
    let query = query.trim();
    let order_clause = song_order_clause(sort, ascending);

    if query.is_empty() {
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
            ORDER BY {order_clause}
            LIMIT ?1 OFFSET ?2
            "#
        );

        return sqlx::query_as::<_, SongListRow>(&sql)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await;
    }

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
            s.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            OR EXISTS (
                SELECT 1
                FROM artists ar2
                WHERE
                    ar2.id = s.artist_id
                    AND ar2.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
            OR EXISTS (
                SELECT 1
                FROM albums al2
                WHERE
                    al2.id = s.album_id
                    AND al2.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
        ORDER BY {order_clause}
        LIMIT ?2 OFFSET ?3
        "#
    );

    sqlx::query_as::<_, SongListRow>(&sql)
        .bind(query)
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
    let query = query.trim();
    let order_clause = song_order_clause(sort, ascending);

    if query.is_empty() {
        let sql = format!(
            r#"
            SELECT s.id
            FROM songs s
            LEFT JOIN artists ar ON s.artist_id = ar.id
            LEFT JOIN albums al ON s.album_id = al.id
            ORDER BY {order_clause}
            LIMIT -1 OFFSET ?1
            "#
        );

        let rows: Vec<(Cuid,)> = sqlx::query_as(&sql).bind(offset).fetch_all(pool).await?;
        return Ok(rows.into_iter().map(|(id,)| id).collect());
    }

    let sql = format!(
        r#"
        SELECT s.id
        FROM songs s
        WHERE
            s.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            OR EXISTS (
                SELECT 1
                FROM artists ar
                WHERE
                    ar.id = s.artist_id
                    AND ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
            OR EXISTS (
                SELECT 1
                FROM albums al
                WHERE
                    al.id = s.album_id
                    AND al.title LIKE '%' || ?1 || '%' COLLATE NOCASE
            )
        ORDER BY {order_clause}
        LIMIT -1 OFFSET ?2
        "#
    );

    let rows: Vec<(Cuid,)> = sqlx::query_as(&sql)
        .bind(query)
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
    let query = query.trim();
    if query.is_empty() {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM artists")
            .fetch_one(pool)
            .await?;
        return Ok(row.0);
    }

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM artists ar
        WHERE
            ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
        "#,
    )
    .bind(query)
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
    let query = query.trim();
    if query.is_empty() {
        return sqlx::query_as::<_, ArtistListRow>(
            r#"
            SELECT
                ar.id,
                ar.name,
                ar.image_id
            FROM artists ar
            ORDER BY ar.name COLLATE NOCASE ASC
            LIMIT ?1 OFFSET ?2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await;
    }

    sqlx::query_as::<_, ArtistListRow>(
        r#"
        SELECT
            ar.id,
            ar.name,
            ar.image_id
        FROM artists ar
        WHERE
            ar.name LIKE '%' || ?1 || '%' COLLATE NOCASE
        ORDER BY ar.name COLLATE NOCASE ASC
        LIMIT ?2 OFFSET ?3
        "#,
    )
    .bind(query)
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

pub async fn search_library(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> sqlx::Result<Vec<SearchResultRow>> {
    let query = query.trim();
    let per_type_limit = (limit.saturating_mul(2)).max(20);

    sqlx::query_as::<_, SearchResultRow>(
        r#"
        WITH
        search_params AS (
            SELECT ?1 AS query_text
        ),
        song_matches AS (
            SELECT
                s.id,
                s.title AS name,
                s.image_id AS image,
                'Song' AS item_type,
                CASE
                    WHEN s.title = sp.query_text COLLATE NOCASE THEN 400
                    WHEN s.title LIKE sp.query_text || '%' COLLATE NOCASE THEN 300
                    WHEN EXISTS (
                        SELECT 1
                        FROM artists ar
                        WHERE
                            ar.id = s.artist_id
                            AND ar.name LIKE sp.query_text || '%' COLLATE NOCASE
                    ) THEN 220
                    WHEN EXISTS (
                        SELECT 1
                        FROM albums al
                        WHERE
                            al.id = s.album_id
                            AND al.title LIKE sp.query_text || '%' COLLATE NOCASE
                    ) THEN 200
                    ELSE 100
                END AS score
            FROM songs s
            CROSS JOIN search_params sp
            WHERE
                s.title LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                OR EXISTS (
                    SELECT 1
                    FROM artists ar
                    WHERE
                        ar.id = s.artist_id
                        AND ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                )
                OR EXISTS (
                    SELECT 1
                    FROM albums al
                    WHERE
                        al.id = s.album_id
                        AND al.title LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                )
            ORDER BY score DESC, s.title COLLATE NOCASE ASC
            LIMIT ?2
        ),
        album_matches AS (
            SELECT
                al.id,
                al.title AS name,
                al.image_id AS image,
                'Album' AS item_type,
                CASE
                    WHEN al.title = sp.query_text COLLATE NOCASE THEN 350
                    WHEN al.title LIKE sp.query_text || '%' COLLATE NOCASE THEN 260
                    WHEN EXISTS (
                        SELECT 1
                        FROM artists ar
                        WHERE
                            ar.id = al.artist_id
                            AND ar.name LIKE sp.query_text || '%' COLLATE NOCASE
                    ) THEN 180
                    ELSE 90
                END AS score
            FROM albums al
            CROSS JOIN search_params sp
            WHERE
                al.title LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                OR EXISTS (
                    SELECT 1
                    FROM artists ar
                    WHERE
                        ar.id = al.artist_id
                        AND ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
                )
            ORDER BY score DESC, al.title COLLATE NOCASE ASC
            LIMIT ?2
        ),
        artist_matches AS (
            SELECT
                ar.id,
                ar.name AS name,
                ar.image_id AS image,
                'Artist' AS item_type,
                CASE
                    WHEN ar.name = sp.query_text COLLATE NOCASE THEN 320
                    WHEN ar.name LIKE sp.query_text || '%' COLLATE NOCASE THEN 250
                    ELSE 80
                END AS score
            FROM artists ar
            CROSS JOIN search_params sp
            WHERE
                ar.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
            ORDER BY score DESC, ar.name COLLATE NOCASE ASC
            LIMIT ?2
        ),
        playlist_matches AS (
            SELECT
                p.id,
                p.name AS name,
                p.image_id AS image,
                'Playlist' AS item_type,
                CASE
                    WHEN p.name = sp.query_text COLLATE NOCASE THEN 300
                    WHEN p.name LIKE sp.query_text || '%' COLLATE NOCASE THEN 240
                    ELSE 70
                END AS score
            FROM playlists p
            CROSS JOIN search_params sp
            WHERE
                p.name LIKE '%' || sp.query_text || '%' COLLATE NOCASE
            ORDER BY score DESC, p.name COLLATE NOCASE ASC
            LIMIT ?2
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
        ORDER BY 
            score DESC,
            name COLLATE NOCASE ASC
        LIMIT ?3
        "#,
    )
    .bind(query)
    .bind(per_type_limit)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_playlists_count_filtered(pool: &SqlitePool, query: &str) -> sqlx::Result<i64> {
    let query = query.trim();
    if query.is_empty() {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM playlists")
            .fetch_one(pool)
            .await?;
        return Ok(row.0);
    }

    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM playlists p
        WHERE
            p.name LIKE '%' || ?1 || '%' COLLATE NOCASE
        "#,
    )
    .bind(query)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
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
