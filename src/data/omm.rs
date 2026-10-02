use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use ureq::Agent;
use ureq::http::Response;

use crate::data::db::{
    AlbumLookup, AlbumMetadata, ArtistMetadata, Database, SongBackfill, SongLookup,
};
use crate::data::images::{prepare_image, write_with_retry};
use crate::data::metadata::{normalize_isrc, normalize_upc};
use crate::data::models::Cuid;

const BASE_URL: &str = "https://api.vleer.app/metadata/v1";
const WARM_BATCH: i64 = 64;
const MAX_NAME_CHARS: usize = 256;
const MAX_LOOKUP_VALUES: usize = 100;
const MAX_LOOKUP_CHARS: usize = 10_000;
const ARTWORK_SIZE: &str = "512";
const REQUEST_INTERVAL: Duration = Duration::from_millis(50);
const RATE_LIMIT_RETRIES: u32 = 5;
const MAX_RETRY_AFTER_SECS: u64 = 60;

type PreparedImage = (String, Option<Vec<u8>>);

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Resource {
    Song(SongResource),
    Album(AlbumResource),
    Artist(ArtistResource),
}

impl Resource {
    fn id(&self) -> &str {
        match self {
            Self::Song(song) => &song.id,
            Self::Album(album) => &album.id,
            Self::Artist(artist) => &artist.id,
        }
    }

    fn artwork_url(&self) -> Option<&str> {
        match self {
            Self::Song(song) => song.attributes.artwork_url.as_deref(),
            Self::Album(album) => album.attributes.artwork_url.as_deref(),
            Self::Artist(artist) => artist.attributes.artwork_url.as_deref(),
        }
    }

    fn into_song(self) -> Option<SongResource> {
        match self {
            Self::Song(song) => Some(song),
            _ => None,
        }
    }

    fn into_album(self) -> Option<AlbumResource> {
        match self {
            Self::Album(album) => Some(album),
            _ => None,
        }
    }

    fn into_artist(self) -> Option<ArtistResource> {
        match self {
            Self::Artist(artist) => Some(artist),
            _ => None,
        }
    }
}

#[derive(Clone, Deserialize)]
struct SongResource {
    id: String,
    attributes: SongAttributes,
    #[serde(default)]
    relationships: Option<SongRelationships>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SongAttributes {
    album_name: Option<String>,
    artwork_url: Option<String>,
    isrc: Option<String>,
    track_number: Option<i32>,
    #[serde(default)]
    genres: Vec<String>,
    release_date: Option<String>,
    duration_ms: Option<i64>,
}

#[derive(Clone, Deserialize)]
struct SongRelationships {
    #[serde(default)]
    albums: Option<List<AlbumResource>>,
}

#[derive(Clone, Deserialize)]
struct List<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Clone, Deserialize)]
struct AlbumResource {
    id: String,
    attributes: AlbumAttributes,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlbumAttributes {
    name: String,
    artwork_url: Option<String>,
    upc: Option<String>,
}

#[derive(Clone, Deserialize)]
struct ArtistResource {
    id: String,
    attributes: ArtistAttributes,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistAttributes {
    name: String,
    artwork_url: Option<String>,
}

#[derive(Deserialize)]
struct Single {
    data: Resource,
}

#[derive(Deserialize)]
struct Many {
    data: Vec<Resource>,
}

fn agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .user_agent(concat!("vleer/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    })
}

fn pass_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn throttle() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock();
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < REQUEST_INTERVAL {
            std::thread::sleep(REQUEST_INTERVAL - elapsed);
        }
    }
    *last = Some(Instant::now());
}

fn blocking<F, T>(f: F) -> futures::channel::oneshot::Receiver<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("vleer-metadata-io".into())
        .spawn(move || {
            let _ = tx.send(f());
        });
    if let Err(e) = spawned {
        warn!("failed to spawn metadata IO thread: {e}");
    }
    rx
}

fn retry_after<B>(response: &Response<B>) -> Duration {
    let secs = ["retry-after", "x-ratelimit-after"]
        .iter()
        .find_map(|name| {
            response
                .headers()
                .get(*name)?
                .to_str()
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(1)
        .clamp(1, MAX_RETRY_AFTER_SECS);
    Duration::from_secs(secs)
}

fn sleep_unless_cancelled(duration: Duration, cancel: &AtomicBool) -> Result<()> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if cancel.load(Ordering::Acquire) {
            return Err(anyhow!("metadata pass cancelled"));
        }
        std::thread::sleep(Duration::from_millis(100).min(deadline - Instant::now()));
    }
    Ok(())
}

