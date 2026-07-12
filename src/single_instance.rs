use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Stream};

const APP_ID: &str = "vleer";

#[cfg(unix)]
fn socket_path() -> String {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime)
            .join(format!("{APP_ID}.sock"))
            .to_string_lossy()
            .into_owned();
    }
    let uid = unsafe { libc::getuid() };
    format!("/run/user/{uid}/{APP_ID}.sock")
}

pub enum AcquireResult {
    Acquired(SingleInstanceGuard),
    AlreadyRunning,
}

pub struct SingleInstanceGuard {
    _rx: mpsc::Receiver<()>,
    #[cfg(unix)]
    use_namespaced: bool,
}

#[cfg(unix)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.use_namespaced {
            let _ = std::fs::remove_file(socket_path());
        }
    }
}

#[cfg(not(unix))]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {}
}

fn make_name() -> interprocess::local_socket::Name<'static> {
    if GenericNamespaced::is_supported() {
        Box::leak(APP_ID.to_string().into_boxed_str())
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    } else {
        Box::leak(socket_path().into_boxed_str())
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    }
}

pub fn try_acquire() -> anyhow::Result<AcquireResult> {
    if Stream::connect(make_name()).is_ok() {
        return Ok(AcquireResult::AlreadyRunning);
    }

    let use_namespaced = GenericNamespaced::is_supported();

    #[cfg(unix)]
    if !use_namespaced {
        let sock_path = socket_path();
        if let Some(parent) = std::path::Path::new(&sock_path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if std::path::Path::new(&sock_path).exists() {
            std::fs::remove_file(&sock_path)
                .map_err(|e| anyhow::anyhow!("failed to remove stale socket {sock_path}: {e}"))?;
        }
    }

    let listener = match ListenerOptions::new().name(make_name()).create_sync() {
        Ok(l) => l,
        Err(_) => {
            if Stream::connect(make_name()).is_ok() {
                return Ok(AcquireResult::AlreadyRunning);
            }
            return Err(anyhow::anyhow!("failed to create single-instance listener"));
        }
    };

    let (tx, rx) = mpsc::channel::<()>();

    thread::spawn(move || {
        for conn in listener.incoming().filter_map(|c| c.ok()) {
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_ok() {
                let _ = tx.send(());
            }
        }
    });

    Ok(AcquireResult::Acquired(SingleInstanceGuard {
        _rx: rx,
        #[cfg(unix)]
        use_namespaced,
    }))
}
