use crate::data::db::models::Toggleable;
use crate::data::db::queries;
use crate::data::models::{
    Album, AlbumListItem, Artist, ArtistListItem, Cuid, Event, EventContext, EventType, Image,
    PinnedItem, Playlist, PlaylistTrack, RecentItem, Song, SongListItem, SongSort,
};
use gpui::Global;
use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Database {
    pub pool: Arc<SqlitePool>,
}

impl Global for Database {}

impl Database {
    pub async fn get_song(&self, id: Cuid) -> sqlx::Result<Option<Song>> {
        Ok(queries::get_song(&self.pool, id)
            .await?
            .map(|row| row.into()))
    }

    pub async fn get_song_by_path(&self, file_path: &str) -> sqlx::Result<Option<Song>> {
        Ok(queries::get_song_by_path(&self.pool, file_path)
            .await?
            .map(|row| row.into()))
    }

    pub async fn upsert_song(
        &self,
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
        queries::upsert_song(
            &self.pool,
            title,
            artist_id,
            album_id,
            file_path,
            duration,
            track_number,
            year,
            genre,
            image_id,
            file_size,
            file_modified,
            lufs,
        )
        .await
    }

    pub async fn delete_song(&self, id: &Cuid) -> sqlx::Result<()> {
        queries::delete_song(&self.pool, id).await
    }

    pub async fn delete_song_by_path(&self, file_path: &str) -> sqlx::Result<()> {
        queries::delete_song_by_path(&self.pool, file_path).await
    }

    pub async fn get_songs_count(&self) -> sqlx::Result<i64> {
        queries::get_songs_count(&self.pool).await
    }

    pub async fn get_songs_paged(&self, offset: i64, limit: i64) -> sqlx::Result<Vec<Song>> {
        Ok(queries::get_songs_paged(&self.pool, offset, limit)
            .await?
            .into_iter()
            .map(|row| row.into())
            .collect())
    }

    pub async fn get_artist(&self, id: Cuid) -> sqlx::Result<Option<Artist>> {
        Ok(queries::get_artist(&self.pool, id)
            .await?
            .map(|row| row.into()))
    }

    pub async fn get_artists_count_filtered(&self, query: &str) -> sqlx::Result<usize> {
        let count = queries::get_artists_count_filtered(&self.pool, query).await?;
        Ok(count as usize)
    }