fn get_json<T: serde::de::DeserializeOwned>(
    path: &str,
    query: &[(&str, &str)],
    cancel: &AtomicBool,
) -> Result<Option<T>> {
    let url = format!("{BASE_URL}{path}");

    for _ in 0..=RATE_LIMIT_RETRIES {
        throttle();
        let mut request = agent().get(&url);
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(e) => {
                warn!("metadata request failed: GET {url} {query:?} -> {e}");
                return Err(e.into());
            }
        };

        match response.status().as_u16() {
            200 => return Ok(Some(response.body_mut().read_json::<T>()?)),
            400 | 404 => return Ok(None),
            429 => {
                let wait = retry_after(&response);
                debug!("metadata API rate limited, waiting {wait:?}");
                sleep_unless_cancelled(wait, cancel)?;
            }
            status => {
                let body = response.body_mut().read_to_string().unwrap_or_default();
                warn!("metadata request: GET {url} {query:?} -> HTTP {status}, body: {body}");
                return Err(anyhow!("{path} returned HTTP {status}"));
            }
        }
    }

    Err(anyhow!("{path} stayed rate limited"))
}

fn clip(value: &str) -> &str {
    match value.char_indices().nth(MAX_NAME_CHARS) {
        Some((end, _)) => &value[..end],
        None => value,
    }
}

fn lookup_chunks(values: &[String]) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for value in values {
        if value.is_empty() || value.contains(',') {
            continue;
        }
        let extra = if current.is_empty() {
            value.len()
        } else {
            value.len() + 1
        };
        if count >= MAX_LOOKUP_VALUES || current.len() + extra > MAX_LOOKUP_CHARS {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
        if !current.is_empty() {
            current.push(',');
        }
        current.push_str(value);
        count += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn catalog(param: &str, values: &[String], cancel: &AtomicBool) -> Result<Vec<Resource>> {
    let mut found = Vec::new();
    for joined in lookup_chunks(values) {
        if let Some(many) = get_json::<Many>("/catalog", &[(param, joined.as_str())], cancel)? {
            found.extend(many.data);
        }
    }
    Ok(found)
}

fn identify(kind: &str, query: &[(&str, &str)], cancel: &AtomicBool) -> Result<Option<Resource>> {
    Ok(get_json::<Single>(&format!("/identify/{kind}"), query, cancel)?.map(|r| r.data))
}

fn identify_song(song: &SongLookup, cancel: &AtomicBool) -> Result<Option<SongResource>> {
    let duration = song.duration.to_string();
    let mut query = vec![("name", clip(song.title.trim()))];
    if !song.artist.trim().is_empty() {
        query.push(("artist", clip(song.artist.trim())));
    }
    if let Some(album) = song
        .album
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        query.push(("album", clip(album)));
    }
    if song.duration > 0 {
        query.push(("duration", &duration));
    }
    Ok(identify("song", &query, cancel)?.and_then(Resource::into_song))
}

fn identify_album(album: &AlbumLookup, cancel: &AtomicBool) -> Result<Option<AlbumResource>> {
    let mut query = vec![("name", clip(album.title.trim()))];
    if !album.artist.trim().is_empty() {
        query.push(("artist", clip(album.artist.trim())));
    }
    Ok(identify("album", &query, cancel)?.and_then(Resource::into_album))
}

fn identify_artist(name: &str, cancel: &AtomicBool) -> Result<Option<ArtistResource>> {
    Ok(identify("artist", &[("name", clip(name))], cancel)?.and_then(Resource::into_artist))
}

fn songs_by_isrc(
    isrcs: &[String],
    cancel: &AtomicBool,
) -> Result<HashMap<String, Vec<SongResource>>> {
    let mut found: HashMap<String, Vec<SongResource>> = HashMap::new();
    for resource in catalog("isrc", isrcs, cancel)? {
        if let Some(song) = resource.into_song()
            && let Some(isrc) = song.attributes.isrc.as_deref().and_then(normalize_isrc)
        {
            found.entry(isrc).or_default().push(song);
        }
    }
    Ok(found)
}

fn upc_key(upc: &str) -> &str {
    upc.trim_start_matches('0')
}

fn albums_by_upc(
    upcs: &[String],
    cancel: &AtomicBool,
) -> Result<HashMap<String, Vec<AlbumResource>>> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for upc in upcs {
        let variants = [
            Some(upc.clone()),
            upc.strip_prefix('0')
                .filter(|_| upc.len() == 13)
                .map(str::to_string),
            (upc.len() == 12).then(|| format!("0{upc}")),
        ];
        for variant in variants.into_iter().flatten() {
            if seen.insert(variant.clone()) {
                values.push(variant);
            }
        }
    }

    let mut found: HashMap<String, Vec<AlbumResource>> = HashMap::new();
    for resource in catalog("upc", &values, cancel)? {
        if let Some(album) = resource.into_album()
            && let Some(upc) = album.attributes.upc.as_deref().and_then(normalize_upc)
        {
            found
                .entry(upc_key(&upc).to_string())
                .or_default()
                .push(album);
        }
    }
    Ok(found)
}

