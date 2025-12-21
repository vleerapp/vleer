use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use gpui::App;
use notify::{EventKind, RecursiveMode, event::ModifyKind};
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use walkdir::WalkDir;

use crate::data::config::Config;
use crate::data::db::Database;
use crate::data::metadata::{AudioMetadata, extract_and_save_cover};
use crate::data::state::State;
use crate::data::telemetry::Telemetry;

const SUPPORTED_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "m4a", "aac", "wav", "mp1", "mp2"];
const MAX_CONCURRENT_SCANS: usize = 16;

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
    pub cover_path: Option<PathBuf>,
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
                        }

                        telemetry_clone.submit(&state_clone, &config_clone).await;
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

    pub async fn scan(&self) -> Result<Vec<ScannedTrack>> {
        let mut all_tracks = Vec::new();

        for path in &self.scan_paths {
            info!("Scanning directory: {:?}", path);

            if !path.exists() {
                warn!("Scan path does not exist: {:?}", path);
                continue;
            }

            if !path.is_dir() {
                warn!("Scan path is not a directory: {:?}", path);
                continue;
            }

            match self.scan_directory(path).await {
                Ok(mut found_tracks) => {
                    all_tracks.append(&mut found_tracks);
                }
                Err(e) => {
                    error!("Error scanning directory {:?}: {}", path, e);
                }
            }
        }

        info!("Scan complete. Found {} total tracks", all_tracks.len());
        Ok(all_tracks)
    }

    async fn scan_directory(&self, path: &Path) -> Result<Vec<ScannedTrack>> {
        let path = path.to_path_buf();

        let audio_files = tokio::task::spawn_blocking(move || {
            let mut files = Vec::new();
            for entry in WalkDir::new(&path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() && Self::is_audio_file(path) {
                    files.push(path.to_path_buf());
                }
            }
            files
        })
        .await
        .context("Failed to walk directory")?;

        debug!("Found {} audio files to scan", audio_files.len());

        let covers_dir = self.covers_dir.clone();
        let tracks: Vec<ScannedTrack> = stream::iter(audio_files)
            .map(|path| {
                let covers_dir = covers_dir.clone();
                async move {
                    let path_clone = path.clone();
                    tokio::task::spawn_blocking(move || Self::read_metadata(&path, &covers_dir))
                        .await
                        .ok()
                        .and_then(|result| {
                            result
                                .map_err(|e| {
                                    warn!("Failed to read metadata from {:?}: {}", path_clone, e);
                                    e
                                })
                                .ok()
                        })
                }
            })
            .buffer_unordered(MAX_CONCURRENT_SCANS)
            .filter_map(|track| async move { track })
            .collect()
            .await;

        debug!("Successfully scanned {} tracks", tracks.len());
        Ok(tracks)
    }

    fn is_audio_file(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
            .unwrap_or(false)
    }

    fn read_metadata(path: &Path, covers_dir: &Path) -> Result<ScannedTrack> {
        let metadata = AudioMetadata::from_path(path)?;

        let cover_path = match extract_and_save_cover(path, covers_dir) {
            Ok(path) => path,
            Err(e) => {
                debug!("Failed to extract cover from {:?}: {}", path, e);
                None
            }
        };

        Ok(ScannedTrack {
            path: path.to_path_buf(),
            metadata,
            cover_path,
        })
    }

    pub async fn scan_and_save(&self, db: &Database) -> Result<ScanStats> {
        let tracks = self.scan().await?;
        let scanned_count = tracks.len();

        debug!("Processing {} scanned tracks", scanned_count);

        let mut stats = ScanStats {
            _scanned: scanned_count,
            added: 0,
            updated: 0,
            removed: 0,
        };

        let mut found_paths = std::collections::HashSet::new();

        for track in tracks {
            let path_str = track.path.to_string_lossy().to_string();
            found_paths.insert(path_str.clone());

            match self.save_or_update_track(db, &track).await {
                Ok(action) => match action {
                    SaveAction::Added => stats.added += 1,
                    SaveAction::Updated => stats.updated += 1,
                    SaveAction::Unchanged => {}
                },
                Err(e) => {
                    error!("Failed to save track {:?}: {}", track.path, e);
                }
            }
        }

        debug!("Checking for songs to remove from database");
        match self.remove_missing_songs(db, &found_paths).await {
            Ok(removed) => {
                stats.removed = removed;
                if removed > 0 {
                    debug!("Removed {} songs that no longer exist", removed);
                }
            }
            Err(e) => {
                error!("Failed to remove missing songs: {}", e);
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

        info!(
            "Scan complete - Added: {}, Updated: {}, Removed: {}",
            stats.added, stats.updated, stats.removed
        );
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

        let cover_hash = track
            .cover_path
            .as_ref()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()));

        let album_id = if let Some(album_name) = &meta.album {
            Some(
                db.insert_album(
                    album_name,
                    artist_id.as_ref(),
                    meta.year,
                    meta.genre.as_deref(),
                    cover_hash.as_deref(),
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
                || existing.cover.as_deref() != cover_hash.as_deref()
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
                    cover_hash.as_deref(),
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
                cover_hash.as_deref(),
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
                    let has_audio_changes = events.iter().any(|event| {
                        let is_meaningful_event = matches!(
                            event.kind,
                            EventKind::Create(_)
                                | EventKind::Modify(ModifyKind::Data(_))
                                | EventKind::Remove(_)
                        );

                        is_meaningful_event
                            && event.paths.iter().any(|path| Scanner::is_audio_file(path))
                    });

                    if has_audio_changes {
                        info!("Detected meaningful changes in music files, triggering rescan");
                        let scanner = scanner_clone.clone();
                        let db = db_clone.clone();
                        let tx = tx.clone();

                        runtime_handle.spawn(async move {
                            match scanner.scan_and_save(&db).await {
                                Ok(stats) => {
                                    info!(
                                        "Auto-scan complete - Added: {}, Updated: {}, Removed: {}",
                                        stats.added, stats.updated, stats.removed
                                    );
                                    let _ = tx.send(stats).await;
                                }
                                Err(e) => {
                                    error!("Auto-scan failed: {}", e);
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
        };

        Ok((watcher, rx))
    }
}
