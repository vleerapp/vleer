use crate::data::config::Config;
use crate::data::db::repo::Database;
use crate::data::models::Cuid;
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use gpui::App;
use std::sync::{Arc, Mutex};
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

                if !*connected.lock().unwrap() {
                    let client = Arc::clone(&client);
                    let connected = Arc::clone(&connected);
                    let ok = tokio::task::spawn_blocking(move || {
                        client.lock().unwrap().connect().is_ok()
                    })
                    .await
                    .unwrap_or(false);
                    *connected.lock().unwrap() = ok;
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
                    let mut client = client.lock().unwrap();
                    if client.clear_activity().is_err() {
                        *connected.lock().unwrap() = false;
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
                        let song = match db.get_song(id).await {
                            Ok(Some(s)) => s,
                            _ => {
                                cached_song_info = None;
                                continue;
                            }
                        };

                        let artist_name = if let Some(ref artist_id) = song.artist_id {
                            db.get_artist(artist_id.clone())
                                .await
                                .ok()
                                .flatten()
                                .map(|a| a.name)
                        } else {
                            None
                        };

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

                let mut client = client.lock().unwrap();

                if cached_song_info.is_none() || is_paused {
                    if client.clear_activity().is_err() {
                        drop(client);
                        *connected.lock().unwrap() = false;
                    }
                    continue;
                }

                let song = cached_song_info.as_ref().unwrap();
                let total_secs = song.duration as f64;
                let elapsed_secs = position as i64;
                let remaining_secs = (total_secs as i64).saturating_sub(elapsed_secs);
                let end = unix_now_i64() + remaining_secs;
                let start = end - total_secs as i64;

                let mut act = activity::Activity::new()
                    .details(&song.title)
                    .state("Playing")
                    .activity_type(activity::ActivityType::Listening)
                    .timestamps(activity::Timestamps::new().start(start).end(end));

                if let Some(ref name) = song.artist_name {
                    act = act.state(name);
                }

                if client.set_activity(act).is_err() {
                    drop(client);
                    *connected.lock().unwrap() = false;
                }
            }
        })
        .detach();
    }
}

fn unix_now_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
