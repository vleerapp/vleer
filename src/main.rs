#![cfg_attr(
    all(not(test), not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::LazyLock;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod data;
mod media;
mod single_instance;
mod ui;

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
});

fn main() -> anyhow::Result<()> {
    let data_dir = dirs::data_dir()
        .expect("couldn't get data directory")
        .join("vleer");

    std::fs::create_dir_all(&data_dir).ok();

    let log_path = data_dir.join("vleer.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)?;

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(log_file))
                .with_ansi(false),
        )
        .init();

    let skip_single_instance = std::env::args().any(|arg| arg == "--skip-single-instance");

    let _instance_guard = if skip_single_instance {
        None
    } else {
        match crate::single_instance::try_acquire()? {
            crate::single_instance::AcquireResult::Acquired(guard) => Some(guard),
            crate::single_instance::AcquireResult::AlreadyRunning => {
                tracing::warn!("Another Vleer instance is already running. Exiting.");
                return Ok(());
            }
        }
    };

    tracing::info!("Starting application");

    crate::ui::app::run()
}
