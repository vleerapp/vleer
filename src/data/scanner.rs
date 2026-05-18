use anyhow::{Context, Result};
use futures::channel::mpsc;
use futures::lock::Mutex as AsyncMutex;
use futures::stream::{self, StreamExt};
use gpui::{App, BackgroundExecutor, Global};
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::UNIX_EPOCH;
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::data::config::Config;
use crate::data::db::repo::Database;
use crate::data::metadata::{AudioMetadata, ImageData, extract_image_data};
use crate::data::models::Cuid;
use crate::data::telemetry::Telemetry;
use crate::ui::components::context_menu::{BackgroundUiEvent, BackgroundUiNotifier};

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "aac", "aiff", "aif", "flac", "mp3", "mp4", "m4a", "mp4a", "ogg", "oga", "opus", "wav", "wv",
];
const MAX_CONCURRENT_SCANS: usize = 20;
const STALE_DELETE_BATCH_SIZE: usize = 400;

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ScanPhase {
    #[default]
    Idle,
    Scanning,
    Completed,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanProgress {
    pub current: usize,
    pub total: usize,
    pub phase: ScanPhase,
}

#[derive(Debug, Clone)]
pub struct ScannedTrack {
    pub path: PathBuf,
    pub file_size: i64,
    pub file_modified: i64,
    pub metadata: AudioMetadata,
    pub image_data: Option<ImageData>,
}

#[derive(Default)]
struct ScanOptions {
    force: bool,
}

#[derive(Clone)]
pub struct Scanner {
    scan_paths: Vec<PathBuf>,
    scan_lock: Arc<AsyncMutex<()>>,
    pending_changed_paths: Arc<AsyncMutex<HashSet<PathBuf>>>,
    incremental_worker_running: Arc<AtomicBool>,
    scan_progress: Arc<std::sync::Mutex<ScanProgress>>,
    executor: BackgroundExecutor,
}

impl Global for Scanner {}

impl Scanner {
    pub fn new(scan_paths: Vec<PathBuf>, executor: BackgroundExecutor) -> Self {
        Self {
            scan_paths,
            scan_lock: Arc::new(AsyncMutex::new(())),
            pending_changed_paths: Arc::new(AsyncMutex::new(HashSet::new())),
            incremental_worker_running: Arc::new(AtomicBool::new(false)),
            scan_progress: Arc::new(std::sync::Mutex::new(ScanProgress::default())),
            executor,
        }
    }

    fn update_scan_progress(&self, progress: ScanProgress) {
        if let Ok(mut state) = self.scan_progress.lock() {
            *state = progress;
        }
    }

    fn clear_scan_progress(&self) {
        self.update_scan_progress(ScanProgress::default());
    }

    pub fn get_scan_progress(&self) -> ScanProgress {
        self.scan_progress
            .lock()
            .map(|state| *state)
            .unwrap_or_default()
    }

    pub fn init(cx: &mut App) {
        let config = cx.global::<Config>().clone();
        let db = cx.global::<Database>().clone();
        let telemetry = cx.global::<Telemetry>().clone();
        let background_ui = cx.try_global::<BackgroundUiNotifier>().cloned();
        let executor = cx.background_executor().clone();

        let scan_paths = expand_scan_paths(&config.get().scan.paths);
        let scanner = Scanner::new(scan_paths, executor.clone());

        cx.set_global(scanner.clone());

        let scanner = Arc::new(scanner);
        let db_arc = Arc::new(db.clone());

        let exec = executor.clone();
        executor
            .spawn(async move {
                match MusicWatcher::new(scanner.clone(), db_arc, exec.clone()) {
                    Ok((watcher, mut rx)) => {
                        let db_clone = db.clone();
                        let telemetry_clone = telemetry.clone();
                        let config_clone = config.clone();
                        let background_ui_clone = background_ui.clone();

                        exec.clone()
                            .spawn(async move {
                                let _watcher = watcher;
                                while let Some(stats) = rx.next().await {
                                    info!(
                                        "Library scan completed - Scanned: {}, Added: {}, Updated: {}, Removed: {}",
                                        stats.scanned, stats.added, stats.updated, stats.removed
                                    );

                                    if stats.scanned > 0 {
                                        telemetry_clone.submit(&db_clone, &config_clone);
                                    }

                                    if (stats.added > 0 || stats.updated > 0 || stats.removed > 0)
                                        && let Some(background_ui) = &background_ui_clone
                                    {
                                        background_ui
                                            .notify(BackgroundUiEvent::LibraryDataChanged);
                                    }
                                }
                            })
                            .detach();

                        let db_clone = db.clone();
                        let telemetry_clone = telemetry.clone();
                        let config_clone = config.clone();
                        let background_ui_clone = background_ui.clone();

                        exec.spawn(async move {
                            let existing_song_count =
                                db_clone.get_songs_count(None).unwrap_or(0);
                            info!(
                                "Starting initial library scan (existing songs: {})...",
                                existing_song_count
                            );
                            match scanner.scan(&db_clone).await {
                                Ok(stats) => {
                                    info!(
                                        "Initial scan complete - Scanned: {}, Added: {}, Updated: {}, Removed: {}",
                                        stats.scanned, stats.added, stats.updated, stats.removed
                                    );

                                    if stats.scanned > 0 {
                                        telemetry_clone.submit(&db_clone, &config_clone);
                                    }

                                    if (stats.added > 0 || stats.updated > 0 || stats.removed > 0)
                                        && let Some(background_ui) = &background_ui_clone
                                    {
                                        background_ui
                                            .notify(BackgroundUiEvent::LibraryDataChanged);
                                    }
                                }
                                Err(e) => {
                                    error!("Initial scan failed: {}", e);
                                }
                            }
                        })
                        .detach();
                    }
                    Err(e) => {
                        error!("Failed to initialize music watcher: {}", e);
                    }
                }
            })
            .detach();
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

            let root = root.clone();
            let files = self
                .executor
                .spawn(async move {
                    WalkDir::new(&root)
                        .follow_links(true)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file() && Scanner::is_audio_file(e.path()))
                        .map(|e| e.path().to_path_buf())
                        .collect::<Vec<_>>()
                })
                .await;

            all_files.extend(files);
        }

        Ok(all_files)
    }

    fn collect_song_paths(&self, db: &Database) -> Result<Vec<String>> {
        db.get_song_paths()
    }

    fn collect_existing_track_state(&self, db: &Database) -> Result<HashMap<String, (i64, i64)>> {
        let states = db.get_song_file_states()?;
        Ok(states
            .into_iter()
            .map(|(path, size, modified)| (path, (size, modified)))
            .collect())
    }

    fn remove_missing_tracks(&self, db: &Database) -> Result<usize> {
        let paths = self.collect_song_paths(db)?;
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
            match db.delete_songs_by_paths(chunk) {
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
        let existing_track_state = Arc::new(self.collect_existing_track_state(db)?);
        let total_files = audio_files.len();

        self.update_scan_progress(ScanProgress {
            current: 0,
            total: total_files.max(1),
            phase: ScanPhase::Scanning,
        });

        info!("Found {} audio files to scan", total_files);
        info!(
            "Loaded {} existing tracks from database",
            existing_track_state.len()
        );

        let force = options.force;
        let executor = self.executor.clone();

        let tracks_stream = stream::iter(audio_files)
            .map({
                let existing_track_state = existing_track_state.clone();
                let executor = executor.clone();
                move |path: PathBuf| {
                    let existing_track_state = existing_track_state.clone();
                    let executor = executor.clone();
                    async move {
                        executor
                            .spawn(async move {
                                process_one_file(path, &existing_track_state, force)
                            })
                            .await
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
            if self
                .save_track(
                    db,
                    &track,
                    &mut seen_image_ids,
                    &mut artist_cache,
                    &mut album_cache,
                )
                .is_ok()
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

            let current = scanned + skipped + failed;
            if current % 1000 == 0 {
                info!(
                    "Progress: {}/{} scanned, {} skipped, {} failed",
                    current, total_files, skipped, failed
                );
            }

            self.update_scan_progress(ScanProgress {
                current,
                total: total_files.max(1),
                phase: ScanPhase::Scanning,
            });
        }

        let removed = self.remove_missing_tracks(db)?;

        info!(
            "Scan complete: {} scanned, {} added, {} updated, {} skipped, {} failed, {} removed",
            scanned, added, updated, skipped, failed, removed
        );

        self.update_scan_progress(ScanProgress {
            current: total_files.max(1),
            total: total_files.max(1),
            phase: ScanPhase::Completed,
        });
        self.clear_scan_progress();

        Ok(ScanStats {
            scanned,
            added,
            updated,
            removed,
        })
    }

    pub async fn scan(&self, db: &Database) -> Result<ScanStats> {
        let _scan_guard = self.scan_lock.lock().await;
        let result = self.scan_with_options(db, ScanOptions::default()).await;
        if result.is_err() {
            self.clear_scan_progress();
        }
        result
    }

    pub async fn force_scan(&self, db: &Database) -> Result<ScanStats> {
        let _scan_guard = self.scan_lock.lock().await;
        let result = self
            .scan_with_options(db, ScanOptions { force: true })
            .await;
        if result.is_err() {
            self.clear_scan_progress();
        }
        result
    }

    fn process_changed_files_inner(
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
                .ok()
                .flatten();

            let is_new = existing.is_none();

            if let Some(existing) = existing
                && existing.file_size == file_size
                && existing.file_modified == file_modified
            {
                continue;
            }

            let metadata = match Self::read_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Failed to read metadata from {:?}: {}", path_clone, e);
                    continue;
                }
            };

            let image_data = Self::extract_image(&path);

            let track = ScannedTrack {
                path: path.clone(),
                file_size,
                file_modified,
                metadata,
                image_data,
            };

            match self.save_track(
                db,
                &track,
                &mut seen_image_ids,
                &mut artist_cache,
                &mut album_cache,
            ) {
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
        tx: mpsc::UnboundedSender<ScanStats>,
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
        self.executor
            .spawn(async move {
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
                    match scanner.process_changed_files_inner(&db, batch) {
                        Ok(stats) => {
                            info!(
                                "Incremental scan complete - Scanned: {}, Added: {}, Updated: {}, Removed: {}",
                                stats.scanned, stats.added, stats.updated, stats.removed
                            );
                            let _ = tx.unbounded_send(stats);
                        }
                        Err(e) => {
                            error!("Incremental scan failed: {}", e);
                        }
                    }
                }
            })
            .detach();
    }

    fn save_track(
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
                db.upsert_image(&image.id, &image.data)?;
            }
            Some(image.id.clone())
        } else {
            None
        };

        let artist_id = if let Some(artist_name) = &meta.artist {
            if let Some(artist_id) = artist_cache.get(artist_name) {
                Some(artist_id.clone())
            } else {
                let artist_id = db.upsert_artist(artist_name)?;
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
                    db.upsert_album(album_name, artist_id.as_ref(), image_id.as_deref())?;
                    *has_image = true;
                }
                Some(cached_album_id.clone())
            } else {
                let album_id =
                    db.upsert_album(album_name, artist_id.as_ref(), image_id.as_deref())?;
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
        )?;

        debug!("Saved track: {:?}", track.path);
        Ok(())
    }
}

