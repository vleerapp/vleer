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

const URL: &str = "https://api.vleer.app/update/v1/check";
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

pub type StatusSetFn =
    Arc<dyn Fn(&str, String, Option<f32>, crate::status::StatusColor) + Send + Sync>;
pub type StatusClearFn = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct Updater {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    agent: Agent,
    status: UpdateStatus,
    status_set: Option<StatusSetFn>,
    status_clear: Option<StatusClearFn>,
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
                status_set: None,
                status_clear: None,
            })),
        }
    }

    pub fn init(cx: &mut App, reporter: &'static crate::status::StatusReporter) {
        let updater = Self::new();
        updater.set_status_reporter(
            Arc::new(|key, text, ratio, color| reporter.set(key, text, ratio, color)),
            Arc::new(|key| reporter.clear(key)),
        );
        cx.set_global(updater);
    }

    pub fn status(&self) -> UpdateStatus {
        self.inner.read().status.clone()
    }

    fn set_status(&self, s: UpdateStatus) {
        self.inner.write().status = s;
    }

    pub fn set_status_reporter(&self, set: StatusSetFn, clear: StatusClearFn) {
        let mut inner = self.inner.write();
        inner.status_set = Some(set);
        inner.status_clear = Some(clear);
    }

    fn report_status(
        &self,
        key: &str,
        text: String,
        ratio: Option<f32>,
        color: crate::status::StatusColor,
    ) {
        if let Some(cb) = self.inner.read().status_set.clone() {
            cb(key, text, ratio, color);
        }
    }

    fn report_clear(&self, key: &str) {
        if let Some(cb) = self.inner.read().status_clear.clone() {
            cb(key);
        }
    }

    fn report_download(&self, current: u64, total: u64) {
        let mb = |b: u64| b as f64 / 1_048_576.0;
        let (text, ratio) = if total == 0 {
            (format!("Downloading: {:.1} MB", mb(current)), None)
        } else {
            let r = (current as f32 / total as f32).clamp(0.0, 1.0);
            (
                format!(
                    "Downloading: {:.1}/{:.1} MB - {:.0}%",
                    mb(current),
                    mb(total),
                    r * 100.0
                ),
                Some(r),
            )
        };
        self.report_status(
            "update.download",
            text,
            ratio,
            crate::status::StatusColor::Accent,
        );
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
        use std::io::{Read, Seek, Write};

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
        let target = update_cache_dir()?.join(&file_name);

        let agent = self.inner.read().agent.clone();
        let response = agent
            .get(&asset.url)
            .header("Accept-Encoding", "identity")
            .call()
            .context("downloading update")?;

        let total = asset.size.unwrap_or(0);
        self.report_download(0, total);

        let mut file = std::fs::File::create(&target).context("creating update file")?;
        let mut reader = response.into_body().into_reader();
        let mut buf = [0u8; 64 * 1024];
        let mut current: u64 = 0;
        loop {
            let n = reader.read(&mut buf).context("reading download body")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).context("writing download body")?;
            current += n as u64;
            self.report_download(current, total);
        }
        file.seek(std::io::SeekFrom::Start(0))
            .context("seeking update file for verification")?;
        let mut bytes = Vec::with_capacity(current as usize);
        file.read_to_end(&mut bytes)
            .context("reading update file for verification")?;
        drop(file);

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
            self.report_clear("update.download");
            return Err(e);
        }

        self.report_clear("update.download");

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
        Where-Object {{ $_.DisplayName -eq 'Vleer' }} |
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

            let script_path = update_cache_dir()?.join("vleer_update.ps1");
            let _ = std::fs::remove_file(&script_path);
            {
                use std::io::Write as _;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&script_path)
                    .context("writing update script")?;
                f.write_all(script.as_bytes())
                    .context("writing update script")?;
            }

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

    let script_path = update_cache_dir()?.join(format!("vleer_update_{pid}.sh"));
    let _ = std::fs::remove_file(&script_path);
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&script_path)
            .context("writing swap script")?;
        f.write_all(script.as_bytes())
            .context("writing swap script")?;
    }

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

fn update_cache_dir() -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| anyhow!("cannot determine user cache directory"))?
        .join("vleer")
        .join("updates");
    std::fs::create_dir_all(&dir).context("creating update cache directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .context("setting update cache directory permissions")?;
    }
    Ok(dir)
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
