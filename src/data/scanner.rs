use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use gpui::{App, Global};
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::UNIX_EPOCH;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::data::config::Config;
use crate::data::db::repo::Database;
use crate::data::metadata::{AudioMetadata, ImageData, extract_image_data};
use crate::data::models::Cuid;
use crate::data::telemetry::Telemetry;

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "aac", "aiff", "aif", "flac", "mp3", "mp4", "m4a", "mp4a", "ogg", "oga", "opus", "wav", "wv",
];
const MAX_CONCURRENT_SCANS: usize = 4;
const STALE_DELETE_BATCH_SIZE: usize = 400;

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Debug, Clone)]
pub struct ScannedTrack {
    pub path: PathBuf,
    pub file_size: i64,
    pub file_modified: i64,
    pub metadata: AudioMetadata,
    pub image_data: Option<ImageData>,
}

struct ScanOptions {
    force: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { force: false }
    }
}

#[derive(Clone)]
pub struct Scanner {
    scan_paths: Vec<PathBuf>,
    scan_lock: Arc<Mutex<()>>,
    pending_changed_paths: Arc<Mutex<HashSet<PathBuf>>>,
    incremental_worker_running: Arc<AtomicBool>,
}

impl Global for Scanner {}

impl Scanner {
    pub fn new(scan_paths: Vec<PathBuf>) -> Self {
        Self {
            scan_paths,
            scan_lock: Arc::new(Mutex::new(())),
            pending_changed_paths: Arc::new(Mutex::new(HashSet::new())),
            incremental_worker_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn init(cx: &mut App) {
        let config = cx.global::<Config>().clone();
        let db = cx.global::<Database>().clone();
        let telemetry = cx.global::<Telemetry>().clone();

        let scan_paths = expand_scan_paths(&config.get().scan.paths);
        let scanner = Scanner::new(scan_paths);

        cx.set_global(scanner.clone());

        let scanner = Arc::new(scanner);
        let scanner_clone = scanner.clone();

        match MusicWatcher::new(scanner.clone(), Arc::new(db.clone())) {
            Ok((watcher, mut rx)) => {
                let db_clone = db.clone();
                let telemetry_clone = telemetry.clone();
                let config_clone = config.clone();

                tokio::spawn(async move {
                    let _watcher = watcher;
                    while let Some(stats) = rx.recv().await {
                        info!(
                            "Library scan completed - Scanned: {}, Added: {}, Updated: {}, Removed: {}",
                            stats.scanned, stats.added, stats.updated, stats.removed
                        );

                        if stats.scanned > 0 {
                            telemetry_clone.submit(&db_clone, &config_clone).await;
                        }
                    }
                });

                let db_clone = db.clone();
                let telemetry_clone = telemetry.clone();
                let config_clone = config.clone();

                tokio::spawn(async move {
                    let existing_song_count = db_clone.get_songs_count().await.unwrap_or(0);
                    info!(
                        "Starting initial library scan immediately (existing songs: {})...",
                        existing_song_count
                    );
                    match scanner_clone.scan(&db_clone).await {
                        Ok(stats) => {
                            info!(
                                "Initial scan complete - Scanned: {}, Added: {}, Updated: {}, Removed: {}",
                                stats.scanned, stats.added, stats.updated, stats.removed
                            );

                            if stats.scanned > 0 {
                                telemetry_clone.submit(&db_clone, &config_clone).await;
                            }
                        }
                        Err(e) => {
                            error!("Initial scan failed: {}", e);
                        }
                    }
                });
            }
            Err(e) => {
                error!("Failed to initialize music watcher: {}", e);
            }
        }
    }

    fn is_audio_file(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    fn read_metadata(path: &Path) -> Result<AudioMetadata> {
        AudioMetadata::from_path_with_options(path, false)
    }

    fn extract_image(path: &Path) -> Option<ImageData> {
        match extract_image_data(path) {
            Ok(image) => image,
            Err(e) => {
                debug!("Failed to extract image from {:?}: {}", path, e);
                None
            }
        }
    }

    fn should_process_file(
        existing_size: i64,
        existing_modified: i64,
        current_size: i64,
        current_modified: i64,
        force: bool,
    ) -> bool {
        force || existing_size != current_size || existing_modified != current_modified
    }

    fn normalize_path_for_matching(path: &Path) -> Option<PathBuf> {
        if path.as_os_str().is_empty() {
            return None;
        }

        if let Ok(canonical) = path.canonicalize() {
            return Some(canonical);
        }

        if path.is_absolute() {
            return Some(path.to_path_buf());
        }

        std::env::current_dir().ok().map(|cwd| cwd.join(path))
    }

    fn scan_roots_for_matching(&self) -> Vec<PathBuf> {
        self.scan_paths
            .iter()
            .filter_map(|root| Self::normalize_path_for_matching(root))
            .collect()
    }

    fn path_in_scan_roots(song_path: &Path, scan_roots: &[PathBuf]) -> bool {
        let Some(normalized_song_path) = Self::normalize_path_for_matching(song_path) else {
            return false;
        };

        scan_roots
            .iter()
            .any(|root| normalized_song_path.starts_with(root))
    }

    async fn collect_audio_files(&self) -> Result<Vec<PathBuf>> {
        let mut all_files = Vec::new();

        for root in &self.scan_paths {
            if !root.exists() || !root.is_dir() {
                continue;
            }

            let files = tokio::task::spawn_blocking({
                let root = root.clone();
                move || {
                    WalkDir::new(&root)
                        .follow_links(true)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file() && Scanner::is_audio_file(e.path()))
                        .map(|e| e.path().to_path_buf())
                        .collect::<Vec<_>>()
                }
            })
            .await
            .context("walkdir failed")?;

            all_files.extend(files);
        }

        Ok(all_files)
    }

    async fn collect_song_paths(&self, db: &Database) -> Result<Vec<String>> {
        let mut offset = 0i64;
        let limit = 500i64;
        let mut paths = Vec::new();

        loop {
            let songs = db.get_songs_paged(offset, limit).await?;
            if songs.is_empty() {
                break;
            }

            paths.extend(songs.into_iter().map(|song| song.file_path));
            offset += limit;
        }

        Ok(paths)
    }

    async fn collect_existing_track_state(
        &self,
        db: &Database,
    ) -> Result<HashMap<String, (i64, i64)>> {
        let mut offset = 0i64;
        let limit = 1000i64;
        let mut state_by_path = HashMap::new();

        loop {
            let songs = db.get_songs_paged(offset, limit).await?;
            if songs.is_empty() {
                break;
            }

            for song in songs {
                state_by_path.insert(song.file_path, (song.file_size, song.file_modified));
            }

            offset += limit;
        }

        Ok(state_by_path)
    }

    async fn remove_missing_tracks(&self, db: &Database) -> Result<usize> {
        let paths = self.collect_song_paths(db).await?;
        let scan_roots = self.scan_roots_for_matching();
        let mut stale_paths = Vec::new();
        let mut missing_candidates = 0usize;
        let mut out_of_scope_candidates = 0usize;

        for path_str in paths {
            let song_path = Path::new(&path_str);
            let missing_on_disk = !song_path.exists();
            let outside_scan_roots = !Self::path_in_scan_roots(song_path, &scan_roots);

            if missing_on_disk || outside_scan_roots {
                if missing_on_disk {
                    missing_candidates += 1;
                } else {
                    out_of_scope_candidates += 1;
                }
                stale_paths.push(path_str);
            }
        }

        if stale_paths.is_empty() {
            return Ok(0);
        }

        let mut removed = 0usize;
        for chunk in stale_paths.chunks(STALE_DELETE_BATCH_SIZE) {
            match db.delete_songs_by_paths(chunk).await {
                Ok(deleted) => {
                    removed += deleted;
                }
                Err(e) => {
                    error!(
                        "Failed to remove stale track batch (size {}): {}",
                        chunk.len(),
                        e
                    );
                }
            }
        }

        info!(
            "Removed {} stale tracks ({} missing, {} out-of-scope)",
            removed, missing_candidates, out_of_scope_candidates
        );

        Ok(removed)
    }

    async fn scan_with_options(&self, db: &Database, options: ScanOptions) -> Result<ScanStats> {
        let mut scanned = 0;
        let mut added = 0;
        let mut updated = 0;
        let mut skipped = 0;
        let mut failed = 0;

        let audio_files = self.collect_audio_files().await?;
        let existing_track_state = Arc::new(self.collect_existing_track_state(db).await?);
        let total_files = audio_files.len();
        info!("Found {} audio files to scan", total_files);
        info!(
            "Loaded {} existing tracks from database",
            existing_track_state.len()
        );

        let tracks_stream = stream::iter(audio_files)
            .map({
                let force = options.force;
                let existing_track_state = existing_track_state.clone();
                move |path: PathBuf| {
                    let existing_track_state = existing_track_state.clone();
                    async move {
                        let file_path = path.to_string_lossy().to_string();

                        let file_meta = match tokio::task::spawn_blocking({
                            let path = path.clone();
                            move || std::fs::metadata(&path)
                        })
                        .await
                        {
                            Ok(Ok(meta)) => meta,
                            Ok(Err(e)) => {
                                warn!("Failed to read metadata for {}: {}", file_path, e);
                                return Some((None, false, true));
                            }
                            Err(e) => {
                                warn!("Task failed for {}: {}", file_path, e);
                                return Some((None, false, true));
                            }
                        };

                        let file_size = file_meta.len() as i64;
                        let file_modified = file_meta
                            .modified()
                            .ok()
                            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);

                        let existing = existing_track_state.get(&file_path).copied();
                        let is_new = existing.is_none();

                        if let Some((existing_size, existing_modified)) = existing {
                            if !Self::should_process_file(
                                existing_size,
                                existing_modified,
                                file_size,
                                file_modified,
                                force,
                            ) {
                                return Some((None, false, false));
                            }
                        }

                        let metadata = match tokio::task::spawn_blocking({
                            let path = path.clone();
                            move || Self::read_metadata(&path)
                        })
                        .await
                        {
                            Ok(Ok(meta)) => meta,
                            Ok(Err(e)) => {
                                warn!("Failed to read metadata for {}: {}", file_path, e);
                                return Some((None, true, false));
                            }
                            Err(e) => {
                                warn!("Metadata task failed for {}: {}", file_path, e);
                                return Some((None, true, false));
                            }
                        };

                        let image_data = tokio::task::spawn_blocking({
                            let path = path.clone();
                            move || Self::extract_image(&path)
                        })
                        .await
                        .ok()
                        .flatten();

                        Some((
                            Some(ScannedTrack {
                                path,
                                file_size,
                                file_modified,
                                metadata,
                                image_data,
                            }),
                            is_new,
                            false,
                        ))
                    }
                }
            })
            .buffer_unordered(MAX_CONCURRENT_SCANS)
            .filter_map(|item| async move { item });

        futures::pin_mut!(tracks_stream);
        let mut seen_image_ids = HashSet::new();
        let mut artist_cache: HashMap<String, Cuid> = HashMap::new();
        let mut album_cache: HashMap<(String, Option<Cuid>), (Cuid, bool)> = HashMap::new();

        while let Some((track_opt, is_new, is_failed)) = tracks_stream.next().await {
            if is_failed {
                failed += 1;
                continue;
            }

            if track_opt.is_none() {
                skipped += 1;
                continue;
            }

            let track = track_opt.unwrap();
            if let Ok(()) = self
                .save_track(
                    db,
                    &track,
                    &mut seen_image_ids,
                    &mut artist_cache,
                    &mut album_cache,
                )
                .await
            {
                scanned += 1;
                if is_new {
                    added += 1;
                } else {
                    updated += 1;
                }
            } else {
                failed += 1;
            }

            if (scanned + skipped + failed) % 1000 == 0 {
                info!(
                    "Progress: {}/{} scanned, {} skipped, {} failed",
                    scanned + skipped + failed,
                    total_files,
                    skipped,
                    failed
                );
            }
        }

        let removed = self.remove_missing_tracks(db).await?;

        info!(
            "Scan complete: {} scanned, {} added, {} updated, {} skipped, {} failed, {} removed",
            scanned, added, updated, skipped, failed, removed
        );

        Ok(ScanStats {
            scanned,
            added,
            updated,
            removed,
        })
    }

    pub async fn scan(&self, db: &Database) -> Result<ScanStats> {
        let _scan_guard = self.scan_lock.lock().await;
        self.scan_with_options(db, ScanOptions::default()).await
    }

    pub async fn force_scan(&self, db: &Database) -> Result<ScanStats> {
        let _scan_guard = self.scan_lock.lock().await;
        self.scan_with_options(db, ScanOptions { force: true })
            .await
    }

    pub async fn process_changed_files(
        &self,
        db: &Database,
        changed_paths: Vec<PathBuf>,
    ) -> Result<ScanStats> {
        let _scan_guard = self.scan_lock.lock().await;
        self.process_changed_files_inner(db, changed_paths).await
    }

    async fn process_changed_files_inner(
        &self,
        db: &Database,
        changed_paths: Vec<PathBuf>,
    ) -> Result<ScanStats> {
        let mut scanned = 0;
        let mut added = 0;
        let mut updated = 0;
        let mut seen_image_ids = HashSet::new();
        let mut artist_cache: HashMap<String, Cuid> = HashMap::new();
        let mut album_cache: HashMap<(String, Option<Cuid>), (Cuid, bool)> = HashMap::new();

        for path in changed_paths {
            let path_clone = path.clone();

            if !path.exists() || !path.is_file() || !Self::is_audio_file(&path) {
                continue;
            }

            let file_meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Failed to read file metadata from {:?}: {}", path_clone, e);
                    continue;
                }
            };

            let file_size = file_meta.len() as i64;
            let file_modified = file_meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let existing = db
                .get_song_by_path(path_clone.to_string_lossy().as_ref())
                .await
                .ok()
                .flatten();

            let is_new = existing.is_none();

            if let Some(existing) = existing {
                if existing.file_size == file_size && existing.file_modified == file_modified {
                    continue;
                }
            }

            let metadata = match tokio::task::spawn_blocking({
                let path = path.clone();
                move || Self::read_metadata(&path)
            })
            .await
            {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => {
                    warn!("Failed to read metadata from {:?}: {}", path_clone, e);
                    continue;
                }
                Err(e) => {
                    error!("Task failed for {:?}: {}", path_clone, e);
                    continue;
                }
            };

            let image_data = tokio::task::spawn_blocking({
                let path = path.clone();
                move || Self::extract_image(&path)
            })
            .await
            .ok()
            .flatten();

            let track = ScannedTrack {
                path: path.clone(),
                file_size,
                file_modified,
                metadata,
                image_data,
            };

            match self
                .save_track(
                    db,
                    &track,
                    &mut seen_image_ids,
                    &mut artist_cache,
                    &mut album_cache,
                )
                .await
            {
                Ok(()) => {
                    scanned += 1;
                    if is_new {
                        added += 1;
                    } else {
                        updated += 1;
                    }
                    debug!("Updated track: {:?}", path_clone);
                }
                Err(e) => error!("Failed to save track {:?}: {}", path_clone, e),
            }
        }

