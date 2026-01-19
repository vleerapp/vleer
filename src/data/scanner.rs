use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use gpui::App;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::data::config::Config;
use crate::data::db::Database;
use crate::data::metadata::{AudioMetadata, extract_and_save_cover};
use crate::data::state::State;
use crate::data::telemetry::Telemetry;

const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "m4a", "aac", "wav", "mp1", "mp2"];
const MAX_CONCURRENT_SCANS: usize = 8;

#[derive(Debug, Clone)]
pub struct ScanStats {
    pub _scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Debug, Clone)]
enum SaveAction {
    Added,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct ScannedTrack {
    pub path: PathBuf,
    pub metadata: AudioMetadata,
    pub cover: Option<String>,
}

pub struct Scanner {
    scan_paths: Vec<PathBuf>,
    covers_dir: PathBuf,
}

impl Scanner {
    pub fn new(scan_paths: Vec<PathBuf>, covers_dir: PathBuf) -> Self {
        Self {
            scan_paths,
            covers_dir,
        }
    }

    pub fn init(cx: &mut App) {
        let config = cx.global::<Config>().clone();
        let db = cx.global::<Database>().clone();
        let state = cx.global::<State>().clone();
        let telemetry = cx.global::<Telemetry>().clone();

        let data_dir = dirs::data_dir()
            .expect("couldn't get data directory")
            .join("vleer");

        let covers_dir = data_dir.join("covers");
        if !covers_dir.exists() {
            let _ = std::fs::create_dir_all(&covers_dir);
        }

        let scan_paths = expand_scan_paths(&config.get().scan.paths);
        let scanner = Arc::new(Scanner::new(scan_paths, covers_dir));
        let scanner_clone = scanner.clone();

        match MusicWatcher::new(scanner.clone(), Arc::new(db.clone())) {
            Ok((watcher, mut rx)) => {
                let state_clone = state.clone();
                let db_clone = db.clone();
                let telemetry_clone = telemetry.clone();
                let config_clone = config.clone();

                tokio::spawn(async move {
                    let _watcher = watcher;
                    while let Some(stats) = rx.recv().await {
                        info!(
                            "Library scan completed - Added: {}, Updated: {}, Removed: {}",
                            stats.added, stats.updated, stats.removed
                        );

                        if stats.added > 0 || stats.updated > 0 || stats.removed > 0 {
                            State::refresh(&db_clone, &state_clone).await;
                            telemetry_clone.submit(&state_clone, &config_clone).await;
                        }
                    }
                });

                let db_clone = db.clone();
                let state_clone = state.clone();
                let telemetry_clone = telemetry.clone();
                let config_clone = config.clone();

                tokio::spawn(async move {
                    info!("Starting initial library scan...");
                    match scanner_clone.scan_and_save(&db_clone).await {
                        Ok(stats) => {
                            info!(
                                "Initial scan complete - Added: {}, Updated: {}, Removed: {}",
                                stats.added, stats.updated, stats.removed
                            );

                            if stats.added > 0 || stats.updated > 0 || stats.removed > 0 {
                                State::refresh(&db_clone, &state_clone).await;
                                telemetry_clone.submit(&state_clone, &config_clone).await;
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

    fn read_metadata_only(path: &Path) -> Result<AudioMetadata> {
        AudioMetadata::from_path_with_options(path, false)
    }

    fn extract_cover_only(path: &Path, covers_dir: &Path) -> Option<String> {
        match extract_and_save_cover(path, covers_dir) {
            Ok(hash) => hash,
            Err(e) => {
                debug!("Failed to extract cover from {:?}: {}", path, e);
                None
            }
        }
    }

    pub async fn scan_and_save(&self, db: &Database) -> Result<ScanStats> {
        let mut stats = ScanStats {
            _scanned: 0,
            added: 0,
            updated: 0,
            removed: 0,
        };

        let mut found_paths = std::collections::HashSet::new();

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

            stats._scanned += audio_files.len();

            let covers_dir = self.covers_dir.clone();
            let tracks_stream = stream::iter(audio_files)
                .map(|path: PathBuf| {
                    let path_str = path.to_string_lossy().to_string();
                    let db = db.clone();
                    let covers_dir = covers_dir.clone();

                    async move {
                        let mtime = path.metadata().and_then(|m| m.modified()).ok();
                        let existing = db.get_song_by_path(&path_str).await.ok().flatten();

                        let needs_scan = match (&existing, mtime) {
                            (Some(song), Some(mod_time)) => {
                                if let Ok(dt) = DateTime::parse_from_rfc3339(&song.date_updated) {
                                    (dt.with_timezone(&Utc).timestamp() as u64)
                                        < mod_time
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap_or(Duration::ZERO)
                                            .as_secs()
                                } else {
                                    true
                                }
                            }
                            _ => true,
                        };

                        if !needs_scan {
                            return None;
                        }

                        let metadata = tokio::task::spawn_blocking({
                            let path = path.clone();
                            move || Self::read_metadata_only(&path)
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok())?;

                        let needs_cover = existing.is_none()
                            || existing.as_ref().and_then(|s| s.cover.as_ref()).is_none();

                        let cover = if needs_cover {
                            tokio::task::spawn_blocking({
                                let path = path.clone();
                                let covers_dir = covers_dir.clone();
                                move || Self::extract_cover_only(&path, &covers_dir)
                            })
                            .await
                            .ok()
                            .flatten()
                        } else {
                            existing.as_ref().and_then(|s| s.cover.clone())
                        };

                        Some((
                            ScannedTrack {
                                path,
                                metadata,
                                cover,
                            },
                            path_str,
                        ))
                    }
                })
                .buffer_unordered(MAX_CONCURRENT_SCANS)
                .filter_map(|item| async move { item });

            futures::pin_mut!(tracks_stream);

            while let Some((track, path_str)) = tracks_stream.next().await {
                found_paths.insert(path_str.clone());

                if let Ok(action) = self.save_or_update_track(db, &track).await {
                    match action {
                        SaveAction::Added => stats.added += 1,
                        SaveAction::Updated => stats.updated += 1,
                        _ => {}
                    }
                }
            }
        }

        stats.removed = self
            .remove_missing_songs(db, &found_paths)
            .await
            .unwrap_or(0);

        if stats.removed > 0 {
            let _ = db.cleanup_orphaned_artists().await;
            let _ = db.cleanup_orphaned_albums().await;
        }

        Ok(stats)
    }

    pub async fn process_changed_files(
        &self,
        db: &Database,
        changed_paths: Vec<PathBuf>,
    ) -> Result<ScanStats> {
        let mut stats = ScanStats {
            _scanned: changed_paths.len(),
            added: 0,
            updated: 0,
            removed: 0,
        };

        let covers_dir = self.covers_dir.clone();

        for path in changed_paths {
            let path_clone = path.clone();

            if path.exists() && path.is_file() && Self::is_audio_file(&path) {
                let path_str = path.to_string_lossy().to_string();
                let existing = db.get_song_by_path(&path_str).await.ok().flatten();

                let metadata = match tokio::task::spawn_blocking({
                    let path = path.clone();
                    move || Self::read_metadata_only(&path)
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

                let needs_cover = existing.is_none()
                    || existing.as_ref().and_then(|s| s.cover.as_ref()).is_none();

                let cover = if needs_cover {
                    tokio::task::spawn_blocking({
                        let path = path.clone();
                        let covers_dir = covers_dir.clone();
                        move || Self::extract_cover_only(&path, &covers_dir)
                    })
                    .await
                    .ok()
                    .flatten()
                } else {
                    existing.as_ref().and_then(|s| s.cover.clone())
                };

                let track = ScannedTrack {
                    path: path.clone(),
                    metadata,
                    cover,
                };

                match self.save_or_update_track(db, &track).await {
                    Ok(SaveAction::Added) => stats.added += 1,
                    Ok(SaveAction::Updated) => stats.updated += 1,
                    Ok(SaveAction::Unchanged) => {}
                    Err(e) => error!("Failed to save track {:?}: {}", path_clone, e),
                }
            } else if !path.exists() {
                let path_str = path.to_string_lossy().to_string();
                match db.get_song_by_path(&path_str).await {
                    Ok(Some(song)) => {
                        if let Err(e) = db.delete_song(&song.id).await {
                            error!("Failed to delete song {:?}: {}", song.id, e);
                        } else {
                            stats.removed += 1;
                            debug!("Removed deleted song: {:?}", path);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => error!("Failed to check song existence: {}", e),
                }
            }
        }

        if stats.removed > 0 {
            debug!("Cleaning up orphaned artists and albums");
            if let Err(e) = db.cleanup_orphaned_artists().await {
                error!("Failed to cleanup orphaned artists: {}", e);
            }
            if let Err(e) = db.cleanup_orphaned_albums().await {
                error!("Failed to cleanup orphaned albums: {}", e);
            }
        }

        Ok(stats)
    }

    async fn save_or_update_track(
        &self,
        db: &Database,
        track: &ScannedTrack,
    ) -> Result<SaveAction> {
        let path_str = track.path.to_string_lossy().to_string();
        let meta = &track.metadata;

        let existing_song = db.get_song_by_path(&path_str).await?;

        let artist_id = if let Some(artist_name) = &meta.artist {
            Some(
                db.insert_artist(artist_name, meta.album_artist.as_deref())
                    .await?,
            )
        } else {
            None
        };

        let cover = track.cover.as_deref();

        let album_id = if let Some(album_name) = &meta.album {
            Some(
                db.insert_album(
                    album_name,
                    artist_id.as_ref(),
                    meta.year,
                    meta.genre.as_deref(),
                    cover.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };

        let title = meta.title.as_deref().unwrap_or("Unknown");
        let duration = meta.duration.as_secs() as i32;
        let track_number = meta.track_number.map(|n| n as i32);

        if let Some(existing) = existing_song {
            let metadata_changed = existing.title != title
                || existing.artist_id.as_ref() != artist_id.as_ref()
                || existing.album_id.as_ref() != album_id.as_ref()
                || existing.duration != duration
                || existing.track_number != track_number
                || existing.date != meta.year.map(|y| y.to_string())
                || existing.genre.as_deref() != meta.genre.as_deref()
                || existing.cover.as_deref() != cover.as_deref()
                || existing.track_lufs != meta.track_lufs;
            if metadata_changed {
                db.update_song_metadata(
                    &existing.id,
                    title,
                    artist_id.as_ref(),
                    album_id.as_ref(),
                    duration,
                    track_number,
                    meta.year,
                    meta.genre.as_deref(),
                    cover.as_deref(),
                    meta.track_lufs,
                )
                .await?;
                debug!("Updated metadata for: {:?}", track.path);
                Ok(SaveAction::Updated)
            } else {
                Ok(SaveAction::Unchanged)
            }
        } else {
            db.insert_song(
                title,
                artist_id.as_ref(),
                album_id.as_ref(),
                &path_str,
                duration,
                track_number,
                meta.year,
                meta.genre.as_deref(),
                cover.as_deref(),
                meta.track_lufs,
            )
            .await?;
            debug!("Added new song: {:?}", track.path);
            Ok(SaveAction::Added)
        }
    }

    async fn remove_missing_songs(
        &self,
        db: &Database,
        found_paths: &std::collections::HashSet<String>,
    ) -> Result<usize> {
        let all_songs = db.get_all_songs().await?;
        let mut removed_count = 0;

        for song in all_songs {
            let song_path = PathBuf::from(&song.file_path);
            let is_in_scan_path = self
                .scan_paths
                .iter()
                .any(|scan_path| song_path.starts_with(scan_path));

            if !is_in_scan_path || !found_paths.contains(&song.file_path) {
                debug!("Removing song: {:?}", song.file_path);
                if let Err(e) = db.delete_song(&song.id).await {
                    error!("Failed to delete song {:?}: {}", song.id, e);
                } else {
                    removed_count += 1;
                }
            }
        }

        Ok(removed_count)
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
                                if Scanner::is_audio_file(path) || (!path.exists() && Scanner::could_be_audio_file(path)) {
                                    changed_audio_files.push(path.clone());
                                }
                            }
                        }
                    }

                    if !changed_audio_files.is_empty() {
                        info!("Detected {} changed audio files, processing incrementally", changed_audio_files.len());
                        let scanner = scanner_clone.clone();
                        let db = db_clone.clone();
                        let tx = tx.clone();

                        runtime_handle.spawn(async move {
                            match scanner.process_changed_files(&db, changed_audio_files).await {
                                Ok(stats) => {
                                    info!(
                                        "Incremental scan complete - Added: {}, Updated: {}, Removed: {}",
                                        stats.added, stats.updated, stats.removed
                                    );
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
