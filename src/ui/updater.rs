use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use gpui::{App, Global};
use parking_lot::RwLock;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};
use ureq::Agent;

#[derive(Debug, Clone, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(UpdateInfo),
    Downloading,
    Installing,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    #[serde(default)]
    pub pub_date: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    pub platforms: HashMap<String, PlatformAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformAsset {
    pub url: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Clone)]
pub struct Updater {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    agent: Agent,
    status: UpdateStatus,
    data_dir: PathBuf,
}

impl Global for Updater {}

impl Updater {
    pub fn new(data_dir: PathBuf) -> Self {
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            inner: Arc::new(RwLock::new(Inner {
                agent,
                status: UpdateStatus::Idle,
                data_dir,
            })),
        }
    }

    pub fn init(cx: &mut App, data_dir: PathBuf) {
        cx.set_global(Self::new(data_dir));
    }

    pub fn status(&self) -> UpdateStatus {
        self.inner.read().status.clone()
    }

    fn set_status(&self, s: UpdateStatus) {
        self.inner.write().status = s;
    }

    fn current_platform_key() -> &'static str {
        if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        }
    }

    pub fn check(&self, url: &str) -> Result<Option<UpdateInfo>> {
        self.set_status(UpdateStatus::Checking);

        #[cfg(target_os = "linux")]
        if !is_appimage() {
            debug!("not running as AppImage; updates disabled (use package manager)");
            self.set_status(UpdateStatus::UpToDate);
            return Ok(None);
        }

        let agent = self.inner.read().agent.clone();
        let info: UpdateInfo = agent
            .get(url)
            .call()
            .context("fetching update manifest")?
            .body_mut()
            .read_json()
            .context("parsing update manifest")?;

        let current =
            Version::parse(env!("CARGO_PKG_VERSION")).context("parsing current version")?;
        let remote = Version::parse(info.version.trim_start_matches('v'))
            .context("parsing remote version")?;

        if remote <= current {
            debug!("up to date ({} <= {})", remote, current);
            self.set_status(UpdateStatus::UpToDate);
            return Ok(None);
        }

        if !info.platforms.contains_key(Self::current_platform_key()) {
            warn!(
                "remote version {} has no asset for platform {}",
                remote,
                Self::current_platform_key()
            );
            self.set_status(UpdateStatus::UpToDate);
            return Ok(None);
        }

        info!("update available: {}", info.version);
        self.set_status(UpdateStatus::Available(info.clone()));
        Ok(Some(info))
    }

    pub fn download(&self, info: &UpdateInfo) -> Result<PathBuf> {
        self.set_status(UpdateStatus::Downloading);

        let asset = info
            .platforms
            .get(Self::current_platform_key())
            .ok_or_else(|| anyhow!("no asset for current platform"))?;

        let file_name = asset
            .url
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("update.bin")
            .to_string();
        let dir = self.inner.read().data_dir.join("updates");
        std::fs::create_dir_all(&dir)?;
        let target = dir.join(&file_name);

        let agent = self.inner.read().agent.clone();
        let bytes = agent
            .get(&asset.url)
            .call()
            .context("downloading update")?
            .body_mut()
            .read_to_vec()
            .context("reading download body")?;

        if let Some(expected) = asset.sha256.as_deref() {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let digest = hasher.finalize();
            let mut actual = String::with_capacity(digest.len() * 2);
            for b in digest.iter() {
                use std::fmt::Write as _;
                let _ = write!(actual, "{:02x}", b);
            }
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(anyhow!(
                    "sha256 mismatch: expected {expected}, got {actual}"
                ));
            }
        } else {
            warn!("no sha256 in manifest; skipping integrity check");
        }

        std::fs::write(&target, &bytes)?;
        Ok(target)
    }

    pub fn install_and_exit(&self, path: &Path) -> Result<()> {
        self.set_status(UpdateStatus::Installing);

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("msiexec")
                .args([
                    "/i",
                    path.to_str().context("non-utf8 path")?,
                    "/qb",
                    "/norestart",
                ])
                .spawn()
                .context("launching msiexec")?;
            std::process::exit(0);
        }

        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(path)
                .spawn()
                .context("launching open")?;
            std::process::exit(0);
        }

        #[cfg(target_os = "linux")]
        {
            replace_appimage(path)?;
            std::process::exit(0);
        }

        #[allow(unreachable_code)]
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn is_appimage() -> bool {
    std::env::var_os("APPIMAGE").is_some()
}

#[cfg(target_os = "linux")]
fn replace_appimage(new_file: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let current = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("APPIMAGE env var not set"))?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow!("AppImage has no parent dir"))?;

    let staged = parent.join(".vleer-new.AppImage");
    let _ = std::fs::remove_file(&staged);
    std::fs::copy(new_file, &staged).context("copying new AppImage next to current")?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&staged, &current).context("atomic swap of AppImage")?;

    std::process::Command::new(&current)
        .arg("--skip-single-instance")
        .spawn()?;
    Ok(())
}

pub fn is_managed_externally() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("APPIMAGE").is_none()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

pub fn run_check_in_background(updater: Updater, url: String) {
    std::thread::spawn(move || match updater.check(&url) {
        Ok(Some(info)) => info!("update {} available", info.version),
        Ok(None) => debug!("no update available"),
        Err(e) => {
            warn!("update check failed: {e}");
            updater.set_status(UpdateStatus::Failed(e.to_string()));
        }
    });
}
