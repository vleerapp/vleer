use rusqlite::Row;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cuid(String);

impl Cuid {
    pub fn new() -> Self {
        Cuid(cuid2::create_id())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for Cuid {
    fn from(value: String) -> Self {
        Cuid(value)
    }
}

impl rusqlite::ToSql for Cuid {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(self.0.as_str().into())
    }
}

impl rusqlite::types::FromSql for Cuid {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        String::column_result(value).map(Cuid)
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
pub struct Image {
    pub id: Cuid,
    pub data: Vec<u8>,
    pub date_created: String,
    pub date_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    pub id: Cuid,
    pub title: String,
    pub artists: Vec<String>,
    pub album_id: Option<Cuid>,
    pub file_path: String,
    pub file_size: i64,
    pub file_modified: i64,
    pub genres: Vec<String>,
    pub date: Option<String>,
    pub duration: i32,
    pub image_id: Option<String>,
    pub track_number: Option<i32>,
    pub favorite: bool,
    pub lufs: Option<f32>,
    pub isrc: Option<String>,
    pub pinned: bool,
    pub date_added: String,
    pub date_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongListItem {
    pub id: Cuid,
    pub title: String,
    pub artist_name: Option<String>,
    pub album_title: Option<String>,
    pub album_id: Option<Cuid>,
    pub duration: i32,
    pub image_id: Option<String>,
    pub genres: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SongSort {
    Default,
    Title,
    Album,
    Duration,
    Genre,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: Cuid,
    pub title: String,
    pub artists: Vec<String>,
    pub image_id: Option<String>,
    pub upc: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLyrics {
    pub source: String,
    pub synced: bool,
    pub instrumental: bool,
    pub content: String,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenreListItem {
    pub id: Cuid,
    pub name: String,
    pub song_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumListItem {
    pub id: Cuid,
    pub title: String,
    pub artist_name: Option<String>,
    pub image_id: Option<String>,
    pub year: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistListItem {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistListItem {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub song_count: i64,
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
    pub album_title: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: Cuid,
    pub event_type: EventType,
    pub context_id: Option<Cuid>,
    pub timestamp: String,
}

#[allow(dead_code)]
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
        id: Cuid,
        title: String,
        artist_name: Option<String>,
        image_id: Option<String>,
    },
    Album {
        id: Cuid,
        title: String,
        artist_name: Option<String>,
        year: Option<String>,
        image_id: Option<String>,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub struct PinnedItem {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
    pub item_type: String,
}

impl From<SearchResultRow> for PinnedItem {
    fn from(r: SearchResultRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            image_id: r.image,
            item_type: r.item_type,
        }
    }
}

fn split_concat(s: Option<String>) -> Vec<String> {
    s.map(|v| {
        v.split(',')
            .map(|x| x.to_string())
            .filter(|x| !x.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

impl Image {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            data: row.get("data")?,
            date_created: row.get("date_created")?,
            date_updated: row.get("date_updated")?,
        })
    }
}

impl Song {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artists: split_concat(row.get::<_, Option<String>>("artists")?),
            album_id: row.get("album_id")?,
            file_path: row.get("file_path")?,
            file_size: row.get("file_size")?,
            file_modified: row.get("file_modified")?,
            genres: split_concat(row.get::<_, Option<String>>("genres")?),
            date: row.get("date")?,
            duration: row.get("duration")?,
            image_id: row.get("image_id")?,
            track_number: row.get("track_number")?,
            favorite: row.get("favorite")?,
            lufs: row.get("lufs")?,
            isrc: row.get("isrc")?,
            pinned: row.get("pinned")?,
            date_added: row.get("date_added")?,
            date_updated: row.get("date_updated")?,
        })
    }
}

impl SongListItem {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artist_name: row.get("artist_name")?,
            album_title: row.get("album_title")?,
            album_id: row.get("album_id")?,
            duration: row.get("duration")?,
            image_id: row.get("image_id")?,
            genres: row.get("genres")?,
        })
    }
}

impl Artist {
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

impl ArtistListItem {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
        })
    }
}

impl Album {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artists: split_concat(row.get::<_, Option<String>>("artists")?),
            image_id: row.get("image_id")?,
            upc: row.get("upc")?,
            favorite: row.get("favorite")?,
            pinned: row.get("pinned")?,
        })
    }
}

impl AlbumListItem {
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

impl Playlist {
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

impl PlaylistListItem {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
            song_count: row.get("song_count")?,
        })
    }
}

impl PlaylistTrack {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("pt_id")?,
            playlist_id: row.get("playlist_id")?,
            position: row.get("position")?,
            song: Song::from_row(row)?,
            album_title: row.get("album_title")?,
        })
    }
}

impl Event {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let event_type: String = row.get("event_type")?;
        Ok(Self {
            id: row.get("id")?,
            event_type: match event_type.as_str() {
                "PLAY" => EventType::Play,
                "STOP" => EventType::Stop,
                "PAUSE" => EventType::Pause,
                "RESUME" => EventType::Resume,
                other => {
                    tracing::error!("Unknown event type in DB: {}; defaulting to PLAY", other);
                    EventType::Play
                }
            },
            context_id: row.get("context_id")?,
            timestamp: row.get("timestamp")?,
        })
    }
}

impl EventContext {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            song_id: row.get("song_id")?,
            playlist_id: row.get("playlist_id")?,
            date_created: row.get("date_created")?,
        })
    }
}

impl PinnedItem {
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            image_id: row.get("image_id")?,
            item_type: row.get("item_type")?,
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
