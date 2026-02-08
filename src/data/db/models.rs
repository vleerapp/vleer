use crate::data::models::{Album, Artist, Cuid, Event, EventType, Playlist, RecentItem, Song};
use sqlx::{FromRow, Row, sqlite::SqliteRow};

#[derive(Debug, Clone, FromRow)]
pub struct ImageRow {
    pub id: String,
    pub data: Vec<u8>,
    pub date_created: String,
    pub date_updated: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SongRow {
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

#[derive(Debug, Clone, FromRow)]
pub struct ArtistRow {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct AlbumRow {
    pub id: Cuid,
    pub title: String,
    pub artist_id: Option<Cuid>,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct PlaylistRow {
    pub id: Cuid,
    pub name: String,
    pub description: Option<String>,
    pub image_id: Option<String>,
    pub pinned: bool,
    pub date_updated: String,
    pub date_created: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct PlaylistTrackRow {
    pub id: Cuid,
    pub playlist_id: Cuid,
    pub position: i32,
    pub song: SongRow,
}

impl PlaylistTrackRow {
    pub fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            playlist_id: row.try_get("playlist_id")?,
            position: row.try_get("position")?,
            song: SongRow {
                id: row.try_get("id")?,
                title: row.try_get("title")?,
                artist_id: row.try_get("artist_id")?,
                album_id: row.try_get("album_id")?,
                file_path: row.try_get("file_path")?,
                file_size: row.try_get("file_size")?,
                file_modified: row.try_get("file_modified")?,
                genre: row.try_get("genre")?,
                date: row.try_get("date")?,
                duration: row.try_get("duration")?,
                image_id: row.try_get("image_id")?,
                track_number: row.try_get("track_number")?,
                favorite: row.try_get("favorite")?,
                lufs: row.try_get("lufs")?,
                pinned: row.try_get("pinned")?,
                date_added: row.try_get("date_added")?,
                date_updated: row.try_get("date_updated")?,
            },
        })
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct EventRow {
    pub id: Cuid,
    pub event_type: String,
    pub context_id: Option<Cuid>,
    pub date_created: String,
    pub timestamp: String,
}

impl EventRow {
    pub fn into_event(self) -> Event {
        Event {
            id: self.id,
            event_type: match self.event_type.as_str() {
                "PLAY" => EventType::Play,
                "STOP" => EventType::Stop,
                "PAUSE" => EventType::Pause,
                "RESUME" => EventType::Resume,
                _ => panic!("Unknown event type"),
            },
            context_id: self.context_id,
            date_created: self.date_created,
            timestamp: self.timestamp,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct EventContextRow {
    pub id: Cuid,
    pub song_id: Option<Cuid>,
    pub playlist_id: Option<Cuid>,
    pub date_created: String,
}

pub trait Toggleable {
    const TABLE: &'static str;
    const ID_COL: &'static str = "id";
}

impl Toggleable for Song {
    const TABLE: &'static str = "songs";
}
impl Toggleable for Album {
    const TABLE: &'static str = "albums";
}
impl Toggleable for Artist {
    const TABLE: &'static str = "artists";
}
impl Toggleable for Playlist {
    const TABLE: &'static str = "playlists";
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchResultRow {
    pub id: Cuid,
    pub name: String,
    pub image: Option<String>,
    pub item_type: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchCountsRow {
    pub song_count: i64,
    pub album_count: i64,
    pub artist_count: i64,
    pub playlist_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct RecentItemRow {
    pub most_recent_date: String,
    pub song_count: i64,
    pub first_song_id: Cuid,
    pub first_song_title: String,
    pub first_artist_id: Option<Cuid>,
    pub image_id: Option<String>,
    pub first_year: Option<String>,
    pub album_id: Option<Cuid>,
    pub album_title: Option<String>,
    pub artist_name: Option<String>,
}

impl RecentItemRow {
    pub fn into_recent_item(self) -> RecentItem {
        if self.song_count > 1 && self.album_id.is_some() {
            RecentItem::Album {
                title: self
                    .album_title
                    .unwrap_or_else(|| "Unknown Album".to_string()),
                artist_name: self.artist_name,
                year: self.first_year,
                image_id: self.image_id,
            }
        } else {
            RecentItem::Song {
                title: self.first_song_title,
                artist_name: self.artist_name,
                image_id: self.image_id,
            }
        }
    }
}

#[derive(sqlx::FromRow)]
pub struct PinnedItemRow {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub item_type: String,
}
