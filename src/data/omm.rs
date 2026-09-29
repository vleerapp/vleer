use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use ureq::Agent;
use ureq::http::Response;

use crate::data::db::repo::{ArtistMetadata, Database, SongBackfill, SongLookup};
use crate::data::images::{prepare_image, write_with_retry};
use crate::data::models::Cuid;

const BASE_URL: &str = "https://api.vleer.app/metadata/v1";
const WARM_BATCH: i64 = 64;
const MAX_NAME_CHARS: usize = 256;
const ARTWORK_SIZE: &str = "512";
const REQUEST_INTERVAL: Duration = Duration::from_millis(50);
const RATE_LIMIT_RETRIES: u32 = 5;
const MAX_RETRY_AFTER_SECS: u64 = 60;

#[derive(Deserialize)]
struct IdentifyResponse {
    data: Identified,
}

#[derive(Deserialize)]
struct Identified {
    id: String,
    attributes: Attributes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attributes {
    name: String,
    artwork_url: Option<String>,
    #[serde(default)]
    track_number: Option<i32>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    release_date: Option<String>,
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

fn identify(kind: &str, query: &[(&str, &str)], cancel: &AtomicBool) -> Result<Option<Identified>> {
    let url = format!("{BASE_URL}/identify/{kind}");

    for _ in 0..=RATE_LIMIT_RETRIES {
        throttle();
        let mut request = agent().get(&url);
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let mut response = match request.call() {
            Ok(response) => response,
            Err(e) => {
                warn!("identify request failed: GET {url} {query:?} -> {e}");
                return Err(e.into());
            }
        };

        match response.status().as_u16() {
            200 => {
                return Ok(Some(
                    response.body_mut().read_json::<IdentifyResponse>()?.data,
                ));
            }
            400 | 404 => return Ok(None),
            429 => {
                let wait = retry_after(&response);
                debug!("metadata API rate limited, waiting {wait:?}");
                sleep_unless_cancelled(wait, cancel)?;
            }
            status => {
                let body = response.body_mut().read_to_string().unwrap_or_default();
                warn!("identify request: GET {url} {query:?} -> HTTP {status}, body: {body}");
                return Err(anyhow!("identify returned HTTP {status}"));
            }
        }
    }

    Err(anyhow!("identify stayed rate limited"))
}

fn identify_artist(name: &str, cancel: &AtomicBool) -> Result<Option<Identified>> {
    identify("artist", &[("name", name)], cancel)
}

fn identify_song(song: &SongLookup, cancel: &AtomicBool) -> Result<Option<Identified>> {
    let duration = song.duration.to_string();
    let mut query = vec![("name", song.title.trim())];
    if !song.artist.trim().is_empty() {
        query.push(("artist", song.artist.trim()));
    }
    if let Some(album) = song
        .album
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        query.push(("album", album));
    }
    if song.duration > 0 {
        query.push(("duration", &duration));
    }
    identify("song", &query, cancel)
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

pub async fn resolve_artist_metadata(
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

    let raw = match artist.attributes.artwork_url.clone() {
        Some(url) => blocking(move || download_artwork(&url))
            .await
            .map_err(|_| anyhow!("artwork download cancelled"))??,
        None => None,
    };

    let image = match raw {
        Some(raw) => match prepare_image(&db, raw).await {
            Ok(image) => Some(image),
            Err(e) => {
                warn!("artwork for artist {name} could not be stored: {e}");
                None
            }
        },
        None => None,
    };

    let Identified {
        id: omm_id,
        attributes,
    } = artist;
    let stored = write_with_retry(move || {
        db.store_artist_metadata(
            &artist_id,
            &ArtistMetadata {
                omm_id: &omm_id,
                name: &attributes.name,
                image: image
                    .as_ref()
                    .map(|(image_id, data)| (image_id.as_str(), data.as_deref())),
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

pub async fn resolve_song_metadata(
    db: Database,
    song: SongLookup,
    cancel: Arc<AtomicBool>,
) -> Result<bool> {
    let song_id = song.id.clone();
    let title_for_log = song.title.clone();
    let title = song.title.trim();
    if title.is_empty() || title.chars().count() > MAX_NAME_CHARS {
        write_with_retry(move || db.mark_song_metadata_missing(&song_id)).await?;
        return Ok(false);
    }

    let identified = blocking(move || identify_song(&song, &cancel))
        .await
        .map_err(|_| anyhow!("song lookup cancelled"))??;

    let Some(matched) = identified else {
        write_with_retry(move || db.mark_song_metadata_missing(&song_id)).await?;
        return Ok(false);
    };

    let has_image = db
        .song_image_target(&song_id)?
        .is_some_and(|target| target.image_id.is_some());

    let raw = match matched.attributes.artwork_url.clone() {
        Some(url) if !has_image => {
            let source = url.clone();
            match blocking(move || download_artwork(&url))
                .await
                .map_err(|_| anyhow!("artwork download cancelled"))?
            {
                Ok(raw) => raw,
                Err(e) => {
                    warn!(
                        "artwork for song {song_id} ({}) skipped: {source} -> {e}",
                        title_for_log
                    );
                    None
                }
            }
        }
        _ => None,
    };

    let image = match raw {
        Some(raw) => match prepare_image(&db, raw).await {
            Ok(image) => Some(image),
            Err(e) => {
                warn!("artwork for song {song_id} could not be stored: {e}");
                None
            }
        },
        None => None,
    };

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
                image: image
                    .as_ref()
                    .map(|(image_id, data)| (image_id.as_str(), data.as_deref())),
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

        for song in batch {
            if cancel.load(Ordering::Acquire) {
                return resolved;
            }

            let song_id = song.id.clone();
            let song_title = song.title.clone();
            match resolve_song_metadata(db.clone(), song, cancel.clone()).await {
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
