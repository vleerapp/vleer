use gpui::{App, BorrowAppContext, KeyBinding, actions};
use tracing::{debug, error, info};

use crate::{
    data::{config::Config, db::repo::Database, scanner::Scanner},
    media::playback::Playback,
    updater::{Updater, run_check_in_background},
};

actions!(
    vleer,
    [Quit, ReloadConfig, Scan, ForceScan, CheckForUpdates]
);
actions!(player, [PlayPause, Next, Previous]);

pub fn register_actions(cx: &mut App) {
    cx.on_action(quit);
    cx.on_action(reload_config);
    cx.on_action(scan);
    cx.on_action(force_scan);
    cx.on_action(check_for_updates);

    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);

    cx.bind_keys([KeyBinding::new("secondary-alt-r", ReloadConfig, None)]);
    cx.bind_keys([KeyBinding::new("secondary-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-r", Scan, None)]);
    cx.bind_keys([KeyBinding::new("secondary-shift-r", ForceScan, None)]);
    cx.bind_keys([KeyBinding::new("secondary-u", CheckForUpdates, None)]);

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
    cx.update_global::<Playback, _>(|playback, cx| {
        playback.previous(cx);
    });
}

fn next(_: &Next, cx: &mut App) {
    cx.update_global::<Playback, _>(|playback, cx| {
        playback.next(cx);
    });
}

fn reload_config(_: &ReloadConfig, cx: &mut App) {
    cx.update_global::<Config, _>(|config, _cx| {
        if let Err(e) = config.reload() {
            error!("Failed to reload config: {}", e);
        }
    });

    use crate::status::StatusColor;
    let warning = cx.global::<Config>().parse_warning.clone();
    if let Some(warning) = warning {
        crate::ui::layout::navbar::status().set(
            "config.parse_error",
            warning,
            None,
            StatusColor::Destructive,
        );
    } else {
        crate::ui::layout::navbar::status().clear("config.parse_error");
    }

    let db = cx.global::<Database>().clone();
    let scanner = cx.global::<Scanner>().clone();

    cx.spawn(async move |_cx| match scanner.scan(&db).await {
        Ok(stats) => {
            if stats.missing > 0 {
                crate::ui::layout::navbar::status().set(
                    "scanner.missing",
                    format!(
                        "{} song{} missing from disk",
                        stats.missing,
                        if stats.missing == 1 { "" } else { "s" }
                    ),
                    None,
                    StatusColor::Warning,
                );
            } else {
                crate::ui::layout::navbar::status().clear("scanner.missing");
            }
        }
        Err(e) => {
            error!("Scan after config reload failed: {}", e);
        }
    })
    .detach();
}

fn scan(_: &Scan, cx: &mut App) {
    let db = cx.global::<Database>().clone();
    let scanner = cx.global::<Scanner>().clone();

    cx.spawn(async move |_cx| match scanner.scan(&db).await {
        Ok(stats) => {
            use crate::status::StatusColor;
            if stats.missing > 0 {
                crate::ui::layout::navbar::status().set(
                    "scanner.missing",
                    format!(
                        "{} song{} missing from disk",
                        stats.missing,
                        if stats.missing == 1 { "" } else { "s" }
                    ),
                    None,
                    StatusColor::Warning,
                );
            } else {
                crate::ui::layout::navbar::status().clear("scanner.missing");
            }
        }
        Err(e) => {
            error!("Manual scan failed: {}", e);
        }
    })
    .detach();
}

fn check_for_updates(_: &CheckForUpdates, cx: &mut App) {
    let updater = cx.global::<Updater>().clone();
    run_check_in_background(updater, cx.background_executor());
}

fn force_scan(_: &ForceScan, cx: &mut App) {
    let db = cx.global::<Database>().clone();
    let scanner = cx.global::<Scanner>().clone();

    cx.spawn(async move |_cx| match scanner.force_scan(&db).await {
        Ok(stats) => {
            use crate::status::StatusColor;
            if stats.missing > 0 {
                crate::ui::layout::navbar::status().set(
                    "scanner.missing",
                    format!(
                        "{} song{} missing from disk",
                        stats.missing,
                        if stats.missing == 1 { "" } else { "s" }
                    ),
                    None,
                    StatusColor::Warning,
                );
            } else {
                crate::ui::layout::navbar::status().clear("scanner.missing");
            }
        }
        Err(e) => {
            error!("Manual Full scan failed: {}", e);
        }
    })
    .detach();
}
