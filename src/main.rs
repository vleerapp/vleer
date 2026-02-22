#![cfg_attr(
    all(not(test), not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::sync::LazyLock;

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
    tracing_subscriber::fmt::init();

    let _instance_guard = match crate::single_instance::try_acquire()? {
        crate::single_instance::AcquireResult::Acquired(guard) => guard,
        crate::single_instance::AcquireResult::AlreadyRunning => {
            tracing::warn!("Another Vleer instance is already running. Exiting.");
            return Ok(());
        }
    };

    tracing::info!("Starting application");

    crate::ui::app::run()
}
