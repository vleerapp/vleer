use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use gpui::{App, Global};
use serde::{Deserialize, Serialize, Serializer};
use tracing::{debug, info, warn};

type SaveJob = (PathBuf, String);

fn save_worker() -> &'static std::sync::mpsc::Sender<SaveJob> {
    static TX: OnceLock<std::sync::mpsc::Sender<SaveJob>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<SaveJob>();
        std::thread::Builder::new()
            .name("vleer-config-save".to_string())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    while let Ok(next) = rx.try_recv() {
                        job = next;
                    }
                    if let Err(e) = fs::write(&job.0, &job.1) {
                        warn!("Failed to write config file: {}", e);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
            .expect("failed to spawn config save thread");
        tx
    })
}

fn round_hundredths(value: f32) -> f64 {
    let rounded = (value * 100.0).round() / 100.0 + 0.0;
    format!("{rounded}").parse().unwrap_or(f64::from(value))
}

fn serialize_rounded<S: Serializer>(values: &[f32], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(values.iter().map(|v| round_hundredths(*v)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqualizerSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub frequencies: Vec<i32>,
    #[serde(default, serialize_with = "serialize_rounded")]
    pub gains: Vec<f32>,
    #[serde(default)]
    pub q_values: Vec<f32>,
}

impl Default for EqualizerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            frequencies: vec![32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000],
            gains: vec![0.0; 10],
            q_values: vec![crate::media::equalizer::Q_DEFAULT; 10],
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeneralSettings {
    #[serde(default)]
    pub tray_icon: bool,
    #[serde(default)]
    pub close_to_tray: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySettings {
    #[serde(default)]
    pub paths: Vec<String>,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            paths: dirs::audio_dir()
                .map(|p| p.to_string_lossy().to_string())
                .into_iter()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u32", into = "u32")]
pub enum FftSize {
    S1024,
    S2048,
    S4096,
    #[default]
    S8192,
    S16384,
}

impl FftSize {
    pub const ALL: [Self; 5] = [
        Self::S1024,
        Self::S2048,
        Self::S4096,
        Self::S8192,
        Self::S16384,
    ];

    pub fn samples(self) -> usize {
        match self {
            Self::S1024 => 1024,
            Self::S2048 => 2048,
            Self::S4096 => 4096,
            Self::S8192 => 8192,
            Self::S16384 => 16384,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::S1024 => "1024 (fastest)",
            Self::S2048 => "2048",
            Self::S4096 => "4096",
            Self::S8192 => "8192 (balanced)",
            Self::S16384 => "16384 (most detailed)",
        }
    }
}

impl From<u32> for FftSize {
    fn from(value: u32) -> Self {
        Self::ALL
            .into_iter()
            .find(|size| size.samples() == value as usize)
            .unwrap_or_else(|| {
                warn!("Unsupported fft_size {value}, using default");
                Self::default()
            })
    }
}

impl From<FftSize> for u32 {
    fn from(size: FftSize) -> Self {
        size.samples() as u32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectrumSettings {
    #[serde(default = "defaults::spectrum")]
    pub enabled: bool,
    #[serde(default)]
    pub peak_caps: bool,
    #[serde(default)]
    pub fft_size: FftSize,
}

impl Default for SpectrumSettings {
    fn default() -> Self {
        Self {
            enabled: defaults::spectrum(),
            peak_caps: false,
            fft_size: FftSize::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    #[serde(default = "defaults::visualizer")]
    pub visualizer: bool,
    #[serde(default)]
    pub spectrum: SpectrumSettings,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            visualizer: defaults::visualizer(),
            spectrum: SpectrumSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackSettings {
    #[serde(default = "defaults::volume")]
    pub volume: f32,
    #[serde(default)]
    pub equalizer: EqualizerSettings,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            volume: defaults::volume(),
            equalizer: EqualizerSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrivacySettings {
    #[serde(default)]
    pub telemetry: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscordSettings {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastfmSettings {
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default = "defaults::scrobble_threshold")]
    pub scrobble_threshold: f32,
}

impl Default for LastfmSettings {
    fn default() -> Self {
        Self {
            session_key: None,
            username: None,
            scrobble_threshold: defaults::scrobble_threshold(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrationsSettings {
    #[serde(default)]
    pub discord: DiscordSettings,
    #[serde(default)]
    pub lastfm: LastfmSettings,
}

pub use crate::updater::UpdateChannel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesSettings {
    #[serde(default = "defaults::auto_check")]
    pub auto_check: bool,
    #[serde(default = "defaults::channel")]
    pub channel: UpdateChannel,
}

impl Default for UpdatesSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            channel: defaults::channel(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsConfig {
    #[serde(default = "defaults::version")]
    pub version: u32,
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub library: LibrarySettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub playback: PlaybackSettings,
    #[serde(default)]
    pub privacy: PrivacySettings,
    #[serde(default)]
    pub integrations: IntegrationsSettings,
    #[serde(default)]
    pub updates: UpdatesSettings,
}

mod defaults {
    pub fn version() -> u32 {
        2
    }
    pub fn visualizer() -> bool {
        true
    }
    pub fn spectrum() -> bool {
        true
    }
    pub fn volume() -> f32 {
        0.5
    }
    pub fn auto_check() -> bool {
        true
    }
    pub fn channel() -> super::UpdateChannel {
        super::UpdateChannel::of_running_build()
    }
    pub fn scrobble_threshold() -> f32 {
        0.5
    }
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            version: defaults::version(),
            general: GeneralSettings::default(),
            library: LibrarySettings::default(),
            appearance: AppearanceSettings::default(),
            playback: PlaybackSettings::default(),
            privacy: PrivacySettings::default(),
            integrations: IntegrationsSettings::default(),
            updates: UpdatesSettings::default(),
        }
    }
}

#[derive(Deserialize)]
struct LegacyAudio {
    #[serde(default = "defaults::visualizer")]
    visualizer: bool,
    #[serde(default = "defaults::spectrum")]
    spectrum: bool,
    #[serde(default = "defaults::volume")]
    volume: f32,
}

impl Default for LegacyAudio {
    fn default() -> Self {
        Self {
            visualizer: defaults::visualizer(),
            spectrum: defaults::spectrum(),
            volume: defaults::volume(),
        }
    }
}

#[derive(Deserialize)]
struct LegacyConfig {
    #[serde(default)]
    telemetry: bool,
    #[serde(default)]
    discord_rpc: bool,
    #[serde(default)]
    equalizer: EqualizerSettings,
    #[serde(default)]
    scan: LibrarySettings,
    #[serde(default)]
    audio: LegacyAudio,
    #[serde(default)]
    updater: UpdatesSettings,
    #[serde(default)]
    lastfm: LastfmSettings,
}

impl From<LegacyConfig> for SettingsConfig {
    fn from(legacy: LegacyConfig) -> Self {
        Self {
            version: defaults::version(),
            general: GeneralSettings::default(),
            library: legacy.scan,
            appearance: AppearanceSettings {
                visualizer: legacy.audio.visualizer,
                spectrum: SpectrumSettings {
                    enabled: legacy.audio.spectrum,
                    ..SpectrumSettings::default()
                },
            },
            playback: PlaybackSettings {
                volume: legacy.audio.volume,
                equalizer: legacy.equalizer,
            },
            privacy: PrivacySettings {
                telemetry: legacy.telemetry,
            },
            integrations: IntegrationsSettings {
                discord: DiscordSettings {
                    enabled: legacy.discord_rpc,
                },
                lastfm: legacy.lastfm,
            },
            updates: legacy.updater,
        }
    }
}

fn parse(content: &str) -> Result<(SettingsConfig, bool), toml::de::Error> {
    let value: toml::Value = toml::from_str(content)?;
    let version = value
        .get("version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1);

    if version < i64::from(defaults::version()) {
        debug!(
            "Migrating config from version {} to {}",
            version,
            defaults::version()
        );
        let legacy: LegacyConfig = value.try_into()?;
        Ok((legacy.into(), true))
    } else {
        Ok((value.try_into()?, false))
    }
}

#[derive(Clone)]
pub struct Config {
    config: SettingsConfig,
    config_path: PathBuf,
    pub parse_warning: Option<String>,
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

        let mut parse_warning: Option<String> = None;
        let mut needs_save = false;
        let mut config = if config_path.exists() {
            let content = fs::read_to_string(&config_path).context("Failed to read config file")?;

            match parse(&content) {
                Ok((config, migrated)) => {
                    needs_save = migrated;
                    config
                }
                Err(e) => {
                    warn!("Failed to parse config file: {}", e);
                    parse_warning = Some("Settings file is corrupted, using defaults".to_string());
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

        Self::validate_equalizer(&mut config.playback.equalizer);

        let config = Self {
            config,
            config_path,
            parse_warning,
        };

        if needs_save {
            config.save().context("Failed to save migrated config")?;
        }

        Ok(config)
    }

    fn validate_equalizer(eq: &mut EqualizerSettings) {
        if eq.frequencies.len() != 10 {
            warn!(
                "Equalizer frequencies array has {} items, expected 10. Resetting to defaults.",
                eq.frequencies.len()
            );
            eq.frequencies = EqualizerSettings::default().frequencies;
        }
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

    pub fn set(&mut self, f: impl FnOnce(&mut SettingsConfig)) {
        if self.parse_warning.is_some() {
            warn!("Config has parse errors, skipping write to preserve file");
            return;
        }
        f(&mut self.config);
        self.config.playback.volume = self.config.playback.volume.clamp(0.0, 1.0);
        self.config.integrations.lastfm.scrobble_threshold = self
            .config
            .integrations
            .lastfm
            .scrobble_threshold
            .clamp(0.05, 1.0);
        let library_paths: Vec<String> = self
            .config
            .library
            .paths
            .iter()
            .filter(|p| !p.is_empty())
            .cloned()
            .collect();
        if !library_paths.is_empty() {
            self.config.library.paths = library_paths;
        }
        Self::validate_equalizer(&mut self.config.playback.equalizer);
        self.save_in_background();
    }

    fn save_in_background(&self) {
        let mut config = self.config.clone();
        Self::validate_equalizer(&mut config.playback.equalizer);
        match toml::to_string_pretty(&config) {
            Ok(content) => {
                let _ = save_worker().send((self.config_path.clone(), content));
            }
            Err(e) => warn!("Failed to serialize config: {}", e),
        }
    }

    pub fn save(&self) -> Result<()> {
        debug!("Saving config to {:?}", self.config_path);

        let mut config = self.config.clone();
        Self::validate_equalizer(&mut config.playback.equalizer);

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

            match parse(&content) {
                Ok((mut config, _)) => {
                    Self::validate_equalizer(&mut config.playback.equalizer);
                    self.config = config;
                    self.parse_warning = None;

                    info!("Config reloaded successfully");
                }
                Err(e) => {
                    self.parse_warning =
                        Some("Settings file is corrupted, using defaults".to_string());
                    warn!("Failed to parse config file during reload: {}", e);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_v1_layout() {
        let old = r#"
telemetry = true
discord_rpc = true
[audio]
visualizer = false
spectrum = false
volume = 0.25
[scan]
paths = ["/music"]
[lastfm]
username = "a"
scrobble_threshold = 0.7
[updater]
auto_check = false
channel = "stable"
"#;
        let (config, migrated) = parse(old).unwrap();
        assert!(migrated);
        assert_eq!(config.version, 2);
        assert!(config.privacy.telemetry);
        assert!(config.integrations.discord.enabled);
        assert!(!config.appearance.visualizer);
        assert!(!config.appearance.spectrum.enabled);
        assert_eq!(config.appearance.spectrum.fft_size, FftSize::S8192);
        assert_eq!(config.playback.volume, 0.25);
        assert_eq!(config.library.paths, vec!["/music".to_string()]);
        assert_eq!(config.integrations.lastfm.username.as_deref(), Some("a"));
        assert!(!config.updates.auto_check);
    }

    #[test]
    fn round_trips_and_rounds_gains() {
        let mut config = SettingsConfig::default();
        config.playback.equalizer.gains[0] = 3.5235;
        config.playback.equalizer.gains[1] = -0.001;
        config.appearance.spectrum.fft_size = FftSize::S2048;
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("3.52,"), "{text}");
        assert!(text.contains("fft_size = 2048"), "{text}");
        let (back, migrated) = parse(&text).unwrap();
        assert!(!migrated);
        assert_eq!(back.appearance.spectrum.fft_size, FftSize::S2048);
    }

    #[test]
    fn unknown_fft_size_falls_back() {
        let (config, _) = parse("version = 2\n[appearance.spectrum]\nfft_size = 3000\n").unwrap();
        assert_eq!(config.appearance.spectrum.fft_size, FftSize::S8192);
    }
}
