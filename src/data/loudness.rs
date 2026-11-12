use crate::data::db::Database;
use crate::data::images::priority::{self, Tier};
use crate::data::jobs::{Bulk, Claims, Job, JobContext, Stop};
use crate::data::models::Cuid;
use crate::ui::components::context_menu::BackgroundUiEvent;
use anyhow::Result;
use ebur128::{EbuR128, Mode};
use rodio::Source;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn};

const BULK_BATCH: usize = 16;
const CHUNK_FRAMES: usize = 8192;
const SILENCE_LUFS: f32 = -70.0;

fn workers() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cores / 4).max(1)
}

fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers())
            .thread_name(|i| format!("loudness-{i}"))
            .start_handler(|_| crate::data::jobs::lower_thread_priority())
            .build()
            .expect("failed to build loudness thread pool")
    })
}

fn measure(path: &str, stop: &dyn Fn() -> bool) -> Result<Option<f32>> {
    let decoder = crate::data::metadata::open_decoder(path)?;

    let channels = decoder.channels().get() as usize;
    let mut state = EbuR128::new(channels as u32, decoder.sample_rate().get(), Mode::I)?;
    let chunk = CHUNK_FRAMES * channels;
    let mut buf: Vec<f32> = Vec::with_capacity(chunk);

    for sample in decoder {
        buf.push(sample);
        if buf.len() == chunk {
            state.add_frames_f32(&buf)?;
            buf.clear();
            if stop() {
                return Ok(None);
            }
        }
    }
    buf.truncate(buf.len() - buf.len() % channels);
    if !buf.is_empty() {
        state.add_frames_f32(&buf)?;
    }

    let loudness = state.loudness_global()?;
    Ok(Some(if loudness.is_finite() {
        loudness as f32
    } else {
        SILENCE_LUFS
    }))
}

type Work = (Cuid, String);

fn pick(db: &Database, claims: &Claims) -> Option<(Tier, Work)> {
    let snapshot = priority::snapshot();

    let mut song_ids: Vec<Cuid> = snapshot.playing.iter().cloned().collect();
    song_ids.extend(snapshot.songs.iter().cloned());
    song_ids.extend(snapshot.near_songs.iter().cloned());
    let by_song: HashMap<Cuid, String> = db
        .get_songs_missing_lufs_in(&song_ids)
        .unwrap_or_default()
        .into_iter()
        .collect();

    let mut album_ids = snapshot.albums.clone();
    album_ids.extend(snapshot.near_albums.iter().cloned());
    let mut by_album: HashMap<Cuid, Vec<Work>> = HashMap::new();
    for (album, id, path) in db
        .get_album_songs_missing_lufs(&album_ids)
        .unwrap_or_default()
    {
        by_album.entry(album).or_default().push((id, path));
    }

    let song = |ids: &[Cuid]| -> Option<Work> {
        ids.iter()
            .filter(|id| !claims.blocked(id))
            .find_map(|id| by_song.get(id).map(|path| (id.clone(), path.clone())))
    };
    let album = |ids: &[Cuid]| -> Option<Work> {
        ids.iter()
            .filter_map(|id| by_album.get(id))
            .flatten()
            .find(|(id, _)| !claims.blocked(id))
            .cloned()
    };

    if let Some(found) = song(&snapshot.playing.iter().cloned().collect::<Vec<_>>()) {
        return Some((Tier::Playing, found));
    }
    if let Some(found) = song(&snapshot.songs) {
        return Some((Tier::Visible, found));
    }
    if let Some(found) = album(&snapshot.albums) {
        return Some((Tier::Visible, found));
    }
    if let Some(found) = song(&snapshot.near_songs) {
        return Some((Tier::Near, found));
    }
    album(&snapshot.near_albums).map(|found| (Tier::Near, found))
}

pub struct LoudnessJob {
    ctx: JobContext,
    claims: Claims,
    measured: AtomicUsize,
}

impl LoudnessJob {
    pub fn new(ctx: &JobContext) -> Self {
        Self {
            ctx: ctx.clone(),
            claims: Claims::default(),
            measured: AtomicUsize::new(0),
        }
    }
}

impl Job for LoudnessJob {
    type Work = Work;

    fn name(&self) -> &'static str {
        "loudness"
    }

    fn bulk_concurrency(&self) -> usize {
        workers()
    }

    fn priority(&self) -> Option<(Tier, Work)> {
        pick(&self.ctx.db, &self.claims)
    }

    async fn bulk(&self) -> Bulk<Work> {
        let limit = (BULK_BATCH + self.claims.failed()) as i64;
        let pending = match self.ctx.db.get_songs_missing_lufs(limit) {
            Ok(rows) => rows,
            Err(e) => {
                warn!("loudness pass could not list songs: {e}");
                return Bulk::Done;
            }
        };
        let batch: Vec<Work> = pending
            .into_iter()
            .filter(|(id, _)| !self.claims.blocked(id))
            .take(BULK_BATCH)
            .collect();
        if batch.is_empty() {
            Bulk::Done
        } else if priority::file_busy() {
            Bulk::WaitFile
        } else {
            Bulk::Works(batch)
        }
    }

    async fn run(&self, _tier: Tier, (id, path): Work, stop: &Stop) {
        let Some(_claim) = self.claims.claim(&id) else {
            return;
        };
        let stop = stop.clone();
        let (tx, rx) = futures::channel::oneshot::channel();
        let measure_path = path.clone();
        pool().spawn(move || {
            let _ = tx.send(measure(&measure_path, &|| stop.requested()));
        });
        match rx.await {
            Ok(Ok(Some(lufs))) => {
                if let Err(e) = self.ctx.db.store_songs_lufs(&[(id.clone(), lufs)]) {
                    warn!("loudness pass could not store result: {e}");
                    return;
                }
                self.measured.fetch_add(1, Ordering::AcqRel);
                if priority::snapshot().playing.as_ref() == Some(&id) {
                    self.ctx.notify(BackgroundUiEvent::LoudnessChanged);
                }
            }
            Ok(Ok(None)) | Err(_) => {}
            Ok(Err(e)) => {
                warn!("loudness failed for {path}: {e:#}");
                self.claims.fail(id);
            }
        }
    }

    fn report(&self) {
        let measured = self.measured.swap(0, Ordering::AcqRel);
        if measured > 0 {
            info!("Measured loudness for {measured} song(s)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<Cuid> {
        names.iter().map(|n| Cuid::from(n.to_string())).collect()
    }

    #[test]
    fn outward_leans_toward_the_scroll_direction() {
        let before = ids(&["b1", "b2", "b3"]);
        let after = ids(&["a1", "a2", "a3", "a4"]);
        let down = priority::outward(before.clone(), after.clone(), true);
        assert_eq!(down, ids(&["a1", "a2", "b1", "a3", "a4", "b2", "b3"]));
        let up = priority::outward(before, after, false);
        assert_eq!(up, ids(&["b1", "b2", "a1", "b3", "a2", "a3", "a4"]));
    }
}
