use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use interprocess::local_socket::prelude::*;
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(not(unix))]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{ListenerOptions, Stream};

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
    let run_user = PathBuf::from(format!("/run/user/{uid}"));
    if run_user.is_dir() {
        return run_user
            .join(format!("{APP_ID}.sock"))
            .to_string_lossy()
            .into_owned();
    }
    std::env::temp_dir()
        .join(format!("{APP_ID}-{uid}.sock"))
        .to_string_lossy()
        .into_owned()
}

pub enum AcquireResult {
    Acquired(SingleInstanceGuard),
    AlreadyRunning,
}

pub struct SingleInstanceGuard {
    _rx: mpsc::Receiver<()>,
}

#[cfg(unix)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(socket_path());
    }
}

fn make_name() -> interprocess::local_socket::Name<'static> {
    #[cfg(unix)]
    {
        Box::leak(socket_path().into_boxed_str())
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    }
    #[cfg(not(unix))]
    {
        Box::leak(APP_ID.to_string().into_boxed_str())
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    }
}

pub fn try_acquire() -> anyhow::Result<AcquireResult> {
    if Stream::connect(make_name()).is_ok() {
        return Ok(AcquireResult::AlreadyRunning);
    }

    #[cfg(unix)]
    {
        let sock_path = socket_path();
        let path = std::path::Path::new(&sock_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if path.exists() {
            std::fs::remove_file(path)
                .map_err(|e| anyhow::anyhow!("failed to remove stale socket {sock_path}: {e}"))?;
        }
    }

    let listener = match ListenerOptions::new().name(make_name()).create_sync() {
        Ok(l) => l,
        Err(e) => {
            if Stream::connect(make_name()).is_ok() {
                return Ok(AcquireResult::AlreadyRunning);
            }
            return Err(anyhow::anyhow!(
                "failed to create single-instance listener: {e}"
            ));
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

    Ok(AcquireResult::Acquired(SingleInstanceGuard { _rx: rx }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_then_second_instance_detected() {
        let _guard = match try_acquire().expect("first acquire should not error") {
            AcquireResult::Acquired(guard) => guard,
            AcquireResult::AlreadyRunning => return,
        };
        match try_acquire().expect("second acquire should not error") {
            AcquireResult::AlreadyRunning => {}
            AcquireResult::Acquired(_) => panic!("second instance should see the first"),
        }
    }
}
