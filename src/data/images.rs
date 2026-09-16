use anyhow::{Result, anyhow};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::data::db::repo::{AlbumImageTarget, Database};
use crate::data::metadata::{encode_cover, image_id_for, read_tag_images};
use crate::data::models::Cuid;

const IMAGE_IO_CONCURRENCY: usize = 4;

const BUSY_RETRIES: u32 = 3;

const WARM_BATCH: i64 = 64;

pub const NO_IMAGE: &str = "no cover image";

pub fn is_missing_image(message: &str) -> bool {
    message == NO_IMAGE
}

type IoJob = Box<dyn FnOnce() + Send + 'static>;

fn image_pool() -> &'static std::sync::mpsc::Sender<IoJob> {
    use std::sync::{Mutex, OnceLock};
    static POOL: OnceLock<std::sync::mpsc::Sender<IoJob>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<IoJob>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..IMAGE_IO_CONCURRENCY {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("vleer-image-io-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let guard = match rx.lock() {
                                Ok(guard) => guard,
                                Err(_) => break,
                            };
                            match guard.recv() {
                                Ok(job) => job,
                                Err(_) => break,
                            }
                        };
                        job();
                    }
                })
                .expect("failed to spawn image IO worker thread");
        }
        tx
    })
}

pub(crate) fn io_spawn<F, T>(f: F) -> futures::channel::oneshot::Receiver<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    let _ = image_pool().send(Box::new(move || {
        let _ = tx.send(f());
    }));
    rx
}

fn io_limit() -> &'static Semaphore {
    static LIMIT: OnceLock<Semaphore> = OnceLock::new();
    LIMIT.get_or_init(|| Semaphore::new(IMAGE_IO_CONCURRENCY))
}

pub fn decode_limit() -> &'static Semaphore {
    static LIMIT: OnceLock<Semaphore> = OnceLock::new();
    LIMIT.get_or_init(|| {
        let permits = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(2))
            .unwrap_or(2);
        Semaphore::new(permits)
    })
}

fn is_busy(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy
                || e.code == rusqlite::ErrorCode::DatabaseLocked
    )
}

pub(crate) async fn write_with_retry<T, F>(f: F) -> Result<T>
where
    F: FnMut() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let mut f = f;
    io_spawn(move || {
        let mut delay_ms = 50;
        for attempt in 0..=BUSY_RETRIES {
            match f() {
                Err(e) if is_busy(&e) && attempt < BUSY_RETRIES => {
                    debug!("image write busy, retrying in {delay_ms}ms");
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    delay_ms *= 2;
                }
                other => return other,
            }
        }
        unreachable!("loop returns on its last attempt")
    })
    .await
    .map_err(|_| anyhow!("image write cancelled"))?
}

pub async fn resolve_song_image(db: Database, song_id: Cuid) -> Result<String> {
    resolve_song_and_artist_image(db, song_id).await.0
}

async fn resolve_song_and_artist_image(db: Database, song_id: Cuid) -> (Result<String>, bool) {
    let target = match db.song_image_target(&song_id) {
        Ok(Some(target)) => target,
        Ok(None) => return (Err(anyhow!("song {song_id} not found")), false),
        Err(e) => return (Err(e), false),
    };

    if let Some(image_id) = target.image_id {
        return (Ok(image_id), false);
    }
    if target.image_checked {
        return (Err(anyhow!(NO_IMAGE)), false);
    }

    let path = target.file_path.clone();
    let images = {
        let _permit = io_limit().acquire().await;
        match io_spawn(move || read_tag_images(Path::new(&path)))
            .await
            .map_err(|_| anyhow!("image read cancelled"))
        {
            Ok(Ok(images)) => images,
            Ok(Err(e)) | Err(e) => return (Err(e), false),
        }
    };

    let artist_stored = match images.artist {
        Some(raw) => match store_song_artist_image(db.clone(), song_id.clone(), raw).await {
            Ok(stored) => stored,
            Err(e) => {
                debug!("artist image from song {song_id} not stored: {e}");
                false
            }
        },
        None => false,
    };

    let Some(raw) = images.cover else {
        let db_for_mark = db.clone();
        let mark_song = song_id.clone();
        if let Err(e) =
            write_with_retry(move || db_for_mark.mark_song_image_missing(&mark_song)).await
        {
            return (Err(e), artist_stored);
        }
        return (Err(anyhow!(NO_IMAGE)), artist_stored);
    };

    let (image_id, encoded) = match prepare_image(&db, raw).await {
        Ok(result) => result,
        Err(e) => return (Err(e), artist_stored),
    };

    let db_for_store = db.clone();
    let (store_song, store_id) = (song_id.clone(), image_id.clone());
    if let Err(e) = write_with_retry(move || {
        db_for_store.store_song_image(&store_song, &store_id, encoded.as_deref())
    })
    .await
    {
        return (Err(e), artist_stored);
    }

    (Ok(image_id), artist_stored)
}

pub(crate) async fn prepare_image(
    db: &Database,
    raw: Vec<u8>,
) -> Result<(String, Option<Vec<u8>>)> {
    let image_id = image_id_for(&raw);
    if db.image_exists(&image_id)? {
        return Ok((image_id, None));
    }

    let encoded = {
        let _permit = decode_limit().acquire().await;
        io_spawn(move || encode_cover(&raw))
            .await
            .map_err(|_| anyhow!("image encode cancelled"))??
    };

    Ok((image_id, Some(encoded)))
}

async fn store_song_artist_image(db: Database, song_id: Cuid, raw: Vec<u8>) -> Result<bool> {
    if !db.song_lead_artist_needs_image(&song_id)? {
        return Ok(false);
    }

    let (image_id, encoded) = prepare_image(&db, raw).await?;
    write_with_retry(move || db.store_song_artist_image(&song_id, &image_id, encoded.as_deref()))
        .await
}

pub async fn warm_images(db: Database, cancel: Arc<AtomicBool>) -> usize {
    let mut resolved = 0usize;

    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        let batch = match db.songs_needing_image(WARM_BATCH) {
            Ok(batch) if !batch.is_empty() => batch,
            Ok(_) => break,
            Err(e) => {
                warn!("warm pass could not list songs: {e}");
                break;
            }
        };

        for song_id in batch {
            if cancel.load(Ordering::Acquire) {
                return resolved;
            }

            let (cover, artist_stored) = resolve_song_and_artist_image(db.clone(), song_id).await;
            if cover.is_ok() || artist_stored {
                resolved += 1;
            }
        }
    }

    resolved
}

pub async fn resolve_album_image(db: Database, album_id: Cuid) -> Result<String> {
    let candidates = match db.album_image_target(&album_id)? {
        Some(AlbumImageTarget::Resolved(image_id)) => return Ok(image_id),
        Some(AlbumImageTarget::Candidates(candidates)) => candidates,
        None => return Err(anyhow!(NO_IMAGE)),
    };

    for song_id in candidates {
        if let Ok(image_id) = resolve_song_image(db.clone(), song_id).await {
            return Ok(image_id);
        }
    }

    Err(anyhow!(NO_IMAGE))
}
