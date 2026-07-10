use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use gpui::{App, Global};
use openpgp::Cert;
use openpgp::parse::Parse;
use openpgp::parse::stream::{
    DetachedVerifierBuilder, MessageLayer, MessageStructure, VerificationHelper,
};
use openpgp::policy::StandardPolicy;
use parking_lot::RwLock;
use semver::Version;
use sequoia_openpgp as openpgp;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use ureq::Agent;

const URL: &str = "http://localhost:3000/update/v1/check";
const PUBLIC_KEY: &[u8] = include_bytes!("../assets/key.asc");

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
}

#[derive(Clone)]
pub struct Updater {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    agent: Agent,
    status: UpdateStatus,
}

impl Global for Updater {}

impl Updater {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let agent: Agent = Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_send_request(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(10)))
            .timeout_recv_body(None)
            .build()
            .into();
        Self {
            inner: Arc::new(RwLock::new(Inner {
                agent,
                status: UpdateStatus::Idle,
            })),
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(Self::new());
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
        let target = std::env::temp_dir().join(&file_name);

        let agent = self.inner.read().agent.clone();
        let response = agent
            .get(&asset.url)
            .header("Accept-Encoding", "identity")
            .call()
            .context("downloading update")?;

        let mut file = std::fs::File::create(&target).context("creating update file")?;
        let mut reader = response.into_body().into_reader();
        std::io::copy(&mut reader, &mut file).context("writing download body")?;
        drop(file);

        let bytes = std::fs::read(&target).context("reading update file for verification")?;

        let sig_url = format!("{}.sig", asset.url);
        let sig_bytes = agent
            .get(&sig_url)
            .call()
            .context("downloading signature")?
            .body_mut()
            .read_to_vec()
            .context("reading signature")?;

        if let Err(e) = verify_signature(PUBLIC_KEY, &bytes, &sig_bytes) {
            let _ = std::fs::remove_file(&target);
            return Err(e);
        }

        Ok(target)
    }

    pub fn install_and_exit(&self, path: &Path) -> Result<()> {
        self.set_status(UpdateStatus::Installing);

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;

            let msi_path = path.to_str().context("non-utf8 path")?;
            let current_exe = std::env::current_exe().context("getting current exe")?;
            let fallback_exe = current_exe.to_str().context("non-utf8 exe path")?;

            let script = format!(
                r#"Start-Sleep -Milliseconds 500
Start-Process msiexec.exe -ArgumentList @('/i', '{msi}', '/passive', '/norestart') -Wait
$exe = $null
$roots = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
         'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
foreach ($root in $roots) {{
    if ($exe) {{ break }}
    Get-ChildItem $root -ErrorAction SilentlyContinue |
        Get-ItemProperty |
        Where-Object {{ $_.DisplayName -like '*leer*' }} |
        ForEach-Object {{
            if ($_.InstallLocation) {{
                $c = Join-Path $_.InstallLocation 'vleer.exe'
                if (Test-Path $c) {{ $exe = $c }}
            }}
        }}
}}
if ($exe) {{ Start-Process $exe }}
elseif (Test-Path '{fallback}') {{ Start-Process '{fallback}' }}
"#,
                msi = msi_path.replace('\'', "''"),
                fallback = fallback_exe.replace('\'', "''"),
            );

            let script_path = std::env::temp_dir().join("vleer_update.ps1");
            std::fs::write(&script_path, &script).context("writing update script")?;

            std::process::Command::new("powershell")
                .creation_flags(CREATE_NO_WINDOW)
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    script_path.to_str().context("non-utf8 script path")?,
                ])
                .spawn()
                .context("launching update script")?;

            std::process::exit(0);
        }

        #[cfg(target_os = "macos")]
        {
            replace_macos_app(path)?;
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

pub fn verify_signature(public_key: &[u8], data: &[u8], sig_bytes: &[u8]) -> Result<()> {
    let cert = Cert::from_bytes(public_key).context("invalid public key")?;
    let policy = StandardPolicy::new();
    let helper = Helper { cert };
    let mut verifier = DetachedVerifierBuilder::from_bytes(sig_bytes)
        .context("parsing signature")?
        .with_policy(&policy, None, helper)
        .context("initializing verifier")?;
    verifier
        .verify_bytes(data)
        .context("signature verification failed")?;
    Ok(())
}

struct Helper {
    cert: Cert,
}

impl VerificationHelper for Helper {
    fn get_certs(&mut self, _ids: &[openpgp::KeyHandle]) -> openpgp::Result<Vec<Cert>> {
        Ok(vec![self.cert.clone()])
    }

    fn check(&mut self, structure: MessageStructure) -> openpgp::Result<()> {
        for layer in structure {
            if let MessageLayer::SignatureGroup { results } = layer {
                for result in results {
                    if result.is_ok() {
                        return Ok(());
                    }
                }
                return Err(anyhow!("no valid signature"));
            }
        }
        Err(anyhow!("no signature layer"))
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
    let _ = std::fs::remove_file(new_file);

    let pid = std::process::id();
    let current_s = current.to_string_lossy().replace('\'', "'\"'\"'");
    let staged_s = staged.to_string_lossy().replace('\'', "'\"'\"'");
    let script = format!(
        "for i in $(seq 1 50); do\n  kill -0 {pid} 2>/dev/null || break\n  sleep 0.2\ndone\n\
mv -f -- '{staged_s}' '{current_s}'\nchmod 0755 -- '{current_s}'\nexec '{current_s}' --skip-single-instance\n"
    );

    let script_path = std::env::temp_dir().join(format!("vleer_update_{pid}.sh"));
    std::fs::write(&script_path, &script).context("writing swap script")?;
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;

    std::process::Command::new("sh")
        .arg(&script_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("launching swap script")?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn replace_macos_app(dmg_path: &Path) -> Result<()> {
    use std::process::Command;

    let output = Command::new("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-noautoopen",
            dmg_path.to_str().context("non-utf8 dmg path")?,
        ])
        .output()
        .context("mounting DMG")?;

    if !output.status.success() {
        return Err(anyhow!(
            "hdiutil attach failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mount_point = stdout
        .lines()
        .filter_map(|line| line.split('\t').next_back())
        .find(|s| s.trim().starts_with("/Volumes/"))
        .map(|s| PathBuf::from(s.trim()))
        .ok_or_else(|| anyhow!("could not find mount point in hdiutil output"))?;

    let app_in_dmg = std::fs::read_dir(&mount_point)
        .context("reading mounted DMG")?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "app"))
        .map(|e| e.path())
        .ok_or_else(|| anyhow!("no .app found in DMG"))?;

    let current_exe = std::env::current_exe().context("getting current exe")?;
    let install_dir = current_exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .and_then(|bundle| bundle.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/Applications"));
    let app_in_dmg_name = app_in_dmg
        .file_name()
        .ok_or_else(|| anyhow!(".app in DMG has no file name"))?;
    let dest = install_dir.join(app_in_dmg_name);

    let staged = install_dir.join(".vleer-new.app");
    let _ = std::fs::remove_dir_all(&staged);
    let app_src = app_in_dmg.to_str().context("non-utf8 .app source path")?;
    let staged_dst = staged.to_str().context("non-utf8 staged dest path")?;
    let status = Command::new("ditto")
        .args([app_src, staged_dst])
        .status()
        .context("copying .app with ditto")?;
    if !status.success() {
        return Err(anyhow!("ditto failed"));
    }

    if let Some(mount_str) = mount_point.to_str() {
        let _ = Command::new("hdiutil").args(["detach", mount_str]).status();
    }

    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&staged, &dest).context("swapping .app into place")?;

    Command::new("open")
        .arg(&dest)
        .spawn()
        .context("launching new app")?;

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

pub fn run_check_in_background(updater: Updater, executor: &gpui::BackgroundExecutor) {
    executor
        .spawn(async move {
            match updater.check(URL) {
                Ok(Some(info)) => info!("update {} available", info.version),
                Ok(None) => debug!("no update available"),
                Err(e) => {
                    warn!("update check failed: {e}");
                    updater.set_status(UpdateStatus::Failed(e.to_string()));
                }
            }
        })
        .detach();
}