fn pick_song(candidates: Vec<SongResource>, song: &SongLookup) -> Option<SongResource> {
    let album = song.album.as_deref().map(str::to_lowercase);
    candidates.into_iter().min_by_key(|candidate| {
        let album_match = match (&album, &candidate.attributes.album_name) {
            (Some(local), Some(remote)) => local == &remote.to_lowercase(),
            _ => false,
        };
        let duration_gap = candidate
            .attributes
            .duration_ms
            .map(|ms| (ms - song.duration as i64).abs())
            .unwrap_or(i64::MAX);
        (!album_match, duration_gap)
    })
}

fn pick_album(candidates: Vec<AlbumResource>, album: &AlbumLookup) -> Option<AlbumResource> {
    let title = album.title.trim().to_lowercase();
    candidates
        .into_iter()
        .min_by_key(|candidate| candidate.attributes.name.to_lowercase() != title)
}

fn related_album<'a>(
    song: &'a SongResource,
    local_album: Option<&str>,
) -> Option<&'a AlbumResource> {
    let albums = &song.relationships.as_ref()?.albums.as_ref()?.data;
    local_album
        .map(str::to_lowercase)
        .and_then(|local| {
            albums
                .iter()
                .find(|album| album.attributes.name.to_lowercase() == local)
        })
        .or_else(|| (albums.len() == 1).then(|| &albums[0]))
}

fn sized_artwork_url(artwork_url: &str) -> String {
    let url = artwork_url
        .replace("{w}", ARTWORK_SIZE)
        .replace("{h}", ARTWORK_SIZE);
    let Some((head, file)) = url.rsplit_once('/') else {
        return url;
    };
    let Some((dims, rest)) = file.split_once("bb.") else {
        return url;
    };
    match dims.split_once('x') {
        Some((w, h))
            if !w.is_empty()
                && !h.is_empty()
                && w.bytes().all(|b| b.is_ascii_digit())
                && h.bytes().all(|b| b.is_ascii_digit()) =>
        {
            format!("{head}/{ARTWORK_SIZE}x{ARTWORK_SIZE}bb.{rest}")
        }
        _ => url,
    }
}

fn download_artwork(artwork_url: &str) -> Result<Option<Vec<u8>>> {
    let url = sized_artwork_url(artwork_url);
    let mut response = match agent().get(&url).call() {
        Ok(response) => response,
        Err(e) => {
            warn!("artwork request failed: GET {url} -> {e}");
            return Err(e.into());
        }
    };

    match response.status().as_u16() {
        200 => match response.body_mut().read_to_vec() {
            Ok(bytes) => Ok(Some(bytes).filter(|b| !b.is_empty())),
            Err(e) => {
                warn!("artwork body read failed: GET {url} -> {e}");
                Err(e.into())
            }
        },
        404 | 410 => Ok(None),
        status => {
            let body = response.body_mut().read_to_string().unwrap_or_default();
            warn!("artwork request: GET {url} -> HTTP {status}, body: {body}");
            Err(anyhow!("artwork download returned HTTP {status}"))
        }
    }
}

async fn fetch_prepared_image(db: &Database, url: String) -> Result<Option<PreparedImage>> {
    let raw = blocking(move || download_artwork(&url))
        .await
        .map_err(|_| anyhow!("artwork download cancelled"))??;
    let Some(raw) = raw else {
        return Ok(None);
    };
    match prepare_image(db, raw).await {
        Ok(image) => Ok(Some(image)),
        Err(e) => {
            warn!("artwork could not be stored: {e}");
            Ok(None)
        }
    }
}

