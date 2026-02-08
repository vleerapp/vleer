CREATE TABLE IF NOT EXISTS songs (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    artist_id TEXT,
    album_id TEXT,
    file_path TEXT NOT NULL,
    file_size INTEGER NOT NULL DEFAULT 0,
    file_modified INTEGER NOT NULL DEFAULT 0,
    genre TEXT,
    date TEXT,
    duration INTEGER NOT NULL,
    image_id TEXT,
    track_number INTEGER,
    favorite BOOLEAN DEFAULT FALSE,
    lufs REAL,
    pinned BOOLEAN DEFAULT FALSE,
    date_added TEXT DEFAULT (DATETIME('now')),
    date_updated TEXT DEFAULT (DATETIME('now')),
    FOREIGN KEY (artist_id) REFERENCES artists(id) ON DELETE CASCADE,
    FOREIGN KEY (album_id) REFERENCES albums(id) ON DELETE CASCADE
    FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS artists (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    image_id TEXT,
    favorite BOOLEAN DEFAULT FALSE,
    pinned BOOLEAN DEFAULT FALSE,
    FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS albums (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    artist_id TEXT,
    image_id TEXT,
    favorite BOOLEAN DEFAULT FALSE,
    pinned BOOLEAN DEFAULT FALSE,
    FOREIGN KEY (artist_id) REFERENCES artists(id) ON DELETE CASCADE,
    FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS playlists (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    image_id TEXT,
    pinned BOOLEAN DEFAULT FALSE,
    date_updated TEXT DEFAULT (DATETIME('now')),
    date_created TEXT DEFAULT (DATETIME('now')),
    FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS playlist_tracks (
    id TEXT PRIMARY KEY,
    playlist_id TEXT NOT NULL,
    song_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    date_added TEXT DEFAULT (DATETIME('now')),
    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
    FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    event_type TEXT CHECK(
        event_type IN ('PLAY', 'STOP', 'PAUSE', 'RESUME')
    ) NOT NULL,
    context_id TEXT,
    date_created TEXT DEFAULT (DATETIME('now')),
    timestamp TEXT DEFAULT (DATETIME('now')),
    FOREIGN KEY (context_id) REFERENCES event_contexts(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS event_contexts (
    id TEXT PRIMARY KEY,
    song_id TEXT,
    playlist_id TEXT,
    date_created TEXT DEFAULT (DATETIME('now')),
    FOREIGN KEY (song_id) REFERENCES songs(id) ON DELETE CASCADE,
    FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS images (
    id TEXT PRIMARY KEY NOT NULL,
    data BLOB NOT NULL,
    date_created TEXT NOT NULL DEFAULT (DATETIME('now')),
    date_updated TEXT NOT NULL DEFAULT (DATETIME('now'))
);
CREATE INDEX IF NOT EXISTS idx_songs_artist ON songs(artist_id);
CREATE INDEX IF NOT EXISTS idx_songs_album ON songs(album_id);
CREATE INDEX IF NOT EXISTS idx_songs_file_path ON songs(file_path);
CREATE INDEX IF NOT EXISTS idx_albums_artist ON albums(artist_id);
CREATE INDEX IF NOT EXISTS idx_events_type ON EVENTS(event_type);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_context ON events(context_id);
CREATE INDEX IF NOT EXISTS idx_event_contexts_song ON event_contexts(song_id);
CREATE INDEX IF NOT EXISTS idx_event_contexts_playlist ON event_contexts(playlist_id);
CREATE INDEX IF NOT EXISTS idx_songs_favorite ON songs(favorite);
CREATE INDEX IF NOT EXISTS idx_albums_favorite ON albums(favorite);
CREATE INDEX IF NOT EXISTS idx_artists_favorite ON artists(favorite);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist ON playlist_tracks(playlist_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_song ON playlist_tracks(song_id);
CREATE INDEX IF NOT EXISTS idx_images_date_created ON images(date_created);
CREATE INDEX IF NOT EXISTS idx_songs_image_id ON songs(image_id);
CREATE INDEX IF NOT EXISTS idx_albums_image_id ON albums(image_id);
CREATE INDEX IF NOT EXISTS idx_artists_image_id ON artists(image_id);
CREATE INDEX IF NOT EXISTS idx_playlists_image_id ON playlists(image_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_albums_title_artist ON albums(title, artist_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_songs_file_path_unique ON songs(file_path);
CREATE UNIQUE INDEX IF NOT EXISTS idx_playlist_tracks_unique ON playlist_tracks(playlist_id, song_id);
CREATE TRIGGER IF NOT EXISTS delete_album_trigger
AFTER DELETE ON songs BEGIN
DELETE FROM albums
WHERE albums.id = OLD.album_id
    AND NOT EXISTS (
        SELECT 1
        FROM songs
        WHERE songs.album_id = OLD.album_id
    );
END;
CREATE TRIGGER IF NOT EXISTS delete_artist_trigger
AFTER DELETE ON albums BEGIN
DELETE FROM artists
WHERE artists.id = OLD.artist_id
    AND NOT EXISTS (
        SELECT 1
        FROM albums
        WHERE albums.artist_id = OLD.artist_id
    );
END;
CREATE TRIGGER IF NOT EXISTS delete_unused_artist_image
AFTER DELETE ON artists BEGIN
DELETE FROM images
WHERE id = OLD.image_id
    AND NOT EXISTS (
        SELECT 1
        FROM artists
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM albums
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM songs
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM playlists
        WHERE image_id = OLD.image_id
    );
END;
CREATE TRIGGER IF NOT EXISTS delete_unused_album_image
AFTER DELETE ON albums BEGIN
DELETE FROM images
WHERE id = OLD.image_id
    AND NOT EXISTS (
        SELECT 1
        FROM artists
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM albums
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM songs
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM playlists
        WHERE image_id = OLD.image_id
    );
END;
CREATE TRIGGER IF NOT EXISTS delete_unused_song_image
AFTER DELETE ON songs BEGIN
DELETE FROM images
WHERE id = OLD.image_id
    AND NOT EXISTS (
        SELECT 1
        FROM artists
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM albums
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM songs
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM playlists
        WHERE image_id = OLD.image_id
    );
END;
CREATE TRIGGER IF NOT EXISTS delete_unused_playlist_image
AFTER DELETE ON playlists BEGIN
DELETE FROM images
WHERE id = OLD.image_id
    AND NOT EXISTS (
        SELECT 1
        FROM artists
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM albums
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM songs
        WHERE image_id = OLD.image_id
    )
    AND NOT EXISTS (
        SELECT 1
        FROM playlists
        WHERE image_id = OLD.image_id
    );
END;