fn process_one_file(
    path: PathBuf,
    existing_track_state: &HashMap<String, (i64, i64)>,
    force: bool,
) -> Option<(Option<ScannedTrack>, bool, bool)> {
    let file_path = path.to_string_lossy().to_string();

    let file_meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) => {
            warn!("Failed to read metadata for {}: {}", file_path, e);
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

    if let Some((existing_size, existing_modified)) = existing
        && !Scanner::should_process_file(
            existing_size,
            existing_modified,
            file_size,
            file_modified,
            force,
        )
    {
        return Some((None, false, false));
    }

    let metadata = match Scanner::read_metadata(&path) {
        Ok(meta) => meta,
        Err(e) => {
            warn!("Failed to read metadata for {}: {}", file_path, e);
            return Some((None, true, false));
        }
    };

    let image_data = Scanner::extract_image(&path);

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

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
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
        executor: BackgroundExecutor,
    ) -> Result<(Self, mpsc::UnboundedReceiver<ScanStats>)> {
        let (tx, rx) = mpsc::unbounded::<ScanStats>();
        let scanner_clone = scanner.clone();
        let db_clone = db.clone();
        let exec_clone = executor.clone();

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
                        exec_clone
                            .spawn(async move {
                                if db.delete_song_by_path(&path_str).is_ok() {
                                    let stats = ScanStats {
                                        scanned: 0,
                                        added: 0,
                                        updated: 0,
                                        removed: 1,
                                    };
                                    let _ = tx.unbounded_send(stats);
                                } else {
                                    error!(
                                        "Failed to remove deleted track {}: (error logged above)",
                                        path_str
                                    );
                                }
                            })
                            .detach();
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
                        exec_clone
                            .spawn(async move {
                                scanner.queue_changed_files(db, tx, changed_audio_files).await;
                            })
                            .detach();
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
