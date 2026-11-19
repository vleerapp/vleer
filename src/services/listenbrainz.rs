use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use gpui::{App, Global};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;
use ureq::Agent;

#[derive(Clone, Debug, Default)]
pub enum ListenbrainzStatus {
    #[default]
    Idle,
    Checking,
    Error(String),
}

impl Global for ListenbrainzStatus {}

#[derive(Deserialize)]
struct ValidateResponse {
    #[serde(default)]
    valid: bool,
    #[serde(default)]
    user_name: Option<String>,
}

#[derive(Clone)]
pub struct ListenbrainzClient {
    agent: Agent,
}

impl Global for ListenbrainzClient {}

fn endpoint(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let base = if base.contains("://") {
        base.to_string()
    } else {
        format!("https://{base}")
    };
    format!("{base}/1/{path}")
}

fn track_metadata(artist: &str, track: &str, album: Option<&str>, duration: i32) -> Value {
    let mut additional = json!({
        "media_player": "Vleer",
        "submission_client": "Vleer",
        "submission_client_version": env!("CARGO_PKG_VERSION"),
    });
    if duration > 0 {
        additional["duration_ms"] = json!(i64::from(duration) * 1000);
    }

    let mut meta = json!({
        "artist_name": artist,
        "track_name": track,
        "additional_info": additional,
    });
    if let Some(album) = album {
        meta["release_name"] = json!(album);
    }
    meta
}

impl ListenbrainzClient {
    pub fn init(cx: &mut App) {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(8)))
            .build()
            .into();
        cx.set_global(Self { agent });
        cx.set_global(ListenbrainzStatus::default());
    }

    pub fn validate_token(&self, url: &str, token: &str) -> Result<String> {
        let resp: ValidateResponse = self
            .agent
            .get(endpoint(url, "validate-token"))
            .header("Authorization", format!("Token {token}"))
            .call()
            .map_err(|e| anyhow!("request failed: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| anyhow!("response parse failed: {e}"))?;

        if !resp.valid {
            bail!("invalid token");
        }
        Ok(resp.user_name.unwrap_or_else(|| "unknown".to_string()))
    }

    fn submit(&self, url: &str, token: &str, body: Value) {
        if let Err(e) = self
            .agent
            .post(endpoint(url, "submit-listens"))
            .header("Authorization", format!("Token {token}"))
            .send_json(body)
        {
            warn!("listenbrainz submit failed: {e}");
        }
    }

    pub fn update_now_playing(
        &self,
        url: &str,
        token: &str,
        artist: &str,
        track: &str,
        album: Option<&str>,
        duration: i32,
    ) {
        self.submit(
            url,
            token,
            json!({
                "listen_type": "playing_now",
                "payload": [{ "track_metadata": track_metadata(artist, track, album, duration) }],
            }),
        );
    }

    pub fn scrobble(
        &self,
        url: &str,
        token: &str,
        artist: &str,
        track: &str,
        album: Option<&str>,
        timestamp: i64,
        duration: i32,
    ) {
        self.submit(
            url,
            token,
            json!({
                "listen_type": "single",
                "payload": [{
                    "listened_at": timestamp,
                    "track_metadata": track_metadata(artist, track, album, duration),
                }],
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve(response_body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(300)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = vec![0u8; 8192];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let req = String::from_utf8_lossy(&data).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(resp.as_bytes()).unwrap();
            req
        });
        (format!("http://{addr}/apis/listenbrainz"), handle)
    }

    fn client() -> ListenbrainzClient {
        ListenbrainzClient {
            agent: Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(8)))
                .build()
                .into(),
        }
    }

    #[test]
    fn validates_token() {
        let (url, handle) = serve(r#"{"code":200,"valid":true,"user_name":"bob"}"#);
        let name = client().validate_token(&url, "tok").unwrap();
        assert_eq!(name, "bob");
        let req = handle.join().unwrap().to_lowercase();
        assert!(
            req.starts_with("get /apis/listenbrainz/1/validate-token"),
            "{req}"
        );
        assert!(req.contains("authorization: token tok"), "{req}");
    }

    #[test]
    fn submits_listen() {
        let (url, handle) = serve(r#"{"status":"ok"}"#);
        client().scrobble(&url, "tok", "a", "b", Some("c"), 123, 200);
        let req = handle.join().unwrap();
        let compact: String = req.split_whitespace().collect();
        assert!(
            req.starts_with("POST /apis/listenbrainz/1/submit-listens"),
            "{req}"
        );
        assert!(compact.contains("\"listen_type\":\"single\""), "{req}");
        assert!(compact.contains("\"listened_at\":123"), "{req}");
    }
}
