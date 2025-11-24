use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;

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

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Song {
    pub id: Cuid,
    pub title: String,
    pub artist_id: Option<Cuid>,
    pub album_id: Option<Cuid>,
    pub file_path: String,
    pub genre: Option<String>,
    pub date: Option<String>,
    pub date_added: String,
    pub duration: i32,
    pub cover: Option<String>,
    pub track_number: Option<i32>,
    pub favorite: bool,
    pub replaygain_track_gain: Option<f32>,
    pub replaygain_track_peak: Option<f32>,
}

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct Artist {
    pub id: Cuid,
    pub name: String,
    pub image: Option<String>,
    pub favorite: bool,
}

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct Album {
    pub id: Cuid,
    pub title: String,
    pub artist: Option<Cuid>,
    pub cover: Option<String>,
    pub favorite: bool,
}

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct Playlist {
    pub id: Cuid,
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub date_created: String,
    pub date_updated: String,
}

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct Event {
    pub id: Cuid,
    pub event_type: EventType,
    pub context_id: Option<Cuid>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum EventType {
    #[sqlx(rename = "PLAY")]
    Play,
    #[sqlx(rename = "STOP")]
    Stop,
    #[sqlx(rename = "PAUSE")]
    Pause,
    #[sqlx(rename = "RESUME")]
    Resume,
}

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct EventContext {
    pub id: Cuid,
    pub song_id: Option<Cuid>,
    pub playlist_id: Option<Cuid>,
}
