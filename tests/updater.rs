use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

use vleer::updater::{PlatformAsset, UpdateInfo, Updater, is_managed_externally, verify_signature};

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

fn serve_update(binary: Vec<u8>, sig: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for _ in 0..2 {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                if req.contains(".minisig") {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        sig.len(),
                        sig
                    );
                    let _ = s.write_all(resp.as_bytes());
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        binary.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(&binary);
                }
            }
        }
    });
    format!("http://127.0.0.1:{}/vleer.bin", addr.port())
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

fn make_info(url: String) -> UpdateInfo {
    let mut platforms = HashMap::new();
    platforms.insert(platform().to_string(), PlatformAsset { url, size: None });
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
            "macos":   {"url": "https://example.com/app.dmg", "size": 102400},
            "windows": {"url": "https://example.com/app.msi"},
            "linux":   {"url": "https://example.com/app.AppImage"}
        }
    }"#;
    let info: UpdateInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.version, "2.3.1");
    assert_eq!(info.pub_date.as_deref(), Some("2025-06-01"));
    assert_eq!(info.notes_url.as_deref(), Some("https://example.com/notes"));
    assert_eq!(info.platforms.len(), 3);
    assert_eq!(info.platforms["macos"].size, Some(102400));
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
fn verify_signature_rejects_invalid_public_key() {
    assert!(verify_signature("not_a_key", b"data", "not_a_sig").is_err());
}

#[test]
fn verify_signature_rejects_malformed_sig_content() {
    let fake_key =
        "RWAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    assert!(verify_signature(fake_key, b"data", "not a minisig").is_err());
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
    assert!(updater.check(&serve_json("not json".to_string())).is_err());
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
fn download_rejects_malformed_sig() {
    let data = b"fake installer".to_vec();
    let url = serve_update(data, "not a valid minisig file".to_string());
    let dir = tmp_dir();
    let updater = Updater::new(dir.clone());
    let err = updater.download(&make_info(url)).unwrap_err();
    assert!(
        err.to_string().contains("signature") || err.to_string().contains("invalid"),
        "unexpected error: {err}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn download_file_placed_under_updates_subdir() {
    let data = b"payload".to_vec();
    let url = serve_update(data, "bad sig".to_string());
    let dir = tmp_dir();

    let updater = Updater::new(dir.clone());
    let _ = updater.download(&make_info(url));
    assert!(dir.join("updates").exists());
    std::fs::remove_dir_all(&dir).ok();
}
