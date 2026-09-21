use gpui::{App, BorrowAppContext, Context, KeyBinding, Window, actions};
use tracing::{debug, error, info};

use crate::{
    data::{config::Config, db::repo::Database, scanner::Scanner},
    media::playback::Playback,
    ui::app::MainWindow,
    updater::{Updater, run_check_in_background},
};

actions!(
    vleer,
    [Quit, ReloadConfig, Scan, ForceScan, CheckForUpdates]
);
actions!(navigation, [GoBack, GoForward]);
actions!(player, [PlayPause, Next, Previous]);

pub fn register_actions(cx: &mut App) {
    cx.on_action(quit);
    cx.on_action(reload_config);
    cx.on_action(scan);
    cx.on_action(force_scan);
    cx.on_action(check_for_updates);

    cx.on_action(go_back);
    cx.on_action(go_forward);
    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);

    cx.bind_keys([KeyBinding::new("secondary-alt-r", ReloadConfig, None)]);
    cx.bind_keys([KeyBinding::new("secondary-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-r", Scan, None)]);
    cx.bind_keys([KeyBinding::new("secondary-shift-r", ForceScan, None)]);
    cx.bind_keys([KeyBinding::new("secondary-u", CheckForUpdates, None)]);

    cx.bind_keys([KeyBinding::new("secondary-alt-right", Next, None)]);
    cx.bind_keys([KeyBinding::new("secondary-alt-left", Previous, None)]);

    let (back_key, forward_key) = if cfg!(target_os = "macos") {
        ("cmd-left", "cmd-right")
    } else {
        ("alt-left", "alt-right")
    };
    cx.bind_keys([KeyBinding::new(back_key, GoBack, None)]);
    cx.bind_keys([KeyBinding::new(forward_key, GoForward, None)]);
    cx.bind_keys([KeyBinding::new("space", PlayPause, None)]);

    debug!("Actions: {:?}", cx.all_action_names());
}

fn quit(_: &Quit, cx: &mut App) {
    info!("Quitting...");

    cx.update_global::<Config, _>(|config, _| {
        if let Err(e) = config.save() {
            error!("Failed to save config: {}", e);
        }
    });

    cx.quit();
}

fn navigate(cx: &mut App, f: fn(&mut MainWindow, &mut Window, &mut Context<MainWindow>)) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, cx| {
                if let Some(Some(root)) = window.root::<MainWindow>() {
                    root.update(cx, |view, cx| f(view, window, cx));
                }
            })
            .ok();
    });
}

fn go_back(_: &GoBack, cx: &mut App) {
    debug!("GoBack");
    navigate(cx, MainWindow::go_back);
}

fn go_forward(_: &GoForward, cx: &mut App) {
    debug!("GoForward");
    navigate(cx, MainWindow::go_forward);
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

    let config = cx.global::<Config>().clone();
    let (eq_enabled, gains, q_values) = {
        let eq = &config.get().equalizer;
        (eq.enabled, eq.gains.clone(), eq.q_values.clone())
    };
    cx.update_global::<Playback, _>(|playback, _cx| {
        playback.apply_config(&config);
        if eq_enabled {
            playback.apply_eq_settings(&gains, &q_values);
        } else {
            playback.set_eq_enabled(false);
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
    let channel = cx.global::<Config>().get().updater.channel;
    run_check_in_background(updater, channel, cx.background_executor());
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
