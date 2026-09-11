use anyhow::{Context, Result};
use futures::StreamExt;
use futures::channel::mpsc;
use futures::lock::Mutex as AsyncMutex;
use gpui::{App, BackgroundExecutor, Global};
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use std::time::{Instant, UNIX_EPOCH};
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::data::config::Config;
use crate::data::db::repo::{BatchTrack, Database, ScanCache};
use crate::data::metadata::{AudioMetadata, read_metadata};
use crate::data::telemetry::Telemetry;
use crate::ui::components::context_menu::{BackgroundUiEvent, BackgroundUiNotifier};
use crate::ui::layout::navbar;

type FsWatcher = Debouncer<notify::RecommendedWatcher, RecommendedCache>;

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "aac", "aiff", "aif", "flac", "mp3", "mp4", "m4a", "mp4a", "ogg", "oga", "opus", "wav", "wv",
];
/// Files handed to one parallel read job. Small so the write of one batch
/// overlaps the read of the next at a fine grain.
const READ_CHUNK_SIZE: usize = 256;

/// Tracks gathered before opening a transaction.
///
/// Deliberately much larger than the read chunk. Committing is not free, and
/// measured against a real library it dominated the write: a commit per 256
/// files meant hundreds of them, each costing far more than the rows it
/// carried. The two numbers used to be one, which tied how often we commit to
/// how finely reads pipeline -- unrelated concerns that want opposite answers.
const DB_FLUSH_SIZE: usize = 4096;
const UI_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub missing: usize,
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
    pub recheck_image: bool,
}

#[derive(Default)]
struct ScanOptions {
    force: bool,
}

#[derive(Clone)]
pub struct Scanner {
    scan_paths: Arc<std::sync::RwLock<Vec<PathBuf>>>,
    watcher: Arc<std::sync::Mutex<Option<FsWatcher>>>,
    scan_lock: Arc<AsyncMutex<()>>,
    cancel_flag: Arc<AtomicBool>,
    scan_generation: Arc<AtomicU64>,
    pending_changed_paths: Arc<AsyncMutex<HashSet<PathBuf>>>,
    incremental_worker_running: Arc<AtomicBool>,

    warm_cancel: Arc<AtomicBool>,
    executor: BackgroundExecutor,
    background_ui: Option<BackgroundUiNotifier>,
}

impl Global for Scanner {}

