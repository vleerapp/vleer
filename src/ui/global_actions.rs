use gpui::{App, BorrowAppContext, KeyBinding, actions};
use tracing::{debug, error, info};

use crate::{
    data::{config::Config, db::repo::Database, scanner::Scanner},
    media::{playback::Playback, queue::Queue},
};

actions!(vleer, [Quit, ReloadConfig, Scan, ForceScan]);
actions!(player, [PlayPause, Next, Previous]);

pub fn register_actions(cx: &mut App) {
    cx.on_action(quit);
    cx.on_action(reload_config);
    cx.on_action(scan);
    cx.on_action(force_scan);

    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);

    cx.bind_keys([KeyBinding::new("secondary-alt-r", ReloadConfig, None)]);
    cx.bind_keys([KeyBinding::new("secondary-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-r", Scan, None)]);
    cx.bind_keys([KeyBinding::new("secondary-shift-r", ForceScan, None)]);

    cx.bind_keys([KeyBinding::new("alt-right", Next, None)]);
    cx.bind_keys([KeyBinding::new("alt-left", Previous, None)]);
    cx.bind_keys([KeyBinding::new("space", PlayPause, None)]);

    debug!("Actions: {:?}", cx.all_action_names());
}

fn quit(_: &Quit, cx: &mut App) {
    info!("Quitting...");

    cx.update_global::<Config, _>(|config, _| {
        if let Err(e) = config.save() {
            tracing::error!("Failed to save config: {}", e);
        }
    });

    cx.quit();
}

fn play_pause(_: &PlayPause, cx: &mut App) {
    cx.update_global::<Playback, _>(|playback, cx| {
        playback.play_pause(cx);
    });
}

fn previous(_: &Previous, cx: &mut App) {
    cx.update_global::<Queue, _>(|queue, cx| {
        queue.previous(cx);
    });
}

fn next(_: &Next, cx: &mut App) {
    cx.update_global::<Queue, _>(|queue, cx| {
        queue.next(cx);
    });
}

fn reload_config(_: &ReloadConfig, cx: &mut App) {
    cx.update_global::<Config, _>(|config, _cx| {
        if let Err(e) = config.reload() {
            error!("Failed to reload config: {}", e);
        }
    });
}

fn scan(_: &Scan, cx: &mut App) {
    let db = cx.global::<Database>().clone();
    let scanner = cx.global::<Scanner>().clone();

    cx.spawn(async move |_cx| match scanner.scan(&db).await {
        Ok(stats) => {
            info!(
                "Manual scan complete - Scanned: {}, Added: {}, Updated: {}",
                stats.scanned, stats.added, stats.updated
            );
        }
        Err(e) => {
            error!("Manual scan failed: {}", e);
        }
    })
    .detach();
}

fn force_scan(_: &ForceScan, cx: &mut App) {
    let db = cx.global::<Database>().clone();
    let scanner = cx.global::<Scanner>().clone();

    cx.spawn(async move |_cx| match scanner.force_scan(&db).await {
        Ok(stats) => {
            info!(
                "Manual Full scan complete - Scanned: {}, Added: {}, Updated: {}",
                stats.scanned, stats.added, stats.updated
            );
        }
        Err(e) => {
            error!("Manual Full scan failed: {}", e);
        }
    })
    .detach();
}