    pub async fn get_artists_paged_filtered(
        &self,
        query: &str,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<ArtistListItem>> {
        let rows = queries::get_artists_paged_filtered(&self.pool, query, limit, offset).await?;
        Ok(rows
            .into_iter()
            .map(|row| ArtistListItem {
                id: row.id,
                name: row.name,
                image_id: row.image_id,
            })
            .collect())
    }

    pub async fn upsert_artist(&self, name: &str) -> sqlx::Result<Cuid> {
        queries::upsert_artist(&self.pool, name).await
    }

    pub async fn get_album(&self, id: Cuid) -> sqlx::Result<Option<Album>> {
        Ok(queries::get_album(&self.pool, id)
            .await?
            .map(|row| row.into()))
    }

    pub async fn get_albums_count_filtered(&self, query: &str) -> sqlx::Result<usize> {
        let count = queries::get_albums_count_filtered(&self.pool, query).await?;
        Ok(count as usize)
    }

    pub async fn get_albums_paged_filtered(
        &self,
        query: &str,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<AlbumListItem>> {
        let rows = queries::get_albums_paged_filtered(&self.pool, query, limit, offset).await?;
        Ok(rows
            .into_iter()
            .map(|row| AlbumListItem {
                id: row.id,
                title: row.title,
                artist_name: row.artist_name,
                image_id: row.image_id,
                year: row.year,
            })
            .collect())
    }

    pub async fn get_albums_with_artist(&self) -> sqlx::Result<Vec<AlbumListItem>> {
        let rows = queries::get_albums_with_artist(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| AlbumListItem {
                id: row.id,
                title: row.title,
                artist_name: row.artist_name,
                image_id: row.image_id,
                year: row.year,
            })
            .collect())
    }

    pub async fn upsert_album(
        &self,
        title: &str,
        artist_id: Option<&Cuid>,
        image_id: Option<&str>,
    ) -> sqlx::Result<Cuid> {
        queries::upsert_album(&self.pool, title, artist_id, image_id).await
    }

    pub async fn delete_album(&self, id: &Cuid) -> sqlx::Result<()> {
        queries::delete_album(&self.pool, id).await
    }

    pub async fn get_songs_count_filtered(&self, query: &str) -> sqlx::Result<usize> {
        let count = queries::get_songs_count_filtered(&self.pool, query).await?;
        Ok(count as usize)
    }

    pub async fn get_songs_paged_filtered(
        &self,
        query: &str,
        sort: SongSort,
        ascending: bool,
        offset: i64,
        limit: i64,
    ) -> sqlx::Result<Vec<SongListItem>> {
        let rows =
            queries::get_songs_paged_filtered(&self.pool, query, sort, ascending, limit, offset)
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| SongListItem {
                id: row.id,
                title: row.title,
                artist_name: row.artist_name,
                album_title: row.album_title,
                duration: row.duration,
                image_id: row.image_id,
            })
            .collect())
    }

    pub async fn get_song_ids_from_offset_filtered(
        &self,
        query: &str,
        sort: SongSort,
        ascending: bool,
        offset: i64,
    ) -> sqlx::Result<Vec<Cuid>> {
        queries::get_song_ids_from_offset_filtered(&self.pool, query, sort, ascending, offset).await
    }

    pub async fn get_album_songs(&self, album_id: &Cuid) -> sqlx::Result<Vec<Song>> {
        Ok(queries::get_album_songs(&self.pool, album_id)
            .await?
            .into_iter()
            .map(|row| row.into())
            .collect())
    }

    pub async fn get_playlist(&self, id: &Cuid) -> sqlx::Result<Option<Playlist>> {
        Ok(queries::get_playlist(&self.pool, id)
            .await?
            .map(|row| row.into()))
    }

    pub async fn upsert_playlist(
        &self,
        id: &Cuid,
        name: &str,
        description: Option<&str>,
        image_id: Option<&str>,
        pinned: bool,
    ) -> sqlx::Result<()> {
        queries::upsert_playlist(&self.pool, id, name, description, image_id, pinned).await
    }

    pub async fn delete_playlist(&self, id: &Cuid) -> sqlx::Result<()> {
        queries::delete_playlist(&self.pool, id).await
    }

    pub async fn upsert_playlist_song(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
        position: i32,
    ) -> sqlx::Result<()> {
        queries::upsert_playlist_song(&self.pool, playlist_id, song_id, position).await
    }

    pub async fn delete_playlist_song(
        &self,
        playlist_id: &Cuid,
        song_id: &Cuid,
    ) -> sqlx::Result<()> {
        queries::delete_playlist_song(&self.pool, playlist_id, song_id).await
    }

    pub async fn get_playlist_songs(&self, playlist_id: &Cuid) -> sqlx::Result<Vec<PlaylistTrack>> {
        Ok(queries::get_playlist_songs(&self.pool, playlist_id)
            .await?
            .into_iter()
            .map(|row| row.into())
            .collect())
    }

    pub async fn get_event(&self, id: &Cuid) -> sqlx::Result<Option<Event>> {
        queries::get_event(&self.pool, id).await
    }

    pub async fn insert_event(
        &self,
        event_type: EventType,
        context_id: Option<&Cuid>,
    ) -> sqlx::Result<Cuid> {
        queries::insert_event(&self.pool, event_type, context_id).await
    }

    pub async fn get_events_by_type(&self, event_type: EventType) -> sqlx::Result<Vec<Event>> {
        queries::get_events_by_type(&self.pool, event_type).await
    }

    pub async fn insert_event_context(
        &self,
        song_id: Option<&Cuid>,
        playlist_id: Option<&Cuid>,
    ) -> sqlx::Result<Cuid> {
        queries::insert_event_context(&self.pool, song_id, playlist_id).await
    }

    pub async fn get_event_context(&self, id: &Cuid) -> sqlx::Result<Option<EventContext>> {
        queries::get_event_context(&self.pool, id).await
    }

    pub async fn get_event_context_by_song(
        &self,
        song_id: &Cuid,
    ) -> sqlx::Result<Vec<EventContext>> {
        queries::get_event_context_by_song(&self.pool, song_id).await
    }

    pub async fn get_event_context_by_playlist(
        &self,
        playlist_id: &Cuid,
    ) -> sqlx::Result<Vec<EventContext>> {
        queries::get_event_context_by_playlist(&self.pool, playlist_id).await
    }

    pub async fn set_favorite<T: Toggleable>(&self, id: &Cuid, favorite: bool) -> sqlx::Result<()> {
        queries::set_favorite::<T>(&self.pool, id, favorite).await
    }

    pub async fn set_pinned<T: Toggleable>(&self, id: &Cuid, pinned: bool) -> sqlx::Result<()> {
        queries::set_pinned::<T>(&self.pool, id, pinned).await
    }

    pub async fn search_library(
        &self,
        query: &str,
    ) -> sqlx::Result<Vec<(Cuid, String, Option<String>, String)>> {
        let results = queries::search_library(&self.pool, query).await?;

        Ok(results
            .into_iter()
            .map(|r| (r.id, r.name, r.image, r.item_type))
            .collect())
    }

    pub async fn get_search_match_counts(
        &self,
        query: &str,
    ) -> sqlx::Result<(usize, usize, usize, usize)> {
        let counts = queries::get_search_match_counts(&self.pool, query).await?;

        Ok((
            counts.song_count as usize,
            counts.album_count as usize,
            counts.artist_count as usize,
            counts.playlist_count as usize,
        ))
    }

    pub async fn upsert_image(&self, id: &str, data: &[u8]) -> sqlx::Result<()> {
        queries::upsert_image(&self.pool, id, data).await
    }

    pub async fn get_image(&self, id: &str) -> sqlx::Result<Option<Image>> {
        Ok(queries::get_image(&self.pool, id)
            .await?
            .map(|row| row.into()))
    }

    pub async fn delete_image(&self, id: &str) -> sqlx::Result<()> {
        queries::delete_image(&self.pool, id).await
    }

    pub async fn get_recently_added_items(&self, limit: i64) -> sqlx::Result<Vec<RecentItem>> {
        let rows = queries::get_recently_added_items(&self.pool, limit).await?;
        Ok(rows.into_iter().map(|row| row.into_recent_item()).collect())
    }

    pub async fn get_pinned_items(&self) -> Vec<PinnedItem> {
        queries::get_pinned_items(&self.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(id, name, image_id, item_type)| PinnedItem {
                id,
                name,
                image_id,
                item_type,
            })
            .collect()
    }
}
