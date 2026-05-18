use anyhow::Result;
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use ureq::Agent;
use std::{fs, path::PathBuf, time::Duration};
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::data::{config::Config, db::repo::Database};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Os {
    Windows,
    #[serde(rename = "macOS")]
    MacOS,
    Linux,
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
    agent: Agent,
    data_dir: PathBuf,
}

impl Global for Telemetry {}

impl Telemetry {
    pub fn init(cx: &mut App, data_dir: PathBuf) {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .build()
            .into();

        cx.set_global(Self { agent, data_dir });
    }

    pub fn submit(&self, db: &Database, config: &Config) {
        if !config.get().telemetry {
            debug!("Telemetry disabled in settings, skipping submission.");
            return;
        }

        let url = if cfg!(debug_assertions) {
            "http://localhost:3000/telemetry/v1"
        } else {
            "https://api.vleer.app/telemetry/v1"
        };

        let user_id = match self.get_or_create_user_id() {
            Ok(id) => id,
            Err(e) => {
                error!("Telemetry failed to get user_id: {e}");
                return;
            }
        };

        let payload = TelemetrySubmission {
            user_id,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: self.current_os(),
            song_count: db.get_songs_count(None).unwrap_or(0),
        };

        match self.agent.post(url).send_json(&payload) {
            Ok(res) if res.status().is_success() => info!("Telemetry sent"),
            Ok(res) => error!("Telemetry status: {}", res.status()),
            Err(e) if cfg!(debug_assertions) => debug!("Telemetry error (debug build): {e}"),
            Err(e) => error!("Telemetry error: {e}"),
        }
    }

    fn current_os(&self) -> Os {
        match std::env::consts::OS {
            "windows" => Os::Windows,
            "macos" => Os::MacOS,
            "linux" => Os::Linux,
            _ => Os::Unknown,
        }
    }

    fn get_or_create_user_id(&self) -> Result<Uuid> {
        let path = self.data_dir.join("user_id.txt");
        if let Ok(s) = fs::read_to_string(&path)
            && let Ok(id) = Uuid::parse_str(s.trim())
        {
            return Ok(id);
        }

        let id = Uuid::new_v4();
        if !self.data_dir.exists() {
            fs::create_dir_all(&self.data_dir)?;
        }
        fs::write(&path, id.to_string())?;
        Ok(id)
    }
}
