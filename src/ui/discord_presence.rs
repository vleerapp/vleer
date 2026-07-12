use crate::data::config::Config;
use crate::data::db::repo::Database;
use crate::data::models::Cuid;
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use gpui::App;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct DiscordPresence {}

struct CachedSongInfo {
    title: String,
    duration: i32,
    artist_name: Option<String>,
}

impl DiscordPresence {
    pub fn init(cx: &mut App) {
        let app_id = Arc::new("1194990403963858984".to_string());
        let client = Arc::new(Mutex::new(DiscordIpcClient::new(&*app_id)));
        let connected = Arc::new(Mutex::new(false));

        let client = Arc::clone(&client);
        let connected = Arc::clone(&connected);
        let db = cx.global::<Database>().clone();

        cx.spawn(async move |cx| {
            let mut cached_song_id: Option<Cuid> = None;
            let mut cached_song_info: Option<CachedSongInfo> = None;

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;

                if !*connected.lock() {
                    let client = Arc::clone(&client);
                    let ok = cx
                        .background_executor()
                        .spawn(async move { client.lock().connect().is_ok() })
                        .await;
                    *connected.lock() = ok;
                    if !ok {
                        continue;
                    }
                }

                let discord_enabled = cx.update(|app| {
                    app.try_global::<Config>()
                        .map(|c| c.get().discord_rpc)
                        .unwrap_or(false)
                });

                if !discord_enabled {
                    let mut client = client.lock();
                    if client.clear_activity().is_err() {
                        *connected.lock() = false;
                    }
                    continue;
                }

                let song_id = cx.update(|app| {
                    app.try_global::<Queue>()
                        .and_then(|q| q.get_current_song_id())
                });

                if song_id != cached_song_id {
                    cached_song_id = song_id.clone();

                    if let Some(id) = song_id {
                        let song = match db.get_song(&id) {
                            Ok(Some(s)) => s,
                            _ => {
                                cached_song_info = None;
                                continue;
                            }
                        };

                        let artist_name = Some(song.artists.join(", ")).filter(|s| !s.is_empty());

                        cached_song_info = Some(CachedSongInfo {
                            title: song.title,
                            duration: song.duration,
                            artist_name,
                        });
                    } else {
                        cached_song_info = None;
                    }
                }

                let (position, is_paused) = cx.update(|app| {
                    app.try_global::<Playback>()
                        .map(|p| (p.get_position(), p.get_paused()))
                        .unwrap_or((0.0f32, true))
                });

                let mut client = client.lock();

                let Some(song) = cached_song_info.as_ref() else {
                    if client.clear_activity().is_err() {
                        drop(client);
                        *connected.lock() = false;
                    }
                    continue;
                };

                if is_paused {
                    if client.clear_activity().is_err() {
                        drop(client);
                        *connected.lock() = false;
                    }
                    continue;
                }

                let total_secs = song.duration as f64;
                let elapsed_secs = position as i64;
                let remaining_secs = (total_secs as i64).saturating_sub(elapsed_secs);
                let end = unix_now_i64() + remaining_secs;
                let start = end - total_secs as i64;

                let mut act = activity::Activity::new()
                    .details(&song.title)
                    .activity_type(activity::ActivityType::Listening)
                    .timestamps(activity::Timestamps::new().start(start).end(end));

                match &song.artist_name {
                    Some(name) => {
                        act = act.state(name);
                    }
                    None => {
                        act = act.state("Playing");
                    }
                }

                if client.set_activity(act).is_err() {
                    drop(client);
                    *connected.lock() = false;
                }
            }
        })
        .detach();
    }
}

fn unix_now_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
