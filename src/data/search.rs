use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::data::db::models::SearchResultRow;
use crate::data::models::Cuid;

#[derive(Clone)]
pub struct SongSearchEntry {
    pub id: Cuid,
    pub title: String,
    pub image_id: Option<String>,
    pub haystack: String,
}

impl SongSearchEntry {
    pub fn new(
        id: Cuid,
        title: String,
        artist: String,
        album: String,
        image_id: Option<String>,
    ) -> Self {
        let haystack = match (artist.is_empty(), album.is_empty()) {
            (true, true) => title.clone(),
            (false, true) => format!("{title} {artist}"),
            _ => format!("{title} {artist} {album}"),
        };
        Self {
            id,
            title,
            image_id,
            haystack,
        }
    }
}

#[derive(Clone)]
pub struct ArtistSearchEntry {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
}

#[derive(Clone)]
pub struct AlbumSearchEntry {
    pub id: Cuid,
    pub title: String,
    pub image_id: Option<String>,
    pub haystack: String,
}

impl AlbumSearchEntry {
    pub fn new(id: Cuid, title: String, artist: String, image_id: Option<String>) -> Self {
        let haystack = if artist.is_empty() {
            title.clone()
        } else {
            format!("{title} {artist}")
        };
        Self {
            id,
            title,
            image_id,
            haystack,
        }
    }
}

#[derive(Clone)]
pub struct PlaylistSearchEntry {
    pub id: Cuid,
    pub name: String,
    pub image_id: Option<String>,
}

#[derive(Default)]
pub struct SearchIndex {
    pub songs: Vec<SongSearchEntry>,
    pub artists: Vec<ArtistSearchEntry>,
    pub albums: Vec<AlbumSearchEntry>,
    pub playlists: Vec<PlaylistSearchEntry>,
}

fn make_matcher() -> Matcher {
    let mut config = Config::DEFAULT;
    config.prefer_prefix = true;
    Matcher::new(config)
}

fn score_entry(
    pattern: &Pattern,
    haystack: &str,
    matcher: &mut Matcher,
    buf: &mut Vec<char>,
) -> Option<u32> {
    let h = Utf32Str::new(haystack, buf);
    pattern.score(h, matcher)
}

fn fuzzy_ids<'a>(
    query: &str,
    entries: impl Iterator<Item = (&'a str, &'a Cuid)>,
) -> Vec<(u32, Cuid)> {
    if query.is_empty() {
        return Vec::new();
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = make_matcher();
    let mut buf: Vec<char> = Vec::new();
    let mut results: Vec<(u32, Cuid)> = entries
        .filter_map(|(haystack, id)| {
            score_entry(&pattern, haystack, &mut matcher, &mut buf).map(|s| (s, id.clone()))
        })
        .collect();
    results.sort_unstable_by_key(|b| std::cmp::Reverse(b.0));
    results
}

impl SearchIndex {
    pub fn fuzzy_song_ids(&self, query: &str) -> Vec<(u32, Cuid)> {
        fuzzy_ids(
            query,
            self.songs.iter().map(|e| (e.haystack.as_str(), &e.id)),
        )
    }

    pub fn fuzzy_artist_ids(&self, query: &str) -> Vec<(u32, Cuid)> {
        fuzzy_ids(query, self.artists.iter().map(|e| (e.name.as_str(), &e.id)))
    }

    pub fn fuzzy_album_ids(&self, query: &str) -> Vec<(u32, Cuid)> {
        fuzzy_ids(
            query,
            self.albums.iter().map(|e| (e.haystack.as_str(), &e.id)),
        )
    }

    pub fn fuzzy_playlist_ids(&self, query: &str) -> Vec<(u32, Cuid)> {
        fuzzy_ids(
            query,
            self.playlists.iter().map(|e| (e.name.as_str(), &e.id)),
        )
    }

    pub fn upsert_song(
        &mut self,
        id: Cuid,
        title: String,
        artist: String,
        album: String,
        image_id: Option<String>,
    ) {
        let entry = SongSearchEntry::new(id.clone(), title, artist, album, image_id);
        self.songs.retain(|e| e.id != id);
        self.songs.push(entry);
    }

    pub fn upsert_artist(&mut self, id: Cuid, name: String) {
        if let Some(existing) = self.artists.iter_mut().find(|e| e.id == id) {
            existing.name = name;
        } else {
            self.artists.push(ArtistSearchEntry {
                id,
                name,
                image_id: None,
            });
        }
    }

    pub fn upsert_album(
        &mut self,
        id: Cuid,
        title: String,
        artist: String,
        image_id: Option<String>,
    ) {
        let entry = AlbumSearchEntry::new(id.clone(), title, artist, image_id);
        self.albums.retain(|e| e.id != id);
        self.albums.push(entry);
    }

    pub fn upsert_playlist(&mut self, id: Cuid, name: String, image_id: Option<String>) {
        self.playlists.retain(|e| e.id != id);
        self.playlists
            .push(PlaylistSearchEntry { id, name, image_id });
    }

    pub fn remove_playlist(&mut self, id: &Cuid) {
        self.playlists.retain(|e| &e.id != id);
    }

    pub fn fuzzy_search_all(&self, query: &str, limit: usize) -> Vec<SearchResultRow> {
        if query.is_empty() {
            return Vec::new();
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut matcher = make_matcher();
        let mut buf = Vec::new();
        let mut results: Vec<(u32, SearchResultRow)> = Vec::new();

        for e in &self.songs {
            if let Some(score) = score_entry(&pattern, &e.haystack, &mut matcher, &mut buf) {
                results.push((
                    score,
                    SearchResultRow {
                        id: e.id.clone(),
                        name: e.title.clone(),
                        image: e.image_id.clone(),
                        item_type: "Song".to_string(),
                    },
                ));
            }
        }
        for e in &self.artists {
            if let Some(score) = score_entry(&pattern, &e.name, &mut matcher, &mut buf) {
                results.push((
                    score,
                    SearchResultRow {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        image: e.image_id.clone(),
                        item_type: "Artist".to_string(),
                    },
                ));
            }
        }
        for e in &self.albums {
            if let Some(score) = score_entry(&pattern, &e.haystack, &mut matcher, &mut buf) {
                results.push((
                    score,
                    SearchResultRow {
                        id: e.id.clone(),
                        name: e.title.clone(),
                        image: e.image_id.clone(),
                        item_type: "Album".to_string(),
                    },
                ));
            }
        }
        for e in &self.playlists {
            if let Some(score) = score_entry(&pattern, &e.name, &mut matcher, &mut buf) {
                results.push((
                    score,
                    SearchResultRow {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        image: e.image_id.clone(),
                        item_type: "Playlist".to_string(),
                    },
                ));
            }
        }

        results.sort_unstable_by_key(|b| std::cmp::Reverse(b.0));
        results.truncate(limit);
        results.into_iter().map(|(_, r)| r).collect()
    }
}
