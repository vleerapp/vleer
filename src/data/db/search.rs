use super::Database;
use crate::data::{
    models::{Cuid, SearchResultRow},
    search::{
        AlbumSearchEntry, ArtistSearchEntry, PlaylistSearchEntry, SearchIndex, SongSearchEntry,
    },
};
use anyhow::Result;

impl Database {
    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchResultRow>> {
        let query = query.trim();
        if query.is_empty() || limit <= 0 {
            return Ok(Vec::new());
        }
        let index = self.search_index.lock();
        Ok(index.fuzzy_search_all(query, limit as usize))
    }

    pub fn count_search_matches(&self, query: &str) -> Result<(usize, usize, usize, usize, usize)> {
        let query = query.trim();
        if query.is_empty() {
            return Ok((0, 0, 0, 0, 0));
        }
        let genres = self.get_genres(query)?.len();
        let index = self.search_index.lock();
        Ok((
            index.fuzzy_song_ids(query).len(),
            index.fuzzy_album_ids(query).len(),
            index.fuzzy_artist_ids(query).len(),
            genres,
            index.fuzzy_playlist_ids(query).len(),
        ))
    }

    pub fn rebuild_search_index(&self) {
        match self.load_search_index() {
            Ok(index) => *self.search_index.lock() = index,
            Err(e) => tracing::error!("rebuild_search_index failed: {e}"),
        }
    }

    pub(super) fn load_search_index(&self) -> Result<SearchIndex> {
        let conn = self.index_conn.lock();

        let songs = {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title,
                        COALESCE((SELECT GROUP_CONCAT(name, ', ')
                                  FROM (SELECT ar.name FROM songs_artists sa
                                        JOIN artists ar ON sa.artist_id = ar.id
                                        WHERE sa.song_id = s.id ORDER BY sa.position)), '') AS artist,
                        COALESCE(al.title, '') AS album,
                        s.image_id
                 FROM songs s
                 LEFT JOIN albums al ON s.album_id = al.id",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Cuid>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(id, title, artist, album, image_id)| {
                SongSearchEntry::new(id, title, artist, album, image_id)
            })
            .collect()
        };

        let artists = {
            let mut stmt = conn.prepare("SELECT id, name, image_id FROM artists")?;
            stmt.query_map([], |row| {
                Ok(ArtistSearchEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    image_id: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        let albums = {
            let mut stmt = conn.prepare(
                "SELECT al.id, al.title,
                        COALESCE((SELECT GROUP_CONCAT(name, ', ')
                                  FROM (SELECT ar.name FROM albums_artists aa
                                        JOIN artists ar ON aa.artist_id = ar.id
                                        WHERE aa.album_id = al.id ORDER BY aa.position)), '') AS artist,
                        al.image_id
                 FROM albums al",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, Cuid>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(id, title, artist, image_id)| AlbumSearchEntry::new(id, title, artist, image_id))
            .collect()
        };

        let playlists = {
            let mut stmt = conn.prepare("SELECT id, name, image_id FROM playlists")?;
            stmt.query_map([], |row| {
                Ok(PlaylistSearchEntry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    image_id: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        Ok(SearchIndex {
            songs,
            artists,
            albums,
            playlists,
        })
    }
}
