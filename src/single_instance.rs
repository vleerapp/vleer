use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc;
use std::thread;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Stream};

const APP_ID: &str = "vleer";

pub enum AcquireResult {
    Acquired(SingleInstanceGuard),
    AlreadyRunning,
}

pub struct SingleInstanceGuard {
    _rx: mpsc::Receiver<()>,
    #[cfg(unix)]
    app_id: String,
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(format!("/tmp/{}.sock", self.app_id));
        }
    }
}

fn make_name() -> interprocess::local_socket::Name<'static> {
    if GenericNamespaced::is_supported() {
        Box::leak(APP_ID.to_string().into_boxed_str())
            .to_ns_name::<GenericNamespaced>()
            .unwrap()
    } else {
        Box::leak(format!("/tmp/{APP_ID}.sock").into_boxed_str())
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    }
}

pub fn try_acquire() -> anyhow::Result<AcquireResult> {
    if Stream::connect(make_name()).is_ok() {
        return Ok(AcquireResult::AlreadyRunning);
    }

    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(format!("/tmp/{APP_ID}.sock"));
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let listener = ListenerOptions::new()
        .name(make_name())
        .create_sync()
        .map_err(|e| anyhow::anyhow!("failed to create single-instance listener: {e}"))?;

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
        app_id: APP_ID.to_string(),
    }))
}
