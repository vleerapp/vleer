use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use gpui::App;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::UNIX_EPOCH;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::data::config::Config;
use crate::data::db::repo::Database;
use crate::data::metadata::{AudioMetadata, ImageData, extract_image_data};
use crate::data::telemetry::Telemetry;

const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "m4a", "aac", "wav", "mp1", "mp2"];
const MAX_CONCURRENT_SCANS: usize = 16;

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub scanned: usize,
}

#[derive(Debug, Clone)]
pub struct ScannedTrack {
    pub path: PathBuf,
    pub file_size: i64,
    pub file_modified: i64,
    pub metadata: AudioMetadata,
    pub image_data: Option<ImageData>,
}

pub struct Scanner {
    scan_paths: Vec<PathBuf>,
}

impl Scanner {
    pub fn new(scan_paths: Vec<PathBuf>) -> Self {
        Self { scan_paths }
    }

    pub fn init(cx: &mut App) {
        let config = cx.global::<Config>().clone();
        let db = cx.global::<Database>().clone();
        let telemetry = cx.global::<Telemetry>().clone();

        let scan_paths = expand_scan_paths(&config.get().scan.paths);
        let scanner = Arc::new(Scanner::new(scan_paths));
        let scanner_clone = scanner.clone();

        match MusicWatcher::new(scanner.clone(), Arc::new(db.clone())) {
            Ok((watcher, mut rx)) => {
                let db_clone = db.clone();
                let telemetry_clone = telemetry.clone();
                let config_clone = config.clone();

                tokio::spawn(async move {
                    let _watcher = watcher;
                    while let Some(stats) = rx.recv().await {
                        info!("Library scan completed - Scanned: {}", stats.scanned);

                        if stats.scanned > 0 {
                            telemetry_clone.submit(&db_clone, &config_clone).await;
                        }
                    }
                });

                let db_clone = db.clone();
                let telemetry_clone = telemetry.clone();
                let config_clone = config.clone();

                tokio::spawn(async move {
                    info!("Starting initial library scan...");
                    match scanner_clone.scan_and_save(&db_clone).await {
                        Ok(stats) => {
                            info!("Initial scan complete - Scanned: {}", stats.scanned);

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

    fn could_be_audio_file(path: &Path) -> bool {
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

    pub async fn scan_and_save(&self, db: &Database) -> Result<ScanStats> {
        let mut scanned = 0;

        for root in &self.scan_paths {
            if !root.exists() || !root.is_dir() {
                continue;
            }

            let audio_files = tokio::task::spawn_blocking({
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

            let tracks_stream = stream::iter(audio_files)
                .map({
                    let db = db.clone();
                    move |path: PathBuf| {
                        let db = db.clone();
                        async move {
                            let file_path = path.to_string_lossy().to_string();
                            let file_meta = tokio::task::spawn_blocking({
                                let path = path.clone();
                                move || std::fs::metadata(&path)
                            })
                            .await
                            .ok()
                            .and_then(|r| r.ok())?;

                            let file_size = file_meta.len() as i64;
                            let file_modified = file_meta
                                .modified()
                                .ok()
                                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);

                            if let Ok(Some(existing)) = db.get_song_by_path(&file_path).await {
                                if existing.file_size == file_size
                                    && existing.file_modified == file_modified
                                {
                                    return None;
                                }
                            }

                            let metadata = tokio::task::spawn_blocking({
                                let path = path.clone();
                                move || Self::read_metadata(&path)
                            })
                            .await
                            .ok()
                            .and_then(|r| r.ok())?;

                            let image_data = tokio::task::spawn_blocking({
                                let path = path.clone();
                                move || Self::extract_image(&path)
                            })
                            .await
                            .ok()
                            .flatten();

                            Some(ScannedTrack {
                                path,
                                file_size,
                                file_modified,
                                metadata,
                                image_data,
                            })
                        }
                    }
                })
                .buffer_unordered(MAX_CONCURRENT_SCANS)
                .filter_map(|item| async move { item });

            futures::pin_mut!(tracks_stream);

            while let Some(track) = tracks_stream.next().await {
                if let Ok(()) = self.save_track(db, &track).await {
                    scanned += 1;
                }
            }
        }

        Ok(ScanStats { scanned })
    }

    pub async fn process_changed_files(
        &self,
        db: &Database,
        changed_paths: Vec<PathBuf>,
    ) -> Result<ScanStats> {
        let mut scanned = 0;

        for path in changed_paths {
            let path_clone = path.clone();

            if path.exists() && path.is_file() && Self::is_audio_file(&path) {
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

                if let Ok(Some(existing)) = db
                    .get_song_by_path(path_clone.to_string_lossy().as_ref())
                    .await
                {
                    if existing.file_size == file_size && existing.file_modified == file_modified {
                        continue;
                    }
                }

                let track = ScannedTrack {
                    path: path.clone(),
                    file_size,
                    file_modified,
                    metadata,
                    image_data,
                };

                match self.save_track(db, &track).await {
                    Ok(()) => {
                        scanned += 1;
                        debug!("Updated track: {:?}", path_clone);
                    }
                    Err(e) => error!("Failed to save track {:?}: {}", path_clone, e),
                }
            }
        }

        Ok(ScanStats { scanned })
    }

    async fn save_track(&self, db: &Database, track: &ScannedTrack) -> Result<()> {
        let path_str = track.path.to_string_lossy().to_string();
        let meta = &track.metadata;

        // First, save image if present (deduplication happens via hash ID)
        let image_id = if let Some(image) = &track.image_data {
            db.upsert_image(&image.id, &image.data).await?;
            Some(image.id.clone())
        } else {
            None
        };

        let artist_id = if let Some(artist_name) = &meta.artist {
            Some(db.upsert_artist(artist_name).await?)
        } else {
            None
        };

        let album_id = if let Some(album_name) = &meta.album {
            Some(
                db.upsert_album(album_name, artist_id.as_ref(), image_id.as_deref())
                    .await?,
            )
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
                    let mut changed_audio_files = Vec::new();

                    for event in events {
                        debug!("File event: {:?} - {:?}", event.kind, event.paths);

                        let is_meaningful_event = matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );

                        if is_meaningful_event {
                            for path in &event.paths {
                                if Scanner::is_audio_file(path)
                                    || (!path.exists() && Scanner::could_be_audio_file(path))
                                {
                                    changed_audio_files.push(path.clone());
                                }
                            }
                        }
                    }

                    if !changed_audio_files.is_empty() {
                        info!(
                            "Detected {} changed audio files, processing incrementally",
                            changed_audio_files.len()
                        );
                        let scanner = scanner_clone.clone();
                        let db = db_clone.clone();
                        let tx = tx.clone();

                        runtime_handle.spawn(async move {
                            match scanner
                                .process_changed_files(&db, changed_audio_files)
                                .await
                            {
                                Ok(stats) => {
                                    info!("Incremental scan complete - Scanned: {}", stats.scanned);
                                    let _ = tx.send(stats).await;
                                }
                                Err(e) => {
                                    error!("Incremental scan failed: {}", e);
                                }
                            }
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
