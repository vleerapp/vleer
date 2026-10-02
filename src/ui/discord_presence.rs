use crate::data::config::Config;
use crate::data::db::Database;
use crate::data::models::Cuid;
use crate::media::playback::Playback;
use crate::media::queue::Queue;
use discord_rich_presence::activity::StatusDisplayType;
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
    large_image: Option<String>,
    small_image: Option<String>,
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
                        .map(|c| c.get().integrations.discord.enabled)
                        .unwrap_or(false)
                });

                if !discord_enabled {
                    let client = Arc::clone(&client);
                    let ok = cx
                        .background_executor()
                        .spawn(async move { client.lock().clear_activity().is_ok() })
                        .await;
                    if !ok {
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

                        let (song_omm, artist_omm) = db.song_omm_ids(&id).unwrap_or_default();
                        let ids: Vec<String> =
                            song_omm.iter().chain(artist_omm.iter()).cloned().collect();
                        let urls = crate::data::omm::fetch_artwork_urls(ids).await;
                        let large_image = song_omm.and_then(|id| urls.get(&id).cloned());
                        let small_image = artist_omm.and_then(|id| urls.get(&id).cloned());

                        cached_song_info = Some(CachedSongInfo {
                            title: song.title,
                            duration: song.duration / 1000,
                            artist_name,
                            large_image,
                            small_image,
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

                let desired = match cached_song_info.as_ref() {
                    Some(song) if !is_paused => {
                        let total_secs = song.duration as i64;
                        let elapsed_secs = position as i64;
                        let remaining_secs = total_secs.saturating_sub(elapsed_secs);
                        let end = unix_now_i64() + remaining_secs;
                        let start = end - total_secs;
                        Some((
                            song.title.clone(),
                            song.artist_name.clone(),
                            song.large_image.clone(),
                            song.small_image.clone(),
                            start,
                            end,
                        ))
                    }
                    _ => None,
                };

                let client = Arc::clone(&client);
                let ok = cx
                    .background_executor()
                    .spawn(async move {
                        let mut client = client.lock();
                        match &desired {
                            Some((title, artist_name, large_image, small_image, start, end)) => {
                                let mut act = activity::Activity::new()
                                    .status_display_type(StatusDisplayType::Details)
                                    .details(title)
                                    .activity_type(activity::ActivityType::Listening)
                                    .timestamps(
                                        activity::Timestamps::new().start(*start).end(*end),
                                    );

                                if let Some(name) = artist_name {
                                    act = act.state(name);
                                }

                                if large_image.is_some() || small_image.is_some() {
                                    let mut assets = activity::Assets::new();
                                    if let Some(url) = large_image {
                                        assets = assets.large_image(url).large_text(title);
                                    }
                                    if let Some(url) = small_image {
                                        assets = assets.small_image(url);
                                        if let Some(name) = artist_name {
                                            assets = assets.small_text(name);
                                        }
                                    }
                                    act = act.assets(assets);
                                }

                                client.set_activity(act).is_ok()
                            }
                            None => client.clear_activity().is_ok(),
                        }
                    })
                    .await;

                if !ok {
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
