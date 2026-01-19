#![cfg_attr(
    all(not(test), not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod data;
mod media;
mod ui;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> anyhow::Result<()> {
    let _profiler = dhat::Profiler::new_heap();

    tracing_subscriber::fmt::init();
    tracing::info!("Starting application");

    crate::ui::app::run()
}
