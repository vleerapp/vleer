use anyhow::{Result, anyhow};
use image::imageops::FilterType;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::data::db::{AlbumImageTarget, Database};
use crate::data::metadata::{convert_to_jpeg, encode_cover, image_id_for, read_tag_images};
use crate::data::models::Cuid;
use priority::{Progress, Tier};

pub mod priority {
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    use crate::data::models::Cuid;

    const MAX_VISIBLE: usize = 64;
    const PROGRESS_INTERVAL: Duration = Duration::from_millis(750);

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    pub enum Tier {
        Playing,
        Visible,
        Bulk,
    }

    #[derive(Clone, Default)]
    pub struct Snapshot {
        pub playing: Option<Cuid>,
        pub songs: Vec<Cuid>,
        pub albums: Vec<Cuid>,
    }

    pub struct Signal {
        epoch: AtomicU64,
        notify: Notify,
    }

    impl Signal {
        pub(crate) const fn new() -> Self {
            Self {
                epoch: AtomicU64::new(0),
                notify: Notify::const_new(),
            }
        }

        pub fn fire(&self) {
            self.epoch.fetch_add(1, Ordering::AcqRel);
            self.notify.notify_waiters();
        }

        pub fn seen(&self) -> u64 {
            self.epoch.load(Ordering::Acquire)
        }

        pub async fn changed(&self, seen: u64) {
            loop {
                let notified = self.notify.notified();
                let mut notified = std::pin::pin!(notified);
                notified.as_mut().enable();
                if self.seen() != seen {
                    return;
                }
                notified.await;
                if self.seen() != seen {
                    return;
                }
            }
        }
    }

    pub static WAKE: Signal = Signal::new();
    pub static FILE_PROGRESS: Signal = Signal::new();
    static FILE_BUSY: AtomicBool = AtomicBool::new(false);

    static STATE: Mutex<Snapshot> = Mutex::new(Snapshot {
        playing: None,
        songs: Vec::new(),
        albums: Vec::new(),
    });

    pub fn wake() {
        WAKE.fire();
    }

    pub fn set_file_busy(busy: bool) {
        FILE_BUSY.store(busy, Ordering::Release);
        FILE_PROGRESS.fire();
    }

    pub fn file_busy() -> bool {
        FILE_BUSY.load(Ordering::Acquire)
    }

    pub fn snapshot() -> Snapshot {
        STATE.lock().clone()
    }

    pub fn set_playing(id: Option<Cuid>) {
        let mut state = STATE.lock();
        if state.playing != id {
            state.playing = id;
            drop(state);
            wake();
        }
    }

    pub fn set_visible_songs(mut ids: Vec<Cuid>) {
        ids.truncate(MAX_VISIBLE);
        let mut state = STATE.lock();
        let changed = !state.albums.is_empty() || state.songs != ids;
        state.albums.clear();
        if changed {
            state.songs = ids;
            drop(state);
            wake();
        }
    }

    pub fn set_visible_albums(mut ids: Vec<Cuid>) {
        ids.truncate(MAX_VISIBLE);
        let mut state = STATE.lock();
        let changed = !state.songs.is_empty() || state.albums != ids;
        state.songs.clear();
        if changed {
            state.albums = ids;
            drop(state);
            wake();
        }
    }

    pub fn clear_visible() {
        let mut state = STATE.lock();
        state.songs.clear();
        state.albums.clear();
    }

    pub struct Progress {
        notify: Arc<dyn Fn() + Send + Sync>,
        last: Instant,
        dirty: bool,
    }

    impl Progress {
        pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Self {
            Self {
                notify,
                last: Instant::now(),
                dirty: false,
            }
        }

        pub fn changed(&mut self, tier: Tier) {
            self.dirty = true;
            if tier == Tier::Playing || self.last.elapsed() >= PROGRESS_INTERVAL {
                self.flush();
            }
        }

        pub fn flush(&mut self) {
            if self.dirty {
                self.dirty = false;
                self.last = Instant::now();
                (self.notify)();
            }
        }
    }
}

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

fn first_unskipped(ids: Vec<Cuid>, skipped: &HashSet<Cuid>) -> Option<Cuid> {
    ids.into_iter().find(|id| !skipped.contains(id))
}

fn priority_image_job(db: &Database, skipped: &HashSet<Cuid>) -> Option<(Tier, Cuid)> {
    let snapshot = priority::snapshot();

    if let Some(playing) = snapshot.playing {
        let needing = db
            .get_songs_without_image_in(&[playing])
            .unwrap_or_default();
        if let Some(id) = first_unskipped(needing, skipped) {
            return Some((Tier::Playing, id));
        }
    }

    let needing = db
        .get_songs_without_image_in(&snapshot.songs)
        .unwrap_or_default();
    if let Some(id) = first_unskipped(needing, skipped) {
        return Some((Tier::Visible, id));
    }

    let needing = db
        .get_album_songs_without_image(&snapshot.albums)
        .unwrap_or_default();
    first_unskipped(needing, skipped).map(|id| (Tier::Visible, id))
}

pub async fn warm_images(
    db: Database,
    cancel: Arc<AtomicBool>,
    mut progress: Progress,
    on_idle: impl Fn(usize),
) {
    let mut bulk_resolved = 0usize;
    let mut skipped: HashSet<Cuid> = HashSet::new();
    let mut bulk: VecDeque<Cuid> = VecDeque::new();

    'work: loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        let job = match priority_image_job(&db, &skipped) {
            Some(job) => Some(job),
            None => {
                if bulk.is_empty() {
                    match db.get_songs_without_image(WARM_BATCH + skipped.len() as i64) {
                        Ok(batch) => {
                            bulk.extend(batch.into_iter().filter(|id| !skipped.contains(id)))
                        }
                        Err(e) => {
                            warn!("warm pass could not list songs: {e}");
                            break;
                        }
                    }
                }
                bulk.pop_front().map(|id| (Tier::Bulk, id))
            }
        };

        let Some((tier, song_id)) = job else {
            progress.flush();
            if bulk_resolved > 0 {
                on_idle(bulk_resolved);
                bulk_resolved = 0;
            }
            priority::set_file_busy(false);
            loop {
                let seen = priority::WAKE.seen();
                if cancel.load(Ordering::Acquire) {
                    break 'work;
                }
                if priority_image_job(&db, &skipped).is_some() {
                    break;
                }
                priority::WAKE.changed(seen).await;
            }
            priority::set_file_busy(true);
            continue;
        };

        let (cover, artist_stored) =
            resolve_song_and_artist_image(db.clone(), song_id.clone()).await;
        if cover.is_ok() || artist_stored {
            if tier == Tier::Bulk {
                bulk_resolved += 1;
            }
            progress.changed(tier);
            priority::FILE_PROGRESS.fire();
        }
        if cover.is_err() {
            skipped.insert(song_id);
        }
    }

    progress.flush();
    if bulk_resolved > 0 {
        on_idle(bulk_resolved);
    }
    priority::set_file_busy(false);
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
