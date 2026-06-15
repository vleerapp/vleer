use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

use vleer::ui::updater::{PlatformAsset, UpdateInfo, Updater, is_managed_externally};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn tmp_dir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from(format!("/tmp/vleer_upd_test_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[allow(dead_code)]
fn serve_json(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn serve_bytes(data: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            let _ = s.write_all(&data);
        }
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as FmtWrite;
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    let mut out = String::with_capacity(64);
    for b in d.iter() {
        let _ = write!(out, "{:02x}", b);
    }
    out
}

fn make_info(url: String, sha256: Option<String>) -> UpdateInfo {
    let mut platforms = HashMap::new();
    platforms.insert(
        platform().to_string(),
        PlatformAsset {
            url,
            size: None,
            sha256,
        },
    );
    UpdateInfo {
        version: "99.0.0".to_string(),
        pub_date: None,
        notes_url: None,
        platforms,
    }
}

#[test]
fn parse_manifest_minimal() {
    let json = r#"{"version":"1.0.0","platforms":{}}"#;
    let info: UpdateInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.version, "1.0.0");
    assert!(info.pub_date.is_none());
    assert!(info.notes_url.is_none());
    assert!(info.platforms.is_empty());
}

#[test]
fn parse_manifest_full() {
    let json = r#"{
        "version": "2.3.1",
        "pub_date": "2025-06-01",
        "notes_url": "https://example.com/notes",
        "platforms": {
            "macos":   {"url": "https://example.com/app.dmg", "size": 102400, "sha256": "abc"},
            "windows": {"url": "https://example.com/app.msi"},
            "linux":   {"url": "https://example.com/app.AppImage", "sha256": "def"}
        }
    }"#;
    let info: UpdateInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.version, "2.3.1");
    assert_eq!(info.pub_date.as_deref(), Some("2025-06-01"));
    assert_eq!(info.notes_url.as_deref(), Some("https://example.com/notes"));
    assert_eq!(info.platforms.len(), 3);
    assert_eq!(info.platforms["macos"].size, Some(102400));
    assert_eq!(info.platforms["macos"].sha256.as_deref(), Some("abc"));
    assert!(info.platforms["windows"].sha256.is_none());
    assert!(info.platforms["windows"].size.is_none());
}

#[test]
fn parse_manifest_invalid_json() {
    assert!(serde_json::from_str::<UpdateInfo>("not json").is_err());
}

#[test]
fn parse_manifest_missing_version() {
    assert!(serde_json::from_str::<UpdateInfo>(r#"{"platforms":{}}"#).is_err());
}

#[test]
fn parse_manifest_v_prefix_strips_for_semver() {
    let v = "v3.1.0";
    let parsed = semver::Version::parse(v.trim_start_matches('v')).unwrap();
    assert_eq!(parsed, semver::Version::new(3, 1, 0));
}

#[test]
#[cfg(not(target_os = "linux"))]
fn not_managed_externally_on_non_linux() {
    assert!(!is_managed_externally());
}

#[test]
#[cfg(target_os = "linux")]
fn managed_externally_without_appimage_env() {
    unsafe { std::env::remove_var("APPIMAGE") };
    assert!(is_managed_externally());
}

#[test]
fn sha256_empty_string_known_hash() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_output_is_64_lowercase_hex_chars() {
    let h = sha256_hex(b"vleer update test");
    assert_eq!(h.len(), 64);
    assert!(h.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
}

#[test]
#[cfg(not(target_os = "linux"))]
fn check_none_when_same_version() {
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let current = env!("CARGO_PKG_VERSION");
    let json = format!(
        r#"{{"version":"{}","platforms":{{"{}":{{"url":"https://x.test/f"}}}}}}"#,
        current,
        platform()
    );
    assert!(updater.check(&serve_json(json)).unwrap().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[cfg(not(target_os = "linux"))]
fn check_none_when_older_version() {
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let json = format!(
        r#"{{"version":"0.0.1","platforms":{{"{}":{{"url":"https://x.test/f"}}}}}}"#,
        platform()
    );
    assert!(updater.check(&serve_json(json)).unwrap().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[cfg(not(target_os = "linux"))]
fn check_some_when_newer_version_with_platform_asset() {
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let json = format!(
        r#"{{"version":"99.0.0","platforms":{{"{}":{{"url":"https://x.test/f"}}}}}}"#,
        platform()
    );
    let info = updater.check(&serve_json(json)).unwrap().unwrap();
    assert_eq!(info.version, "99.0.0");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[cfg(not(target_os = "linux"))]
fn check_none_when_newer_version_missing_platform_asset() {
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let other = if platform() == "macos" {
        "windows"
    } else {
        "macos"
    };
    let json = format!(
        r#"{{"version":"99.0.0","platforms":{{"{}":{{"url":"https://x.test/f"}}}}}}"#,
        other
    );
    assert!(updater.check(&serve_json(json)).unwrap().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[cfg(not(target_os = "linux"))]
fn check_error_on_invalid_json_response() {
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    assert!(
        updater
            .check(&serve_json("not json at all".to_string()))
            .is_err()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[cfg(target_os = "linux")]
fn check_none_on_linux_without_appimage_env() {
    unsafe { std::env::remove_var("APPIMAGE") };
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    assert!(updater.check("http://127.0.0.1:1").unwrap().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_succeeds_without_sha256() {
    let data = b"fake installer bytes".to_vec();
    let url = serve_bytes(data);
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let path = updater.download(&make_info(url, None)).unwrap();
    assert!(path.exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_succeeds_with_correct_sha256() {
    let data = b"exact installer payload".to_vec();
    let checksum = sha256_hex(&data);
    let url = serve_bytes(data);
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let path = updater.download(&make_info(url, Some(checksum))).unwrap();
    assert!(path.exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_rejects_wrong_sha256() {
    let data = b"some installer".to_vec();
    let url = serve_bytes(data);
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let bad = "0".repeat(64);
    let err = updater.download(&make_info(url, Some(bad))).unwrap_err();
    assert!(
        err.to_string().contains("sha256 mismatch"),
        "unexpected error: {err}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_file_placed_under_updates_subdir() {
    let data = b"payload".to_vec();
    let url = serve_bytes(data);
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let path = updater.download(&make_info(url, None)).unwrap();
    assert!(path.starts_with(dir.join("updates")));
    std::fs::remove_dir_all(&dir).ok();
}
