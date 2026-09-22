use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use gpui::{App, Global};
use serde::Deserialize;
use tracing::debug;
use ureq::Agent;

use crate::data::config::Config;
use crate::data::db::repo::Database;
use crate::data::models::Cuid;
use crate::media::playback::Playback;
use crate::media::queue::Queue;

const API_KEY: &str = match option_env!("LASTFM_API_KEY") {
    Some(v) => v,
    None => "",
};
const API_SECRET: &str = match option_env!("LASTFM_API_SECRET") {
    Some(v) => v,
    None => "",
};

const API_ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
const AUTH_ROOT: &str = "https://www.last.fm/api/auth/";
const MIN_SCROBBLE_DURATION_SECS: i32 = 30;

#[derive(Clone, Debug, Default)]
pub enum LastfmAuthStatus {
    #[default]
    Idle,
    Connecting,
    WaitingForBrowser,
    Error(String),
}

impl Global for LastfmAuthStatus {}

fn sign(params: &BTreeMap<&str, String>) -> String {
    let mut s = String::new();
    for (k, v) in params {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(API_SECRET);
    format!("{:x}", md5::compute(s.as_bytes()))
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}

#[derive(Deserialize)]
struct SessionResponse {
    session: SessionInner,
}

#[derive(Deserialize)]
struct SessionInner {
    key: String,
    name: String,
}

#[derive(Clone)]
pub struct LastfmClient {
    agent: Agent,
}

impl Global for LastfmClient {}

impl LastfmClient {
    pub fn init(cx: &mut App) {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .build()
            .into();
        cx.set_global(Self { agent });
        cx.set_global(LastfmAuthStatus::default());
    }

    pub fn is_configured() -> bool {
        !API_KEY.is_empty() && !API_SECRET.is_empty()
    }

    pub fn auth_url(token: &str) -> String {
        format!("{AUTH_ROOT}?api_key={API_KEY}&token={token}")
    }

    pub fn request_token(&self) -> Result<String> {
        if !Self::is_configured() {
            bail!("Last.fm integration is not configured");
        }

        let mut params = BTreeMap::new();
        params.insert("api_key", API_KEY.to_string());
        params.insert("method", "auth.getToken".to_string());
        let sig = sign(&params);

        let resp: TokenResponse = self
            .agent
            .get(API_ROOT)
            .query("api_key", API_KEY)
            .query("method", "auth.getToken")
            .query("api_sig", &sig)
            .query("format", "json")
            .call()
            .map_err(|e| anyhow!("last.fm request failed: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| anyhow!("last.fm response parse failed: {e}"))?;

        Ok(resp.token)
    }

    pub fn get_session(&self, token: &str) -> Result<(String, String)> {
        let mut params = BTreeMap::new();
        params.insert("api_key", API_KEY.to_string());
        params.insert("method", "auth.getSession".to_string());
        params.insert("token", token.to_string());
        let sig = sign(&params);

        let resp: SessionResponse = self
            .agent
            .get(API_ROOT)
            .query("api_key", API_KEY)
            .query("method", "auth.getSession")
            .query("token", token)
            .query("api_sig", &sig)
            .query("format", "json")
            .call()
            .map_err(|e| anyhow!("last.fm auth failed: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| anyhow!("last.fm response parse failed: {e}"))?;

        Ok((resp.session.key, resp.session.name))
    }

    fn signed_form(&self, mut params: BTreeMap<&str, String>) -> Vec<(String, String)> {
        let sig = sign(&params);
        params.insert("format", "json".to_string());
        let mut form: Vec<(String, String)> = params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        form.push(("api_sig".to_string(), sig));
        form
    }

    pub fn update_now_playing(
        &self,
        session_key: &str,
        artist: &str,
        track: &str,
        album: Option<&str>,
        duration: i32,
    ) {
        let mut params = BTreeMap::new();
        params.insert("api_key", API_KEY.to_string());
        params.insert("method", "track.updateNowPlaying".to_string());
        params.insert("sk", session_key.to_string());
        params.insert("artist", artist.to_string());
        params.insert("track", track.to_string());
        if let Some(album) = album {
            params.insert("album", album.to_string());
        }
        if duration > 0 {
            params.insert("duration", duration.to_string());
        }

        let form = self.signed_form(params);
        if let Err(e) = self.agent.post(API_ROOT).send_form(form) {
            debug!("last.fm updateNowPlaying failed: {e}");
        }
    }

    pub fn scrobble(
        &self,
        session_key: &str,
        artist: &str,
        track: &str,
        album: Option<&str>,
        timestamp: i64,
        duration: i32,
    ) {
        let mut params = BTreeMap::new();
        params.insert("api_key", API_KEY.to_string());
        params.insert("method", "track.scrobble".to_string());
        params.insert("sk", session_key.to_string());
        params.insert("artist", artist.to_string());
        params.insert("track", track.to_string());
        params.insert("timestamp", timestamp.to_string());
        if let Some(album) = album {
            params.insert("album", album.to_string());
        }
        if duration > 0 {
            params.insert("duration", duration.to_string());
        }

        let form = self.signed_form(params);
        if let Err(e) = self.agent.post(API_ROOT).send_form(form) {
            debug!("last.fm scrobble failed: {e}");
        }
    }
}

struct CachedSongInfo {
    title: String,
    artist: String,
    album: Option<String>,
    duration: i32,
    started_at: i64,
    now_playing_sent: bool,
    scrobbled: bool,
}

pub struct LastfmScrobbler {}

impl LastfmScrobbler {
    pub fn init(cx: &mut App) {
        let db = cx.global::<Database>().clone();

        cx.spawn(async move |cx| {
            let mut cached_song_id: Option<Cuid> = None;
            let mut cached_song_info: Option<CachedSongInfo> = None;

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;

                if !LastfmClient::is_configured() {
                    continue;
                }

                let (session_key, threshold) = cx.update(|app| {
                    let cfg = app.global::<Config>().get();
                    (
                        cfg.lastfm.session_key.clone(),
                        cfg.lastfm.scrobble_threshold,
                    )
                });
                let Some(session_key) = session_key else {
                    cached_song_id = None;
                    cached_song_info = None;
                    continue;
                };

                let song_id = cx.update(|app| {
                    app.try_global::<Queue>()
                        .and_then(|q| q.get_current_song_id())
                });

                if song_id != cached_song_id {
                    cached_song_id = song_id.clone();

                    cached_song_info = match &song_id {
                        Some(id) => match db.get_song(id) {
                            Ok(Some(song)) => {
                                let artist = song.artists.join(", ");
                                let album = song
                                    .album_id
                                    .as_ref()
                                    .and_then(|album_id| db.get_album(album_id).ok().flatten())
                                    .map(|a| a.title);
                                Some(CachedSongInfo {
                                    title: song.title,
                                    artist,
                                    album,
                                    duration: song.duration,
                                    started_at: unix_now_i64(),
                                    now_playing_sent: false,
                                    scrobbled: false,
                                })
                            }
                            _ => None,
                        },
                        None => None,
                    };
                }

                let Some(info) = cached_song_info.as_mut() else {
                    continue;
                };

                if info.artist.is_empty() {
                    continue;
                }

                let (position, is_paused) = cx.update(|app| {
                    app.try_global::<Playback>()
                        .map(|p| (p.get_position(), p.get_paused()))
                        .unwrap_or((0.0f32, true))
                });

                if is_paused {
                    continue;
                }

                let client = cx.update(|app| app.global::<LastfmClient>().clone());

                if !info.now_playing_sent {
                    info.now_playing_sent = true;
                    let client = client.clone();
                    let session_key = session_key.clone();
                    let title = info.title.clone();
                    let artist = info.artist.clone();
                    let album = info.album.clone();
                    let duration = info.duration;
                    cx.background_executor()
                        .spawn(async move {
                            client.update_now_playing(
                                &session_key,
                                &artist,
                                &title,
                                album.as_deref(),
                                duration,
                            );
                        })
                        .detach();
                }

                let scrobble_at = info.duration as f32 * threshold;
                if !info.scrobbled
                    && info.duration >= MIN_SCROBBLE_DURATION_SECS
                    && position >= scrobble_at
                {
                    info.scrobbled = true;
                    let client = client.clone();
                    let session_key = session_key.clone();
                    let title = info.title.clone();
                    let artist = info.artist.clone();
                    let album = info.album.clone();
                    let duration = info.duration;
                    let started_at = info.started_at;
                    cx.background_executor()
                        .spawn(async move {
                            client.scrobble(
                                &session_key,
                                &artist,
                                &title,
                                album.as_deref(),
                                started_at,
                                duration,
                            );
                        })
                        .detach();
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