fn stored_image(image: &Option<PreparedImage>) -> Option<(&str, Option<&[u8]>)> {
    image
        .as_ref()
        .map(|(image_id, data)| (image_id.as_str(), data.as_deref()))
}

pub async fn fetch_artwork_urls(ids: Vec<String>) -> HashMap<String, String> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let cancel = AtomicBool::new(false);
    match blocking(move || catalog("ids", &ids, &cancel)).await {
        Ok(Ok(found)) => found
            .iter()
            .filter_map(|resource| {
                let url = resource.artwork_url().filter(|url| !url.is_empty())?;
                Some((resource.id().to_string(), sized_artwork_url(url)))
            })
            .collect(),
        Ok(Err(e)) => {
            warn!("catalog artwork lookup failed: {e}");
            HashMap::new()
        }
        Err(_) => HashMap::new(),
    }
}

async fn resolve_artist_metadata(
    db: Database,
    artist_id: Cuid,
    name: String,
    cancel: Arc<AtomicBool>,
) -> Result<bool> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME_CHARS {
        write_with_retry(move || db.mark_artist_metadata_missing(&artist_id)).await?;
        return Ok(false);
    }

    let identified = blocking(move || identify_artist(&trimmed, &cancel))
        .await
        .map_err(|_| anyhow!("artist lookup cancelled"))??;

    let Some(artist) = identified else {
        write_with_retry(move || db.mark_artist_metadata_missing(&artist_id)).await?;
        return Ok(false);
    };

    let image = match artist
        .attributes
        .artwork_url
        .clone()
        .filter(|url| !url.is_empty())
    {
        Some(url) => fetch_prepared_image(&db, url).await?,
        None => None,
    };

    let ArtistResource {
        id: omm_id,
        attributes,
    } = artist;
    let stored = write_with_retry(move || {
        db.store_artist_metadata(
            &artist_id,
            &ArtistMetadata {
                omm_id: &omm_id,
                name: &attributes.name,
                image: stored_image(&image),
            },
        )
    })
    .await?;

    Ok(stored.is_some())
}

pub async fn warm_artist_metadata(db: Database, cancel: Arc<AtomicBool>) -> usize {
    let _pass = pass_lock().lock().await;
    let mut resolved = 0usize;

    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        let batch = match db.artists_needing_metadata(WARM_BATCH) {
            Ok(batch) if !batch.is_empty() => batch,
            Ok(_) => break,
            Err(e) => {
                warn!("metadata pass could not list artists: {e}");
                break;
            }
        };

        for (artist_id, name) in batch {
            if cancel.load(Ordering::Acquire) {
                return resolved;
            }

            match resolve_artist_metadata(db.clone(), artist_id, name, cancel.clone()).await {
                Ok(true) => resolved += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!("metadata pass stopped: {e}");
                    return resolved;
                }
            }
        }
    }

    resolved
}