        Ok(ScanStats {
            scanned,
            added,
            updated,
            removed: 0,
        })
    }

    pub async fn queue_changed_files(
        &self,
        db: Arc<Database>,
        tx: mpsc::Sender<ScanStats>,
        changed_paths: Vec<PathBuf>,
    ) {
        if changed_paths.is_empty() {
            return;
        }

        {
            let mut pending = self.pending_changed_paths.lock().await;
            pending.extend(changed_paths);
        }

        if self.incremental_worker_running.swap(true, Ordering::AcqRel) {
            return;
        }

        let scanner = self.clone();
        tokio::spawn(async move {
            loop {
                let batch: Vec<PathBuf> = {
                    let mut pending = scanner.pending_changed_paths.lock().await;
                    if pending.is_empty() {
                        Vec::new()
                    } else {
                        pending.drain().collect()
                    }
                };

                if batch.is_empty() {
                    scanner
                        .incremental_worker_running
                        .store(false, Ordering::Release);

                    let has_pending = {
                        let pending = scanner.pending_changed_paths.lock().await;
                        !pending.is_empty()
                    };

                    if has_pending
                        && !scanner
                            .incremental_worker_running
                            .swap(true, Ordering::AcqRel)
                    {
                        continue;
                    }

                    break;
                }

                info!(
                    "Processing coalesced incremental batch with {} files",
                    batch.len()
                );

                let _scan_guard = scanner.scan_lock.lock().await;
                match scanner.process_changed_files_inner(&db, batch).await {
                    Ok(stats) => {
                        info!(
                            "Incremental scan complete - Scanned: {}, Added: {}, Updated: {}, Removed: {}",
                            stats.scanned, stats.added, stats.updated, stats.removed
                        );
                        let _ = tx.send(stats).await;
                    }
                    Err(e) => {
                        error!("Incremental scan failed: {}", e);
                    }
                }
            }
        });
    }

    async fn save_track(
        &self,
        db: &Database,
        track: &ScannedTrack,
        seen_image_ids: &mut HashSet<String>,
        artist_cache: &mut HashMap<String, Cuid>,
        album_cache: &mut HashMap<(String, Option<Cuid>), (Cuid, bool)>,
    ) -> Result<()> {
        let path_str = track.path.to_string_lossy().to_string();
        let meta = &track.metadata;

        let image_id = if let Some(image) = &track.image_data {
            if seen_image_ids.insert(image.id.clone()) {
                db.upsert_image(&image.id, &image.data).await?;
            }
            Some(image.id.clone())
        } else {
            None
        };

        let artist_id = if let Some(artist_name) = &meta.artist {
            if let Some(artist_id) = artist_cache.get(artist_name) {
                Some(artist_id.clone())
            } else {
                let artist_id = db.upsert_artist(artist_name).await?;
                artist_cache.insert(artist_name.clone(), artist_id.clone());
                Some(artist_id)
            }
        } else {
            None
        };

        let album_id = if let Some(album_name) = &meta.album {
            let key = (album_name.clone(), artist_id.clone());
            if let Some((cached_album_id, has_image)) = album_cache.get_mut(&key) {
                if !*has_image && image_id.is_some() {
                    db.upsert_album(album_name, artist_id.as_ref(), image_id.as_deref())
                        .await?;
                    *has_image = true;
                }
                Some(cached_album_id.clone())
            } else {
                let album_id = db
                    .upsert_album(album_name, artist_id.as_ref(), image_id.as_deref())
                    .await?;
                album_cache.insert(key, (album_id.clone(), image_id.is_some()));
                Some(album_id)
            }
        } else {
            None
        };

        let title = meta.title.as_deref().unwrap_or("Unknown");
        let duration = meta.duration.as_secs() as i32;
        let track_number = meta.track_number.map(|n| n as i32);

        db.upsert_song(
            title,
            artist_id.as_ref(),
            album_id.as_ref(),
            &path_str,
            duration,
            track_number,
            meta.year,
            meta.genre.as_deref(),
            image_id.as_deref(),
            track.file_size,
            track.file_modified,
            meta.lufs,
        )
        .await?;

        debug!("Saved track: {:?}", track.path);
        Ok(())
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path[2..]);
        }
    }
    PathBuf::from(path)
}

