use anyhow::{Result, anyhow};
use image::imageops::FilterType;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::data::db::{AlbumImageTarget, Database};
pub use crate::data::jobs::priority;
use crate::data::jobs::{Bulk, Claims, Job, JobContext, Stop};
use crate::data::metadata::{convert_to_jpeg, encode_cover, image_id_for, read_tag_images};
use crate::data::models::Cuid;
use parking_lot::Mutex as PlMutex;
use priority::{Progress, Tier};
use std::sync::atomic::AtomicUsize;

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
                    crate::data::jobs::lower_thread_priority();
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
        if tx.is_canceled() {
            return;
        }
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

pub async fn resolve_song_file_image(db: Database, song_id: Cuid) -> Result<String> {
    resolve_song_and_artist_image(db, song_id).await.0
}

pub async fn resolve_song_image(db: Database, song_id: Cuid) -> Result<String> {
    match resolve_song_file_image(db.clone(), song_id.clone()).await {
        Err(e) if is_missing_image(&e.to_string()) => match db.get_song_album_image(&song_id) {
            Ok(Some(image_id)) => Ok(image_id),
            _ => Err(e),
        },
        other => other,
    }
}

async fn resolve_song_and_artist_image(db: Database, song_id: Cuid) -> (Result<String>, bool) {
    let target = match db.get_song_image_target(&song_id) {
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
    if db.has_image(&image_id)? {
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
    if !db.song_artist_needs_image(&song_id)? {
        return Ok(false);
    }

    let (image_id, encoded) = prepare_image(&db, raw).await?;
    write_with_retry(move || db.store_song_artist_image(&song_id, &image_id, encoded.as_deref()))
        .await
}

fn first_unblocked(ids: Vec<Cuid>, claims: &Claims) -> Option<Cuid> {
    ids.into_iter().find(|id| !claims.blocked(id))
}

fn priority_image_job(db: &Database, claims: &Claims) -> Option<(Tier, Cuid)> {
    let snapshot = priority::snapshot();

    if let Some(playing) = snapshot.playing {
        let needing = db
            .get_songs_without_image_in(&[playing])
            .unwrap_or_default();
        if let Some(id) = first_unblocked(needing, claims) {
            return Some((Tier::Playing, id));
        }
    }

    let needing = db
        .get_songs_without_image_in(&snapshot.songs)
        .unwrap_or_default();
    if let Some(id) = first_unblocked(needing, claims) {
        return Some((Tier::Visible, id));
    }

    let needing = db
        .get_album_songs_without_image(&snapshot.albums)
        .unwrap_or_default();
    if let Some(id) = first_unblocked(needing, claims) {
        return Some((Tier::Visible, id));
    }

    let mut needing = db
        .get_songs_without_image_in(&snapshot.near_songs)
        .unwrap_or_default();
    needing.sort_by_key(|id| snapshot.near_songs.iter().position(|near| near == id));
    if let Some(id) = first_unblocked(needing, claims) {
        return Some((Tier::Near, id));
    }

    let needing = db
        .get_album_songs_without_image(&snapshot.near_albums)
        .unwrap_or_default();
    first_unblocked(needing, claims).map(|id| (Tier::Near, id))
}

pub struct ImageJob {
    ctx: JobContext,
    claims: Claims,
    resolved: AtomicUsize,
    progress: PlMutex<Progress>,
}

impl ImageJob {
    pub fn new(ctx: &JobContext) -> Self {
        Self {
            ctx: ctx.clone(),
            claims: Claims::default(),
            resolved: AtomicUsize::new(0),
            progress: PlMutex::new(ctx.progress()),
        }
    }
}

impl Job for ImageJob {
    type Work = Cuid;

    fn name(&self) -> &'static str {
        "image"
    }

    fn prepare(&self) {
        priority::set_file_busy(true);
    }

    fn priority(&self) -> Option<(Tier, Cuid)> {
        priority_image_job(&self.ctx.db, &self.claims)
    }

    async fn bulk(&self) -> Bulk<Cuid> {
        match self
            .ctx
            .db
            .get_songs_without_image(WARM_BATCH + self.claims.failed() as i64)
        {
            Ok(batch) => {
                let batch: Vec<Cuid> = batch
                    .into_iter()
                    .filter(|id| !self.claims.blocked(id))
                    .collect();
                if batch.is_empty() {
                    Bulk::Done
                } else {
                    Bulk::Works(batch)
                }
            }
            Err(e) => {
                warn!("warm pass could not list songs: {e}");
                Bulk::Done
            }
        }
    }

    async fn run(&self, tier: Tier, song_id: Cuid, _stop: &Stop) {
        let Some(_claim) = self.claims.claim(&song_id) else {
            return;
        };
        let (cover, artist_stored) =
            resolve_song_and_artist_image(self.ctx.db.clone(), song_id.clone()).await;
        if cover.is_ok() || artist_stored {
            if tier == Tier::Bulk {
                self.resolved.fetch_add(1, Ordering::AcqRel);
            }
            self.progress.lock().changed(tier);
            priority::FILE_PROGRESS.fire();
        }
        if cover.is_err() {
            self.claims.fail(song_id);
        }
    }

    fn report(&self) {
        self.progress.lock().flush();
        let resolved = self.resolved.swap(0, Ordering::AcqRel);
        if resolved > 0 {
            info!("Resolved {resolved} cover/artist image(s)");
            crate::data::jobs::spawn_genre_covers(&self.ctx);
            self.ctx.library_changed();
        }
        priority::set_file_busy(false);
    }

    fn resume(&self) {
        priority::set_file_busy(true);
    }
}

pub async fn resolve_album_image(db: Database, album_id: Cuid) -> Result<String> {
    let candidates = match db.get_album_image_target(&album_id)? {
        Some(AlbumImageTarget::Resolved(image_id)) => return Ok(image_id),
        Some(AlbumImageTarget::Candidates(candidates)) => candidates,
        None => return Err(anyhow!(NO_IMAGE)),
    };

    for song_id in candidates {
        if let Ok(image_id) = resolve_song_file_image(db.clone(), song_id).await {
            return Ok(image_id);
        }
    }

    Err(anyhow!(NO_IMAGE))
}

const GENRE_COVER_SIZE: u32 = 512;
const GENRE_COVER_TILES: usize = 4;

const GENRE_TINTS: [[f32; 3]; 10] = [
    [0.0, 190.0, 110.0],
    [240.0, 100.0, 50.0],
    [255.0, 0.0, 60.0],
    [230.0, 180.0, 0.0],
    [240.0, 90.0, 140.0],
    [90.0, 130.0, 240.0],
    [140.0, 70.0, 200.0],
    [0.0, 150.0, 160.0],
    [190.0, 200.0, 0.0],
    [255.0, 120.0, 0.0],
];

struct GenreCoverPlan {
    image_ids: Vec<String>,
    image_id: String,
    tint: [f32; 3],
}

fn plan_genre_cover(db: &Database, genre_id: &Cuid) -> Result<Option<GenreCoverPlan>> {
    let mut image_ids = db.get_genre_image_ids(genre_id)?;
    if image_ids.is_empty() {
        return Ok(None);
    }
    let seed = xxhash_rust::xxh3::xxh3_64(genre_id.to_string().as_bytes());
    image_ids.sort();
    image_ids.shuffle(&mut StdRng::seed_from_u64(seed));
    image_ids.truncate(GENRE_COVER_TILES);
    let image_id = image_id_for(format!("{genre_id}:{}", image_ids.join(",")).as_bytes());
    let tint = GENRE_TINTS[(seed >> 32) as usize % GENRE_TINTS.len()];
    Ok(Some(GenreCoverPlan {
        image_ids,
        image_id,
        tint,
    }))
}

fn compose_tinted_grid(blobs: &[Vec<u8>], size: u32, tint: [f32; 3]) -> Result<Vec<u8>> {
    let tiles = blobs
        .iter()
        .map(|bytes| image::load_from_memory(bytes).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;

    let mut canvas = image::RgbaImage::new(size, size);
    if tiles.len() == 1 {
        let tile = tiles[0].resize_to_fill(size, size, FilterType::Triangle);
        image::imageops::replace(&mut canvas, &tile.into_rgba8(), 0, 0);
    } else {
        let half = size / 2;
        for slot in 0..4u32 {
            let tile = tiles[slot as usize % tiles.len()]
                .resize_to_fill(half, half, FilterType::Triangle)
                .into_rgba8();
            image::imageops::replace(
                &mut canvas,
                &tile,
                i64::from((slot % 2) * half),
                i64::from((slot / 2) * half),
            );
        }
    }

    for pixel in canvas.as_chunks_mut::<4>().0.iter_mut() {
        let luma =
            (0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32) / 255.0;
        let factor = 0.3 + 0.85 * luma;
        pixel[0] = (tint[0] * factor).min(255.0) as u8;
        pixel[1] = (tint[1] * factor).min(255.0) as u8;
        pixel[2] = (tint[2] * factor).min(255.0) as u8;
        pixel[3] = 255;
    }

    convert_to_jpeg(image::DynamicImage::ImageRgb8(
        image::DynamicImage::ImageRgba8(canvas).into_rgb8(),
    ))
}

async fn build_genre_cover(db: &Database, genre_id: &Cuid, plan: GenreCoverPlan) -> Result<String> {
    let blobs = db.get_image_blobs(&plan.image_ids)?;
    if blobs.is_empty() {
        return Err(anyhow!(NO_IMAGE));
    }
    let bytes = {
        let _permit = decode_limit().acquire().await;
        compose_tinted_grid(&blobs, GENRE_COVER_SIZE, plan.tint)?
    };
    let db = db.clone();
    let genre_id = genre_id.clone();
    let image_id = plan.image_id.clone();
    write_with_retry(move || db.store_genre_cover(&genre_id, &image_id, &bytes)).await?;
    Ok(plan.image_id)
}

pub async fn resolve_genre_cover(db: Database, genre_id: Cuid) -> Result<String> {
    if let Some(image_id) = db.get_genre_image_id(&genre_id)? {
        return Ok(image_id);
    }
    match plan_genre_cover(&db, &genre_id)? {
        Some(plan) => build_genre_cover(&db, &genre_id, plan).await,
        None => Err(anyhow!(NO_IMAGE)),
    }
}

fn genre_pass_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub async fn warm_genre_covers(db: Database, cancel: Arc<AtomicBool>) -> usize {
    let _pass = genre_pass_lock().lock().await;
    let genres = match db.get_genre_covers() {
        Ok(genres) => genres,
        Err(e) => {
            warn!("genre cover pass could not list genres: {e}");
            return 0;
        }
    };

    let mut built = 0;
    for (genre_id, stored_id) in genres {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let plan = match plan_genre_cover(&db, &genre_id) {
            Ok(Some(plan)) if stored_id.as_deref() != Some(plan.image_id.as_str()) => plan,
            Ok(_) => continue,
            Err(e) => {
                warn!("genre cover {genre_id} skipped: {e}");
                continue;
            }
        };
        match build_genre_cover(&db, &genre_id, plan).await {
            Ok(_) => built += 1,
            Err(e) => warn!("genre cover {genre_id} skipped: {e}"),
        }
    }
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db::{BatchTrack, ScanCache};

    fn jpeg_of(shade: u8) -> Vec<u8> {
        convert_to_jpeg(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            8,
            image::Rgb([shade, shade, shade]),
        )))
        .unwrap()
    }

    #[test]
    fn genre_covers_are_stored_and_rebuilt_when_artwork_changes() {
        let path =
            std::path::PathBuf::from(format!("/tmp/vleer_genre_covers_{}.db", std::process::id()));
        let db = Database::new(&path).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let warm = || futures::executor::block_on(warm_genre_covers(db.clone(), cancel.clone()));
        let add = |file: &str, shade: u8| {
            db.upsert_tracks(
                &[BatchTrack {
                    title: "Title",
                    artists: &[],
                    genres: &["Pop"],
                    album: None,
                    album_artist: None,
                    file_path: file,
                    audio_hash: file,
                    duration: 180_000,
                    track_number: None,
                    year: None,
                    recheck_image: false,
                    file_size: 1_000,
                    file_modified: 42,
                    lufs: None,
                    isrc: None,
                    upc: None,
                    omm_song_id: None,
                    omm_album_id: None,
                    omm_artist_id: None,
                }],
                &mut ScanCache::default(),
            )
            .unwrap();
            let bytes = jpeg_of(shade);
            let image_id = image_id_for(&bytes);
            let song_id = db.get_song_by_path(file).unwrap().unwrap().id;
            db.store_song_image(&song_id, &image_id, Some(&bytes))
                .unwrap();
        };
        let genre = Cuid::for_genre("Pop");

        assert_eq!(warm(), 0);
        assert_eq!(db.get_genre_image_id(&genre).unwrap(), None);

        add("/music/1.flac", 40);
        assert_eq!(warm(), 1);
        let first = db
            .get_genre_image_id(&genre)
            .unwrap()
            .expect("cover stored");
        assert_eq!(warm(), 0);

        add("/music/2.flac", 200);
        assert_eq!(warm(), 1);
        assert_ne!(db.get_genre_image_id(&genre).unwrap().unwrap(), first);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
