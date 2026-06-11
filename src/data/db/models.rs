use crate::data::models::{Album, Artist, Cuid, Event, EventType, Playlist, RecentItem, Song};
use rusqlite::Row;

#[derive(Debug, Clone)]
pub struct ImageRow {
    pub id: Cuid,
    pub data: Vec<u8>,
    pub date_created: String,
    pub date_updated: String,
}

impl ImageRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            data: row.get("data")?,
            date_created: row.get("date_created")?,
            date_updated: row.get("date_updated")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SongRow {
    pub id: Cuid,
    pub title: String,
    pub artist_id: Option<Cuid>,
    pub artist_name: Option<String>,
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

impl SongRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(SongRow {
            id: row.get("id")?,
            title: row.get("title")?,
            artist_id: row.get("artist_id")?,
            artist_name: row.get("artist_name")?,
            album_id: row.get("album_id")?,
            file_path: row.get("file_path")?,
            file_size: row.get("file_size")?,
            file_modified: row.get("file_modified")?,
            genre: row.get("genre")?,
            date: row.get("date")?,
            duration: row.get("duration")?,
            image_id: row.get("image_id")?,
            track_number: row.get("track_number")?,
            favorite: row.get("favorite")?,
            lufs: row.get("lufs")?,
            pinned: row.get("pinned")?,
            date_added: row.get("date_added")?,
            date_updated: row.get("date_updated")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SongListRow {
    pub id: Cuid,
    pub title: String,
    pub artist_name: Option<String>,
    pub album_title: Option<String>,
    pub album_id: Option<Cuid>,
    pub duration: i32,
    pub image_id: Option<String>,
}

impl SongListRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artist_name: row.get("artist_name")?,
            album_title: row.get("album_title")?,
            album_id: row.get("album_id")?,
            duration: row.get("duration")?,
            image_id: row.get("image_id")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ArtistRow {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

impl ArtistRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
            favorite: row.get("favorite")?,
            pinned: row.get("pinned")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ArtistListRow {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
}

impl ArtistListRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AlbumRow {
    pub id: Cuid,
    pub title: String,
    pub artist_id: Option<Cuid>,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

impl AlbumRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artist_id: row.get("artist_id")?,
            image_id: row.get("image_id")?,
            favorite: row.get("favorite")?,
            pinned: row.get("pinned")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistListRow {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub song_count: i64,
}

impl PlaylistListRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
            song_count: row.get("song_count")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AlbumListRow {
    pub id: Cuid,
    pub title: String,
    pub artist_name: Option<String>,
    pub image_id: Option<String>,
    pub year: Option<String>,
}

impl AlbumListRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artist_name: row.get("artist_name")?,
            image_id: row.get("image_id")?,
            year: row.get("year")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistRow {
    pub id: Cuid,
    pub name: String,
    pub description: Option<String>,
    pub image_id: Option<String>,
    pub pinned: bool,
    pub date_updated: String,
    pub date_created: String,
}

impl PlaylistRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            image_id: row.get("image_id")?,
            pinned: row.get("pinned")?,
            date_updated: row.get("date_updated")?,
            date_created: row.get("date_created")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlaylistTrackRow {
    pub id: Cuid,
    pub playlist_id: Cuid,
    pub position: i32,
    pub song: SongRow,
    pub album_title: Option<String>,
}

impl PlaylistTrackRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("pt_id")?,
            playlist_id: row.get("playlist_id")?,
            position: row.get("position")?,
            song: SongRow::from_row(row)?,
            album_title: row.get("album_title")?,
        })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: Cuid,
    pub event_type: String,
    pub context_id: Option<Cuid>,
    pub timestamp: String,
}

#[allow(dead_code)]
impl EventRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            event_type: row.get("event_type")?,
            context_id: row.get("context_id")?,
            timestamp: row.get("timestamp")?,
        })
    }

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
            timestamp: self.timestamp,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EventContextRow {
    pub id: Cuid,
    pub song_id: Option<Cuid>,
    pub playlist_id: Option<Cuid>,
    pub date_created: String,
}

#[allow(dead_code)]
impl EventContextRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            song_id: row.get("song_id")?,
            playlist_id: row.get("playlist_id")?,
            date_created: row.get("date_created")?,
        })
    }
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

#[derive(Debug, Clone)]
pub struct SearchResultRow {
    pub id: Cuid,
    pub name: String,
    pub image: Option<String>,
    pub item_type: String,
}

impl SearchResultRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image: row.get("image")?,
            item_type: row.get("item_type")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecentItemRow {
    pub song_count: i64,
    pub first_song_id: Cuid,
    pub first_song_title: String,
    pub image_id: Option<String>,
    pub first_year: Option<String>,
    pub album_id: Option<Cuid>,
    pub album_title: Option<String>,
    pub artist_name: Option<String>,
}

impl RecentItemRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            song_count: row.get("song_count")?,
            first_song_id: row.get("first_song_id")?,
            first_song_title: row.get("first_song_title")?,
            image_id: row.get("image_id")?,
            first_year: row.get("first_year")?,
            album_id: row.get("album_id")?,
            album_title: row.get("album_title")?,
            artist_name: row.get("artist_name")?,
        })
    }

    pub fn into_recent_item(self) -> RecentItem {
        if let Some(album_id) = self.album_id
            && self.song_count > 1
        {
            RecentItem::Album {
                id: album_id,
                title: self
                    .album_title
                    .unwrap_or_else(|| "Unknown Album".to_string()),
                artist_name: self.artist_name,
                year: self.first_year,
                image_id: self.image_id,
            }
        } else {
            RecentItem::Song {
                id: self.first_song_id,
                title: self.first_song_title,
                artist_name: self.artist_name,
                image_id: self.image_id,
            }
        }
    }
}

pub struct PinnedItemRow {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub item_type: String,
}

impl PinnedItemRow {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
            item_type: row.get("item_type")?,
        })
    }
}