pub fn expand_scan_paths(paths: &[String]) -> Vec<PathBuf> {
    paths.iter().map(|p| expand_tilde(p)).collect()
}

pub struct MusicWatcher {
    _scanner: Arc<Scanner>,
    _db: Arc<Database>,
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
}

impl MusicWatcher {
    pub fn new(
        scanner: Arc<Scanner>,
        db: Arc<Database>,
    ) -> Result<(Self, mpsc::Receiver<ScanStats>)> {
        let (tx, rx) = mpsc::channel(100);
        let scanner_clone = scanner.clone();
        let db_clone = db.clone();
        let runtime_handle = tokio::runtime::Handle::current();

        let mut debouncer = new_debouncer(
            Duration::from_secs(2),
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => {
                    let mut changed_audio_files: Vec<PathBuf> = Vec::new();
                    let mut removed_files: Vec<PathBuf> = Vec::new();

                    for event in events {
                        debug!("File event: {:?} - {:?}", event.kind, event.paths);

                        if matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        ) {
                            for path in &event.paths {
                                if Scanner::is_audio_file(path) {
                                    if path.exists() {
                                        changed_audio_files.push(path.clone());
                                    } else {
                                        removed_files.push(path.clone());
                                    }
                                }
                            }
                        }
                    }

                    let removed_files: Vec<PathBuf> = removed_files
                        .into_iter()
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();

                    for path in removed_files {
                        let path_str = path.to_string_lossy().to_string();
                        let db = db_clone.clone();
                        let tx = tx.clone();
                        runtime_handle.spawn(async move {
                            if db.delete_song_by_path(&path_str).await.is_ok() {
                                let stats = ScanStats {
                                    scanned: 0,
                                    added: 0,
                                    updated: 0,
                                    removed: 1,
                                };
                                let _ = tx.send(stats).await;
                            } else {
                                error!(
                                    "Failed to remove deleted track {}: (error logged above)",
                                    path_str
                                );
                            }
                        });
                    }

                    let changed_audio_files: Vec<PathBuf> = changed_audio_files
                        .into_iter()
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();

                    if !changed_audio_files.is_empty() {
                        info!(
                            "Detected {} changed audio files, processing incrementally",
                            changed_audio_files.len()
                        );
                        let scanner = scanner_clone.clone();
                        let db = db_clone.clone();
                        let tx = tx.clone();
                        runtime_handle.spawn(async move {
                            scanner
                                .queue_changed_files(db, tx, changed_audio_files)
                                .await;
                        });
                    }
                }
                Err(errors) => {
                    for error in errors {
                        error!("Filesystem watch error: {:?}", error);
                    }
                }
            },
        )
        .context("Failed to create filesystem watcher")?;

        for path in &scanner.scan_paths {
            debug!("Watching directory for changes: {:?}", path);
            debouncer
                .watch(path, RecursiveMode::Recursive)
                .with_context(|| format!("Failed to watch directory: {:?}", path))?;
        }

        let watcher = Self {
            _scanner: scanner,
            _db: db,
            _debouncer: debouncer,
        };

        Ok((watcher, rx))
    }
}
