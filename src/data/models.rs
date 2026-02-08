use crate::data::db::models::{AlbumRow, ArtistRow, PlaylistRow, PlaylistTrackRow, SongRow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct Cuid(String);

impl Cuid {
    pub fn new() -> Self {
        Cuid(cuid2::create_id())
    }
}

impl Default for Cuid {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Cuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub id: Cuid,
    pub title: String,
    pub artist_id: Option<Cuid>,
    pub album_id: Option<Cuid>,
    pub file_path: String,
    pub file_size: i64,
    pub file_modified: i64,
    pub genre: Option<String>,
    pub date: Option<String>,
    pub duration: i32,
    pub image_id: Option<String>,
    pub track_number: Option<i32>,
    pub favorite: bool,
    pub lufs: Option<f32>,
    pub pinned: bool,
    pub date_added: String,
    pub date_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: Cuid,
    pub title: String,
    pub artist_id: Option<Cuid>,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: Cuid,
    pub name: String,
    pub description: Option<String>,
    pub image_id: Option<String>,
    pub pinned: bool,
    pub date_updated: String,
    pub date_created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistTrack {
    pub id: Cuid,
    pub playlist_id: Cuid,
    pub song: Song,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Cuid,
    pub event_type: EventType,
    pub context_id: Option<Cuid>,
    pub date_created: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventContext {
    pub id: Cuid,
    pub song_id: Option<Cuid>,
    pub playlist_id: Option<Cuid>,
    pub date_created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    Play,
    Stop,
    Pause,
    Resume,
}

#[derive(Debug, Clone)]
pub enum RecentItem {
    Song {
        title: String,
        artist_name: Option<String>,
        image_id: Option<String>,
    },
    Album {
        title: String,
        artist_name: Option<String>,
        year: Option<String>,
        image_id: Option<String>,
    },
}

pub struct PinnedItem {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub item_type: String,
}

impl From<SongRow> for Song {
    fn from(row: SongRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            artist_id: row.artist_id,
            album_id: row.album_id,
            file_path: row.file_path,
            file_size: row.file_size,
            file_modified: row.file_modified,
            genre: row.genre,
            date: row.date,
            duration: row.duration,
            image_id: row.image_id,
            track_number: row.track_number,
            favorite: row.favorite,
            lufs: row.lufs,
            pinned: row.pinned,
            date_added: row.date_added,
            date_updated: row.date_updated,
        }
    }
}

impl From<ArtistRow> for Artist {
    fn from(row: ArtistRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            image_id: row.image_id,
            favorite: row.favorite,
            pinned: row.pinned,
        }
    }
}

impl From<AlbumRow> for Album {
    fn from(row: AlbumRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            artist_id: row.artist_id,
            image_id: row.image_id,
            favorite: row.favorite,
            pinned: row.pinned,
        }
    }
}

impl From<PlaylistRow> for Playlist {
    fn from(row: PlaylistRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            image_id: row.image_id,
            pinned: row.pinned,
            date_updated: row.date_updated,
            date_created: row.date_created,
        }
    }
}

impl From<PlaylistTrackRow> for PlaylistTrack {
    fn from(row: PlaylistTrackRow) -> Self {
        Self {
            id: row.id,
            playlist_id: row.playlist_id,
            song: row.song.into(),
            position: row.position,
        }
    }
}
