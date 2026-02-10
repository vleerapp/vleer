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
        let _ = client.lock().unwrap().connect();

        let client = Arc::clone(&client);

        cx.spawn(async move |cx| {
            let mut cached_song_id: Option<Cuid> = None;
            let mut cached_song_info: Option<CachedSongInfo> = None;

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                let discord_enabled = cx.update(|app| {
                    app.try_global::<Config>()
                        .map(|c| c.get().discord_rpc)
                        .unwrap_or(false)
                });

                if !discord_enabled {
                    let mut client = client.lock().unwrap();
                    let _ = client.clear_activity();
                    continue;
                }

                let song_id =
                    cx.update(|app| app.try_global::<Queue>().and_then(|q| q.get_current_song_id()));

                if song_id != cached_song_id {
                    cached_song_id = song_id.clone();
                    cached_song_info = song_id.clone().and_then(|id| {
                        let song = cx.update(|app| {
                            app.try_global::<Database>().and_then(|db| {
                                futures::executor::block_on(db.get_song(id.clone()))
                                    .ok()
                                    .flatten()
                            })
                        })?;

                        let artist_name = cx.update(|app| {
                            song.artist_id
                                .as_ref()
                                .and_then(|artist_id| {
                                    app.try_global::<Database>().and_then(|db| {
                                        futures::executor::block_on(db.get_artist(artist_id.clone()))
                                            .ok()
                                            .flatten()
                                    })
                                })
                                .map(|artist| artist.name)
                        });

                        Some(CachedSongInfo {
                            title: song.title,
                            duration: song.duration,
                            artist_name,
                        })
                    });
                }

                let (position, is_paused) = cx.update(|app| {
                    app.try_global::<Playback>()
                        .map(|p| (p.get_position(), p.get_paused()))
                        .unwrap_or((0.0f32, true))
                });

                let mut client = client.lock().unwrap();

                if cached_song_info.is_none() || is_paused {
                    let _ = client.clear_activity();
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

                let _ = client.set_activity(act);
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
