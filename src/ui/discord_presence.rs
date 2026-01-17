use crate::media::playback::Playback;
use crate::media::queue::Queue;
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use gpui::App;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct DiscordPresence {}

impl DiscordPresence {
    pub fn init(cx: &mut App) {
        let app_id = Arc::new("1194990403963858984".to_string());
        let client = Arc::new(Mutex::new(DiscordIpcClient::new(&app_id)));
        let _ = client.lock().unwrap().connect();

        let client = Arc::clone(&client);

        cx.spawn(async move |cx| {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                let song_opt = cx
                    .update(|app| app.try_global::<Queue>().and_then(|q| q.current().cloned()))
                    .ok()
                    .flatten();

                let (position, is_paused) = cx
                    .update(|app| {
                        app.try_global::<Playback>()
                            .map(|p| (p.get_position(), p.is_paused()))
                            .unwrap_or((0.0f32, true))
                    })
                    .unwrap_or((0.0f32, true));

                let mut client = client.lock().unwrap();

                if song_opt.is_none() || is_paused {
                    let _ = client.clear_activity();
                    continue;
                }

                let song = song_opt.as_ref().unwrap();
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

                if let Some(artist) = &song.artist {
                    act = act.state(&artist.name);
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