impl Scanner {
    pub fn new(
        scan_paths: Vec<PathBuf>,
        executor: BackgroundExecutor,
        background_ui: Option<BackgroundUiNotifier>,
    ) -> Self {
        Self {
            scan_paths: Arc::new(std::sync::RwLock::new(scan_paths)),
            watcher: Arc::new(std::sync::Mutex::new(None)),
            scan_lock: Arc::new(AsyncMutex::new(())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            scan_generation: Arc::new(AtomicU64::new(0)),
            pending_changed_paths: Arc::new(AsyncMutex::new(HashSet::new())),
            incremental_worker_running: Arc::new(AtomicBool::new(false)),
            warm_cancel: Arc::new(AtomicBool::new(false)),
            executor,
            background_ui,
        }
    }

    fn get_scan_paths(&self) -> Vec<PathBuf> {
        self.scan_paths
            .read()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    fn install_watcher(&self, watcher: FsWatcher) {
        if let Ok(mut slot) = self.watcher.lock() {
            *slot = Some(watcher);
        }
    }

    pub fn update_scan_paths(&self, new_paths: Vec<PathBuf>) {
        let old_paths: Vec<PathBuf> = {
            let mut paths = match self.scan_paths.write() {
                Ok(p) => p,
                Err(_) => return,
            };
            let old = paths.clone();
            *paths = new_paths.clone();
            old
        };

        if old_paths == new_paths {
            return;
        }

        if let Ok(mut watcher_slot) = self.watcher.lock()
            && let Some(watcher) = watcher_slot.as_mut()
        {
            for old in &old_paths {
                if !new_paths.contains(old)
                    && let Err(e) = watcher.unwatch(old)
                {
                    warn!("Failed to unwatch {:?}: {}", old, e);
                }
            }
            for new_p in &new_paths {
                if !old_paths.contains(new_p)
                    && new_p.exists()
                    && let Err(e) = watcher.watch(new_p, RecursiveMode::Recursive)
                {
                    warn!("Failed to watch {:?}: {}", new_p, e);
                }
            }
        }
    }

    fn request_cancel(&self) {
        self.cancel_flag.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Acquire)
    }

    fn update_scan_progress(&self, progress: ScanProgress) {
        use crate::status::StatusColor;
        let reporter = navbar::status();
        match progress.phase {
            ScanPhase::Idle => reporter.clear("library.scan"),
            ScanPhase::Completed => reporter.set(
                "library.scan",
                "Scanning: done",
                Some(1.0),
                StatusColor::Accent,
            ),
            ScanPhase::Scanning => {
                if progress.total == 0 {
                    reporter.clear("library.scan");
                } else {
                    let ratio = (progress.current as f32 / progress.total as f32).clamp(0.0, 1.0);
                    reporter.set(
                        "library.scan",
                        format!(
                            "Scanning: {}/{} - {:.0}%",
                            progress.current,
                            progress.total,
                            ratio * 100.0
                        ),
                        Some(ratio),
                        StatusColor::Accent,
                    );
                }
            }
        }
    }

    fn clear_scan_progress(&self) {
        self.update_scan_progress(ScanProgress::default());
    }

    pub fn init(cx: &mut App) {
        let config = cx.global::<Config>().clone();
        let db = cx.global::<Database>().clone();
        let telemetry = cx.global::<Telemetry>().clone();
        let background_ui = cx.try_global::<BackgroundUiNotifier>().cloned();
        let executor = cx.background_executor().clone();

        let scan_paths = expand_scan_paths(&config.get().scan.paths);
        let scanner = Scanner::new(scan_paths, executor.clone(), background_ui.clone());

        cx.set_global(scanner.clone());

        let scanner = Arc::new(scanner);
        let db_arc = Arc::new(db.clone());

        let scanner_for_observe = scanner.clone();
        let db_for_observe = db.clone();
        let background_ui_for_observe = background_ui.clone();
        let last_paths: Arc<std::sync::Mutex<Vec<PathBuf>>> =
            Arc::new(std::sync::Mutex::new(scanner.get_scan_paths()));
        cx.observe_global::<Config>(move |cx| {
            let new_paths = expand_scan_paths(&cx.global::<Config>().get().scan.paths);
            let (changed, removed_paths) = {
                let mut last = match last_paths.lock() {
                    Ok(l) => l,
                    Err(_) => return,
                };
                if *last == new_paths {
                    (false, Vec::new())
                } else {
                    let removed: Vec<PathBuf> = last
                        .iter()
                        .filter(|p| !new_paths.contains(p))
                        .cloned()
                        .collect();
                    *last = new_paths.clone();
                    (true, removed)
                }
            };
            if !changed {
                return;
            }

            info!("Scan paths changed, updating watcher and rescanning");
            scanner_for_observe.update_scan_paths(new_paths);

            let scanner_clone = scanner_for_observe.clone();
            let db_clone = db_for_observe.clone();
            let background_ui_clone = background_ui_for_observe.clone();
            cx.background_executor()
                .spawn(async move {
                    let observer_start = std::time::Instant::now();
                    if !removed_paths.is_empty() {
                        warn!(
                            "{} scan path(s) removed from config — songs under them were kept in the library. Remove them manually in Settings if no longer needed: {:?}",
                            removed_paths.len(),
                            removed_paths
                        );
                    }

                    let scan_start = std::time::Instant::now();
                    match scanner_clone.scan(&db_clone).await {
                        Ok(stats) => {
                            info!(
                                "Path-change rescan in {:?} (observer total {:?}) - Scanned: {}, Added: {}, Updated: {}, Missing: {}",
                                scan_start.elapsed(),
                                observer_start.elapsed(),
                                stats.scanned, stats.added, stats.updated, stats.missing
                            );
                            if (stats.added > 0 || stats.updated > 0 || stats.removed > 0)
                                && let Some(background_ui) = &background_ui_clone
                            {
                                background_ui.notify(BackgroundUiEvent::LibraryDataChanged);
                            }
                        }
                        Err(e) => {
                            error!("Path-change rescan failed: {}", e);
                        }
                    }
                })
                .detach();
        })
        .detach();

        let exec = executor.clone();
        executor
            .spawn(async move {
                match MusicWatcher::install(scanner.clone(), db_arc, exec.clone()) {
                    Ok(mut rx) => {
                        let db_clone = db.clone();
                        let telemetry_clone = telemetry.clone();
                        let config_clone = config.clone();
                        let background_ui_clone = background_ui.clone();
                        exec.clone()
                            .spawn(async move {
                                while let Some(stats) = rx.next().await {
                                    info!(
                                        "Library scan completed - Scanned: {}, Added: {}, Updated: {}, Missing: {}",
                                        stats.scanned, stats.added, stats.updated, stats.missing
                                    );

                                    if stats.missing > 0 {
                                        navbar::status().set(
                                            "scanner.missing",
                                            format!("{} song{} missing from disk", stats.missing, if stats.missing == 1 { "" } else { "s" }),
                                            None,
                                            crate::status::StatusColor::Warning,
                                        );
                                    } else {
                                        navbar::status().clear("scanner.missing");
                                    }

                                    if stats.scanned > 0 || stats.removed > 0 {
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
                                match scanner.scan(&db_clone).await {
                                Ok(stats) => {

                                    if stats.missing > 0 {
                                        navbar::status().set(
                                            "scanner.missing",
                                            format!("{} song{} missing from disk", stats.missing, if stats.missing == 1 { "" } else { "s" }),
                                            None,
                                            crate::status::StatusColor::Warning,
                                        );
                                    } else {
                                        navbar::status().clear("scanner.missing");
                                    }

                                    if stats.scanned > 0 || stats.removed > 0 {
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
            .map(|ext| {
                SUPPORTED_EXTENSIONS
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            })
            .unwrap_or(false)
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

    async fn collect_audio_files(&self) -> Result<Vec<PathBuf>> {
        let mut all_files = Vec::new();
        let scan_paths = self.get_scan_paths();

        for root in scan_paths {
            if self.is_cancelled() {
                break;
            }
            if !root.exists() || !root.is_dir() {
                continue;
            }

            let cancel_flag = self.cancel_flag.clone();
            let files = self
                .executor
                .spawn(async move {
                    let mut files = Vec::new();
                    for entry in WalkDir::new(&root)
                        .follow_links(false)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        if cancel_flag.load(Ordering::Acquire) {
                            break;
                        }
                        if entry.file_type().is_file() && Scanner::is_audio_file(entry.path()) {
                            files.push(entry.path().to_path_buf());
                        }
                    }
                    files
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

    fn find_missing_songs(
        &self,
        db: &Database,
        scanned_files: &HashSet<String>,
    ) -> Result<Vec<String>> {
        let paths = self.collect_song_paths(db)?;
        Ok(paths
            .into_iter()
            .filter(|p| !scanned_files.contains(p))
            .collect())
    }

    fn remove_missing_songs(
        &self,
        db: &Database,
        scanned_files: &HashSet<String>,
    ) -> Result<usize> {
        let stale_paths = self.find_missing_songs(db, scanned_files)?;

        if stale_paths.is_empty() {
            return Ok(0);
        }

        warn!(
            "{} songs are missing from disk but were kept in the library. Remove them manually in Settings if no longer needed.",
            stale_paths.len()
        );
        for p in &stale_paths {
            warn!("Missing: {}", p);
        }

        Ok(stale_paths.len())
    }

    async fn scan_with_options(&self, db: &Database, options: ScanOptions) -> Result<ScanStats> {
        let mut scanned = 0;
        let mut added = 0;
        let mut updated = 0;
        let mut skipped = 0;
        let mut failed = 0;

        let walk_started = Instant::now();
        let audio_files = self.collect_audio_files().await?;
        let walk_time = walk_started.elapsed();
        if self.is_cancelled() {
            info!("Scan cancelled before processing files");
            self.clear_scan_progress();
            return Ok(ScanStats {
                scanned: 0,
                added: 0,
                updated: 0,
                removed: 0,
                missing: 0,
            });
        }
        let state_started = Instant::now();
        let existing_track_state = Arc::new(self.collect_existing_track_state(db)?);
        let state_time = state_started.elapsed();
        let total_files = audio_files.len();
        let scanned_files: HashSet<String> = audio_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        self.update_scan_progress(ScanProgress {
            current: 0,
            total: total_files.max(1),
            phase: ScanPhase::Scanning,
        });

        let force = options.force;
        crate::data::db::repo::write_profile::reset();
        let files = Arc::new(audio_files);
        let chunk_count = files.len().div_ceil(READ_CHUNK_SIZE);

        let read_chunk = |index: usize| {
            let files = files.clone();
            let state = existing_track_state.clone();
            let (tx, rx) = futures::channel::oneshot::channel();
            rayon::spawn(move || {
                let start = index * READ_CHUNK_SIZE;
                let end = (start + READ_CHUNK_SIZE).min(files.len());
                let outcomes: Vec<_> = files[start..end]
                    .par_iter()
                    .map(|path| process_one_file(path, &state, force))
                    .collect();
                let _ = tx.send(outcomes);
            });
            rx
        };

        let mut read_time = Duration::ZERO;
        let mut write_time = Duration::ZERO;
        let mut cache = ScanCache::default();
        let mut pending: Vec<(ScannedTrack, bool)> = Vec::with_capacity(DB_FLUSH_SIZE);
        let mut last_ui_refresh = Instant::now();
        let mut last_progress = Instant::now();
        let mut dirty = false;
        let mut cancelled = false;

        let mut inflight = (chunk_count > 0).then(|| read_chunk(0));

        for index in 0..chunk_count {
            let Some(rx) = inflight.take() else {
                break;
            };

            let waiting = Instant::now();
            let outcomes = rx.await.unwrap_or_default();
            read_time += waiting.elapsed();

            if self.is_cancelled() {
                cancelled = true;
                break;
            }

            if index + 1 < chunk_count {
                inflight = Some(read_chunk(index + 1));
            }

            for outcome in outcomes {
                match outcome {
                    Some((_, _, true)) => failed += 1,
                    Some((Some(track), is_new, _)) => pending.push((track, is_new)),
                    Some((None, _, _)) => skipped += 1,
                    None => skipped += 1,
                }
            }

            if pending.len() >= DB_FLUSH_SIZE {
                let flush_started = Instant::now();
                match self.flush_batch(db, &pending, &mut cache) {
                    Ok((batch_added, batch_updated)) => {
                        scanned += pending.len();
                        added += batch_added;
                        updated += batch_updated;
                        dirty = true;
                    }
                    Err(e) => {
                        error!("Failed to write scan batch: {}", e);
                        failed += pending.len();
                    }
                }
                write_time += flush_started.elapsed();
                pending.clear();
            }

            let current = scanned + pending.len() + skipped + failed;

            if dirty
                && last_ui_refresh.elapsed() >= UI_REFRESH_INTERVAL
                && let Some(ref ui) = self.background_ui
            {
                ui.notify(BackgroundUiEvent::LibraryDataChanged);
                last_ui_refresh = Instant::now();
                dirty = false;
            }

            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                self.update_scan_progress(ScanProgress {
                    current,
                    total: total_files.max(1),
                    phase: ScanPhase::Scanning,
                });
                last_progress = Instant::now();
            }
        }

        if !pending.is_empty() {
            let flush_started = Instant::now();
            match self.flush_batch(db, &pending, &mut cache) {
                Ok((batch_added, batch_updated)) => {
                    scanned += pending.len();
                    added += batch_added;
                    updated += batch_updated;
                }
                Err(e) => {
                    error!("Failed to write final scan batch: {}", e);
                    failed += pending.len();
                }
            }
            write_time += flush_started.elapsed();
            pending.clear();
        }

        let index_started = Instant::now();
        if added > 0 || updated > 0 {
            db.rebuild_search_index();
        }
        let index_time = index_started.elapsed();

        if cancelled || self.is_cancelled() {
            info!(
                "Scan cancelled mid-progress: {} scanned, {} added, {} updated, {} skipped, {} failed",
                scanned, added, updated, skipped, failed
            );
            self.clear_scan_progress();
            return Ok(ScanStats {
                scanned,
                added,
                updated,
                removed: 0,
                missing: 0,
            });
        }

        let missing_started = Instant::now();
        let missing = self.remove_missing_songs(db, &scanned_files)?;
        let missing_time = missing_started.elapsed();

        info!(
            "Scan complete: {} scanned, {} added, {} updated, {} skipped, {} failed, {} missing",
            scanned, added, updated, skipped, failed, missing
        );
        info!(
            "Scan phases: walk {:?}, existing state {:?}, read {:?}, write {:?}, search index {:?}, missing {:?}",
            walk_time, state_time, read_time, write_time, index_time, missing_time
        );
        let (lock, albums, songs, artists, genres, commit) =
            crate::data::db::repo::write_profile::snapshot();
        let (commits, wal_mb) = crate::data::db::repo::write_profile::commit_stats();
        info!(
            "Write breakdown: waiting for connection {lock:?}, albums {albums:?}, songs \
             {songs:?}, artists {artists:?}, genres {genres:?}, commit {commit:?} over \
             {commits} transactions, wal {wal_mb}MB"
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
            removed: 0,
            missing,
        })
    }

    async fn run_scan(&self, db: &Database, options: ScanOptions) -> Result<ScanStats> {
        let my_gen = self
            .scan_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.request_cancel();

        let _scan_guard = self.scan_lock.lock().await;

        if self.scan_generation.load(Ordering::Acquire) != my_gen {
            debug!("Scan request {} superseded before start", my_gen);
            return Ok(ScanStats {
                scanned: 0,
                added: 0,
                updated: 0,
                removed: 0,
                missing: 0,
            });
        }

        self.warm_cancel.store(true, Ordering::Release);

        self.cancel_flag.store(false, Ordering::Release);
        let result = self.scan_with_options(db, options).await;
        if result.is_err() {
            self.clear_scan_progress();
        }
        if result.is_ok() && !self.is_cancelled() {
            self.spawn_image_warm_pass(db);
        }
        result
    }

    fn spawn_image_warm_pass(&self, db: &Database) {
        self.warm_cancel.store(false, Ordering::Release);

        let db = db.clone();
        let cancel = self.warm_cancel.clone();

        self.executor
            .spawn(async move {
                let started = Instant::now();
                let resolved = crate::data::images::warm_images(db, cancel).await;

                if resolved > 0 {
                    info!(
                        "Resolved {} cover image(s) in {:?}",
                        resolved,
                        started.elapsed()
                    );
                }
            })
            .detach();
    }

    pub async fn scan(&self, db: &Database) -> Result<ScanStats> {
        self.run_scan(db, ScanOptions::default()).await
    }

    pub async fn force_scan(&self, db: &Database) -> Result<ScanStats> {
        self.run_scan(db, ScanOptions { force: true }).await
    }

    fn process_changed_files_inner(
        &self,
        db: &Database,
        changed_paths: Vec<PathBuf>,
    ) -> Result<ScanStats> {
        let mut cache = ScanCache::default();
        let mut pending: Vec<(ScannedTrack, bool)> = Vec::new();

        for path in changed_paths {
            if !path.is_file() || !Self::is_audio_file(&path) {
                continue;
            }

            let file_meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    warn!("Failed to read file metadata from {:?}: {}", path, e);
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
                .get_song_by_path(path.to_string_lossy().as_ref())
                .ok()
                .flatten();

            let is_new = existing.is_none();

            if let Some(existing) = existing
                && existing.file_size == file_size
                && existing.file_modified == file_modified
            {
                continue;
            }

            let metadata = match read_metadata(&path) {
                Ok(metadata) => metadata,
                Err(e) => {
                    warn!("Failed to read metadata from {:?}: {}", path, e);
                    continue;
                }
            };

            pending.push((
                ScannedTrack {
                    path,
                    file_size,
                    file_modified,
                    metadata,

                    recheck_image: true,
                },
                is_new,
            ));
        }

        let (added, updated) = self.flush_batch(db, &pending, &mut cache)?;
        if added > 0 || updated > 0 {
            db.rebuild_search_index();
        }

        Ok(ScanStats {
            scanned: pending.len(),
            added,
            updated,
            removed: 0,
            missing: 0,
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
                                "Incremental scan complete - Scanned: {}, Added: {}, Updated: {}, Missing: {}",
                                stats.scanned, stats.added, stats.updated, stats.missing
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

    fn flush_batch(
        &self,
        db: &Database,
        tracks: &[(ScannedTrack, bool)],
        cache: &mut ScanCache,
    ) -> Result<(usize, usize)> {
        if tracks.is_empty() {
            return Ok((0, 0));
        }

        let paths: Vec<String> = tracks
            .iter()
            .map(|(track, _)| track.path.to_string_lossy().into_owned())
            .collect();
        let artists: Vec<Vec<&str>> = tracks
            .iter()
            .map(|(track, _)| track.metadata.artists.iter().map(|s| s.as_str()).collect())
            .collect();
        let genres: Vec<Vec<&str>> = tracks
            .iter()
            .map(|(track, _)| track.metadata.genres.iter().map(|s| s.as_str()).collect())
            .collect();

        let batch: Vec<BatchTrack<'_>> = tracks
            .iter()
            .enumerate()
            .map(|(i, (track, _))| {
                let meta = &track.metadata;
                BatchTrack {
                    title: meta.title.as_deref().unwrap_or("Unknown"),
                    artists: &artists[i],
                    genres: &genres[i],
                    album: meta.album.as_deref(),
                    file_path: &paths[i],
                    duration: meta.duration.as_secs() as i32,
                    track_number: meta.track_number.map(|n| n as i32),
                    year: meta.year,
                    recheck_image: track.recheck_image,
                    file_size: track.file_size,
                    file_modified: track.file_modified,
                    lufs: meta.lufs,
                }
            })
            .collect();

        db.upsert_tracks_batch(&batch, cache)?;

        let added = tracks.iter().filter(|(_, is_new)| *is_new).count();
        Ok((added, tracks.len() - added))
    }
}

fn process_one_file(
    path: &Path,
    existing_track_state: &HashMap<String, (i64, i64)>,
    force: bool,
) -> Option<(Option<ScannedTrack>, bool, bool)> {
    let file_meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) => {
            warn!("Failed to read metadata for {:?}: {}", path, e);
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

    let existing = existing_track_state
        .get(path.to_string_lossy().as_ref())
        .copied();
    let is_new = existing.is_none();

    let identity_changed = match existing {
        Some((existing_size, existing_modified)) => {
            existing_size != file_size || existing_modified != file_modified
        }
        None => true,
    };

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

    let metadata = match read_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) => {
            warn!("Failed to read metadata for {:?}: {}", path, e);
            return Some((None, false, true));
        }
    };

    Some((
        Some(ScannedTrack {
            path: path.to_path_buf(),
            file_size,
            file_modified,
            metadata,
            recheck_image: identity_changed,
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

pub struct MusicWatcher;

impl MusicWatcher {
    pub fn install(
        scanner: Arc<Scanner>,
        db: Arc<Database>,
        executor: BackgroundExecutor,
    ) -> Result<mpsc::UnboundedReceiver<ScanStats>> {
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
                    let mut removed_dirs: Vec<String> = Vec::new();

                    for event in events {
                        debug!("File event: {:?} - {:?}", event.kind, event.paths);

                        if matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        ) {
                            for path in &event.paths {
                                if path.exists() {
                                    if Scanner::is_audio_file(path) {
                                        changed_audio_files.push(path.clone());
                                    }
                                } else if Scanner::is_audio_file(path) {
                                    removed_files.push(path.clone());
                                } else {
                                    removed_dirs.push(path.to_string_lossy().to_string());
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
                        warn!("File removed from disk, keeping in library: {}", path_str);
                        let _ = tx;
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
                                scanner
                                    .queue_changed_files(db, tx, changed_audio_files)
                                    .await;
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

        for path in scanner.get_scan_paths() {
            if !path.exists() {
                debug!("Skipping non-existent watch path: {:?}", path);
                continue;
            }
            debug!("Watching directory for changes: {:?}", path);
            debouncer
                .watch(&path, RecursiveMode::Recursive)
                .with_context(|| format!("Failed to watch directory: {:?}", path))?;
        }

        scanner.install_watcher(debouncer);

        let sync_paths = scanner.get_scan_paths();
        if let Ok(mut watcher_slot) = scanner.watcher.lock()
            && let Some(w) = watcher_slot.as_mut()
        {
            for path in &sync_paths {
                if path.exists() {
                    let _ = w.watch(path, RecursiveMode::Recursive);
                }
            }
        }

        let _ = db;

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::config::ParseOptions;
    use lofty::file::AudioFile;
    use lofty::probe::Probe;
    use std::io::BufReader;
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    fn bench_files() -> Vec<PathBuf> {
        let root = std::env::var("VLEER_BENCH_DIR").expect("set VLEER_BENCH_DIR");
        WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && Scanner::is_audio_file(e.path()))
            .map(|e| e.path().to_path_buf())
            .collect()
    }

    fn timed(paths: &[PathBuf], threads: usize, f: fn(&Path)) -> std::time::Duration {
        let next = AtomicUsize::new(0);
        let t = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= paths.len() {
                            break;
                        }
                        f(&paths[i]);
                    }
                });
            }
        });
        t.elapsed()
    }

    fn stat_only(path: &Path) {
        let _ = std::fs::metadata(path).map(|m| m.len());
    }

    fn read_tags(path: &Path, art: bool) {
        read_with(path, ParseOptions::new().read_cover_art(art));
    }

    fn read_with(path: &Path, options: ParseOptions) {
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        let probe = Probe::new(BufReader::with_capacity(64 * 1024, file));
        if let Ok(guessed) = probe.guess_file_type() {
            let _ = guessed
                .options(options)
                .read()
                .map(|f| f.properties().duration());
        }
    }

    fn read_no_art(path: &Path) {
        read_tags(path, false);
    }

    fn read_no_properties(path: &Path) {
        read_with(
            path,
            ParseOptions::new()
                .read_cover_art(false)
                .read_properties(false),
        );
    }

    fn read_with_art(path: &Path) {
        read_tags(path, true);
    }

    #[test]
    #[ignore]
    fn bench_scan_phases() {
        let paths = bench_files();
        println!("\n{} audio files\n", paths.len());
        let only: Option<String> = std::env::var("VLEER_BENCH_ONLY").ok();
        let threads_list: Vec<usize> = std::env::var("VLEER_BENCH_THREADS")
            .ok()
            .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
            .unwrap_or_else(|| vec![1, 8, 32]);

        for (name, f) in [
            ("stat only     ", stat_only as fn(&Path)),
            ("tags, no props", read_no_properties as fn(&Path)),
            ("tags, no art  ", read_no_art as fn(&Path)),
            ("tags + art    ", read_with_art as fn(&Path)),
        ] {
            if let Some(only) = &only
                && !name.contains(only.as_str())
            {
                continue;
            }
            for threads in threads_list.iter().copied() {
                println!("{name} t={threads:<3} {:>12.2?}", timed(&paths, threads, f));
            }
            println!();
        }
    }
}
