use anyhow::Result;
use gpui::{App, Global};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::data::{config::Config, state::State};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Os {
    #[serde(rename = "Windows")]
    Windows,

    #[serde(rename = "macOS")]
    MacOs,

    #[serde(rename = "Linux")]
    Linux,

    #[serde(rename = "Unknown")]
    Unknown,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TelemetrySubmission {
    pub user_id: Uuid,
    pub app_version: String,
    pub os: Os,
    pub song_count: i64,
}

#[derive(Clone)]
pub struct Telemetry {
    client: Client,
    data_dir: PathBuf,
}

impl Global for Telemetry {}

impl Telemetry {
    pub fn init(cx: &mut App, data_dir: PathBuf) {
        cx.set_global(Self {
            client: Client::new(),
            data_dir,
        });
    }

    pub async fn submit(&self, state: &State, config: &Config) {
        if !config.get().telemetry {
            debug!("Telemetry disabled in settings, skipping submission.");
            return;
        }

        let song_count = state.get_all_song_ids().await.len();
        let url = if cfg!(debug_assertions) {
            "http://localhost:3000/v1/telemetry"
        } else {
            "https://api.vleer.app/v1/telemetry"
        };

        let user_id = match self.get_or_create_user_id() {
            Ok(id) => id,
            Err(e) => return error!("Telemetry failed to get user_id: {e}"),
        };

        let payload = TelemetrySubmission {
            user_id,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: self.current_os(),
            song_count: song_count as i64,
        };

        match self.client.post(url).json(&payload).send().await {
            Ok(res) if !res.status().is_success() => error!("Telemetry status: {}", res.status()),
            Err(e) => error!("Telemetry error: {e}"),
            _ => info!("Telemetry sent"),
        }
    }

    fn current_os(&self) -> Os {
        match std::env::consts::OS {
            "windows" => Os::Windows,
            "macos" => Os::MacOs,
            "linux" => Os::Linux,
            _ => Os::Unknown,
        }
    }

    fn get_or_create_user_id(&self) -> Result<Uuid> {
        let path = self.data_dir.join("user_id.txt");
        if let Ok(s) = fs::read_to_string(&path) {
            if let Ok(id) = Uuid::parse_str(s.trim()) {
                return Ok(id);
            }
        }

        let id = Uuid::new_v4();
        if !self.data_dir.exists() {
            fs::create_dir_all(&self.data_dir)?;
        }
        fs::write(&path, id.to_string())?;
        Ok(id)
    }
}