async fn resolve_song_metadata(
    db: Database,
    song: SongLookup,
    candidates: Vec<SongResource>,
    cancel: Arc<AtomicBool>,
) -> Result<bool> {
    let song_id = song.id.clone();
    let title_for_log = song.title.clone();
    let title = song.title.trim();
    if title.is_empty() || title.chars().count() > MAX_NAME_CHARS {
        write_with_retry(move || db.mark_song_metadata_missing(&song_id)).await?;
        return Ok(false);
    }

    let local_album = song.album.clone();
    let identified = blocking(move || {
        if let Some(matched) = pick_song(candidates, &song) {
            return Ok(Some(matched));
        }
        identify_song(&song, &cancel)
    })
    .await
    .map_err(|_| anyhow!("song lookup cancelled"))??;

    let Some(matched) = identified else {
        write_with_retry(move || db.mark_song_metadata_missing(&song_id)).await?;
        return Ok(false);
    };

    let has_image = db
        .song_image_target(&song_id)?
        .is_some_and(|target| target.image_id.is_some());
    let song_image = match matched
        .attributes
        .artwork_url
        .clone()
        .filter(|url| !url.is_empty())
    {
        Some(url) if !has_image => fetch_prepared_image(&db, url).await.unwrap_or_else(|e| {
            warn!("artwork for song {song_id} ({title_for_log}) skipped: {e}");
            None
        }),
        _ => None,
    };

    let related = related_album(&matched, local_album.as_deref());
    let upc = related
        .and_then(|album| album.attributes.upc.as_deref())
        .and_then(normalize_upc);
    let album_omm_id = related.map(|album| album.id.clone());
    let album_artwork = related
        .and_then(|album| album.attributes.artwork_url.clone())
        .filter(|url| !url.is_empty());
    let album_image = match album_artwork {
        Some(url) if db.song_album_needs_image(&song_id)? => {
            fetch_prepared_image(&db, url).await.unwrap_or_else(|e| {
                warn!("album artwork for song {song_id} ({title_for_log}) skipped: {e}");
                None
            })
        }
        _ => None,
    };

    let isrc = matched.attributes.isrc.as_deref().and_then(normalize_isrc);
    let omm_id = matched.id;
    let attributes = matched.attributes;
    let stored = write_with_retry(move || {
        let year = attributes
            .release_date
            .as_deref()
            .and_then(|d| d.get(..4))
            .filter(|y| y.bytes().all(|b| b.is_ascii_digit()));
        db.store_song_metadata(
            &song_id,
            &SongBackfill {
                omm_id: &omm_id,
                track_number: attributes.track_number,
                year,
                genres: &attributes.genres,
                image: stored_image(&song_image),
                isrc: isrc.as_deref(),
                upc: upc.as_deref(),
                album_image: stored_image(&album_image),
                album_omm_id: album_omm_id.as_deref(),
            },
        )
    })
    .await?;

    Ok(stored)
}

pub async fn warm_song_metadata(db: Database, cancel: Arc<AtomicBool>) -> usize {
    let _pass = pass_lock().lock().await;
    let mut resolved = 0usize;

    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        let batch = match db.songs_needing_metadata(WARM_BATCH) {
            Ok(batch) if !batch.is_empty() => batch,
            Ok(_) => break,
            Err(e) => {
                warn!("metadata pass could not list songs: {e}");
                break;
            }
        };

        let isrcs: Vec<String> = batch
            .iter()
            .filter_map(|song| song.isrc.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let by_isrc = if isrcs.is_empty() {
            HashMap::new()
        } else {
            let lookup_cancel = cancel.clone();
            match blocking(move || songs_by_isrc(&isrcs, &lookup_cancel)).await {
                Ok(Ok(found)) => found,
                Ok(Err(e)) => {
                    warn!("isrc batch lookup failed, falling back to identify: {e}");
                    HashMap::new()
                }
                Err(_) => HashMap::new(),
            }
        };

        for song in batch {
            if cancel.load(Ordering::Acquire) {
                return resolved;
            }

            let song_id = song.id.clone();
            let song_title = song.title.clone();
            let candidates = song
                .isrc
                .as_deref()
                .and_then(normalize_isrc)
                .and_then(|isrc| by_isrc.get(&isrc).cloned())
                .unwrap_or_default();
            match resolve_song_metadata(db.clone(), song, candidates, cancel.clone()).await {
                Ok(true) => resolved += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!("song metadata pass stopped at {song_id} ({song_title}): {e:#}");
                    return resolved;
                }
            }
        }
    }

    resolved
}

async fn resolve_album_metadata(
    db: Database,
    album: AlbumLookup,
    candidates: Vec<AlbumResource>,
    cancel: Arc<AtomicBool>,
) -> Result<bool> {
    let album_id = album.id.clone();
    let title = album.title.trim();
    if title.is_empty() || title.chars().count() > MAX_NAME_CHARS {
        write_with_retry(move || db.mark_album_metadata_missing(&album_id)).await?;
        return Ok(false);
    }

    let identified = blocking(move || {
        if let Some(matched) = pick_album(candidates, &album) {
            return Ok(Some(matched));
        }
        identify_album(&album, &cancel)
    })
    .await
    .map_err(|_| anyhow!("album lookup cancelled"))??;

    let Some(matched) = identified else {
        write_with_retry(move || db.mark_album_metadata_missing(&album_id)).await?;
        return Ok(false);
    };

    let image = match matched
        .attributes
        .artwork_url
        .clone()
        .filter(|url| !url.is_empty())
    {
        Some(url) if db.album_needs_image(&album_id)? => {
            fetch_prepared_image(&db, url).await.unwrap_or_else(|e| {
                warn!("artwork for album {album_id} skipped: {e}");
                None
            })
        }
        _ => None,
    };

    let upc = matched.attributes.upc.as_deref().and_then(normalize_upc);
    let omm_id = matched.id;
    let stored = write_with_retry(move || {
        db.store_album_metadata(
            &album_id,
            &AlbumMetadata {
                omm_id: &omm_id,
                upc: upc.as_deref(),
                image: stored_image(&image),
            },
        )
    })
    .await?;

    Ok(stored)
}

