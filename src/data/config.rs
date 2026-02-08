use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gpui::{App, Global};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqualizerSettings {
    pub enabled: bool,
    pub frequencies: Vec<i32>,
    pub gains: Vec<f32>,
    pub q_values: Vec<f32>,
}

impl Default for EqualizerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frequencies: vec![32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000],
            gains: vec![0.0; 10],
            q_values: vec![1.461; 10],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSettings {
    pub paths: Vec<String>,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            paths: vec!["~/Music".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSettings {
    pub normalization: bool,
    pub volume: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            normalization: true,
            volume: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    pub telemetry: bool,
    pub discord_rpc: bool,
    pub equalizer: EqualizerSettings,
    pub scan: ScanSettings,
    pub audio: AudioSettings,
}

fn default_version() -> u32 {
    1
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            telemetry: true,
            discord_rpc: false,
            equalizer: EqualizerSettings::default(),
            scan: ScanSettings::default(),
            audio: AudioSettings::default(),
        }
    }
}

#[derive(Clone)]
pub struct Config {
    config: SettingsConfig,
    config_path: PathBuf,
}

impl Global for Config {}

impl Config {
    pub fn init(cx: &mut App, config_dir: impl AsRef<Path>) -> Result<()> {
        let config = Self::load(config_dir)?;
        cx.set_global(config);
        Ok(())
    }

    pub fn load(config_dir: impl AsRef<Path>) -> Result<Self> {
        let config_dir = config_dir.as_ref();
        fs::create_dir_all(config_dir).context("Failed to create config directory")?;

        let config_path = config_dir.join("config.toml");
        debug!("Loading config from {:?}", config_path);

        let mut config = if config_path.exists() {
            let content = fs::read_to_string(&config_path).context("Failed to read config file")?;

            match toml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    warn!("Failed to parse config file: {}. Using defaults.", e);
                    SettingsConfig::default()
                }
            }
        } else {
            debug!("Config file not found, creating default");
            let config = SettingsConfig::default();

            let content =
                toml::to_string_pretty(&config).context("Failed to serialize default config")?;
            fs::write(&config_path, content).context("Failed to write default config file")?;

            config
        };

        Self::validate_equalizer(&mut config.equalizer);
        let needs_save = config.version < default_version();
        Self::migrate_config(&mut config);

        let config = Self {
            config,
            config_path,
        };

        if needs_save {
            config.save().context("Failed to save migrated config")?;
        }

        Ok(config)
    }

    fn migrate_config(config: &mut SettingsConfig) {
        const CURRENT_VERSION: u32 = 1;

        if config.version < CURRENT_VERSION {
            debug!(
                "Migrating config from version {} to {}",
                config.version, CURRENT_VERSION
            );

            config.version = CURRENT_VERSION;
        }
    }

    fn validate_equalizer(eq: &mut EqualizerSettings) {
        if eq.gains.len() != 10 {
            warn!(
                "Equalizer gains array has {} items, expected 10. Resetting to defaults.",
                eq.gains.len()
            );
            eq.gains = vec![0.0; 10];
        }
        if eq.q_values.len() != 10 {
            warn!(
                "Equalizer q_values array has {} items, expected 10. Resetting to defaults.",
                eq.q_values.len()
            );
            eq.q_values = vec![1.461; 10];
        }
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.config.audio.volume = vol;
        self.save().ok();
    }

    pub fn save(&self) -> Result<()> {
        debug!("Saving config to {:?}", self.config_path);

        let mut config = self.config.clone();
        Self::validate_equalizer(&mut config.equalizer);

        let content = toml::to_string_pretty(&config).context("Failed to serialize config")?;
        fs::write(&self.config_path, content).context("Failed to write config file")?;

        Ok(())
    }

    pub fn get(&self) -> &SettingsConfig {
        &self.config
    }

    pub fn reload(&mut self) -> Result<()> {
        if self.config_path.exists() {
            let content =
                fs::read_to_string(&self.config_path).context("Failed to read config file")?;

            match toml::from_str::<SettingsConfig>(&content) {
                Ok(mut config) => {
                    Self::validate_equalizer(&mut config.equalizer);
                    self.config = config;

                    info!("Config reloaded successfully");
                }
                Err(e) => {
                    warn!("Failed to parse config file during reload: {}", e);
                }
            }
        }

        Ok(())
    }
}
