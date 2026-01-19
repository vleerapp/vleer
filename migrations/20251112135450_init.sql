CREATE TABLE IF NOT EXISTS artists (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    image TEXT,
    favorite BOOLEAN DEFAULT FALSE,
    pinned BOOLEAN DEFAULT FALSE
);
CREATE TABLE IF NOT EXISTS albums (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    artist TEXT,
    cover TEXT,
    favorite BOOLEAN DEFAULT FALSE,
    pinned BOOLEAN DEFAULT FALSE,
    FOREIGN KEY (artist) REFERENCES artists(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS songs (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    artist_id TEXT,
    album_id TEXT,
    file_path TEXT NOT NULL,
    genre TEXT,
    date TEXT,
    duration INTEGER NOT NULL,
    cover TEXT,
    track_number INTEGER,
    favorite BOOLEAN DEFAULT FALSE,
    track_lufs REAL,
    pinned BOOLEAN DEFAULT FALSE,
    date_added TEXT DEFAULT (DATETIME('now')),
    date_updated TEXT DEFAULT (DATETIME('now')),
    FOREIGN KEY (artist_id) REFERENCES artists(id) ON DELETE CASCADE,
    FOREIGN KEY (album_id) REFERENCES albums(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS playlists (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    image TEXT,
    pinned BOOLEAN DEFAULT FALSE,
    date_updated TEXT DEFAULT (DATETIME('now')),
    date_created TEXT DEFAULT (DATETIME('now'))
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
CREATE INDEX IF NOT EXISTS idx_songs_artist ON songs(artist_id);
CREATE INDEX IF NOT EXISTS idx_songs_album ON songs(album_id);
CREATE INDEX IF NOT EXISTS idx_songs_file_path ON songs(file_path);
CREATE INDEX IF NOT EXISTS idx_albums_artist ON albums(artist);
CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_context ON events(context_id);
CREATE INDEX IF NOT EXISTS idx_event_contexts_song ON event_contexts(song_id);
CREATE INDEX IF NOT EXISTS idx_event_contexts_playlist ON event_contexts(playlist_id);
CREATE INDEX IF NOT EXISTS idx_songs_favorite ON songs(favorite);
CREATE INDEX IF NOT EXISTS idx_albums_favorite ON albums(favorite);
CREATE INDEX IF NOT EXISTS idx_artists_favorite ON artists(favorite);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_playlist ON playlist_tracks(playlist_id);
CREATE INDEX IF NOT EXISTS idx_playlist_tracks_song ON playlist_tracks(song_id);