pub async fn warm_album_metadata(db: Database, cancel: Arc<AtomicBool>) -> usize {
    let _pass = pass_lock().lock().await;
    let mut resolved = 0usize;

    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        let batch = match db.albums_needing_metadata(WARM_BATCH) {
            Ok(batch) if !batch.is_empty() => batch,
            Ok(_) => break,
            Err(e) => {
                warn!("metadata pass could not list albums: {e}");
                break;
            }
        };

        let upcs: Vec<String> = batch
            .iter()
            .filter_map(|album| album.upc.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let by_upc = if upcs.is_empty() {
            HashMap::new()
        } else {
            let lookup_cancel = cancel.clone();
            match blocking(move || albums_by_upc(&upcs, &lookup_cancel)).await {
                Ok(Ok(found)) => found,
                Ok(Err(e)) => {
                    warn!("upc batch lookup failed, falling back to identify: {e}");
                    HashMap::new()
                }
                Err(_) => HashMap::new(),
            }
        };

        for album in batch {
            if cancel.load(Ordering::Acquire) {
                return resolved;
            }

            let album_id = album.id.clone();
            let album_title = album.title.clone();
            let candidates = album
                .upc
                .as_deref()
                .and_then(|upc| by_upc.get(upc_key(upc)))
                .cloned()
                .unwrap_or_default();
            match resolve_album_metadata(db.clone(), album, candidates, cancel.clone()).await {
                Ok(true) => resolved += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!("album metadata pass stopped at {album_id} ({album_title}): {e:#}");
                    return resolved;
                }
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_song_with_nested_albums() {
        let body = r#"{"data":[{"id":"omm:song:0a1b2c3d4e5f6g7h","type":"song",
            "attributes":{"name":"Song","albumName":"Album","artistNames":["Artist"],"isrc":"USRC17607839",
                "durationMs":180000,"genres":["Pop"],"releaseDate":"2020-01-02","trackNumber":3},
            "relationships":{"albums":{"data":[{"id":"omm:album:1a2b3c4d5e6f7g8h","type":"album",
                "attributes":{"name":"Album","trackCount":10,"artistNames":["Artist"],"upc":"602455878540",
                    "artworkUrl":"https://x/{w}x{h}bb.jpg"}}]},
                "artists":{"data":[{"id":"omm:artist:2a3b4c5d6e7f8g9h","type":"artist",
                    "attributes":{"name":"Artist","genres":["Pop"]}}]}}},
            {"id":"omm:artist:2a3b4c5d6e7f8g9h","type":"artist","attributes":{"name":"Artist"}}]}"#;
        let many: Many = serde_json::from_str(body).unwrap();
        assert_eq!(many.data.len(), 2);
        let song = many.data[0].clone().into_song().unwrap();
        assert_eq!(song.attributes.duration_ms, Some(180_000));
        let album = related_album(&song, Some("album")).unwrap();
        assert_eq!(album.attributes.upc.as_deref(), Some("602455878540"));
        assert!(many.data[1].clone().into_artist().is_some());
    }

    #[test]
    fn chunks_lookup_values_by_count_and_size() {
        let values: Vec<String> = (0..250).map(|i| format!("v{i}")).collect();
        let chunks = lookup_chunks(&values);
        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|c| c.split(',').count() <= MAX_LOOKUP_VALUES)
        );
        let wide: Vec<String> = (0..50).map(|_| "x".repeat(400)).collect();
        assert!(
            lookup_chunks(&wide)
                .iter()
                .all(|c| c.len() <= MAX_LOOKUP_CHARS)
        );
    }

    #[test]
    fn clips_to_the_spec_length() {
        assert_eq!(clip(&"é".repeat(300)).chars().count(), MAX_NAME_CHARS);
        assert_eq!(clip("short"), "short");
    }
}
