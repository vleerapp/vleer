use gpui::{App, BorrowAppContext, KeyBinding, actions};
use tracing::{debug, error, info};

use crate::{
    data::config::Config,
    media::{playback::Playback, queue::Queue},
};

actions!(vleer, [Quit, ReloadConfig]);
actions!(player, [PlayPause, Next, Previous]);

pub fn register_actions(cx: &mut App) {
    cx.on_action(quit);
    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);
    cx.on_action(reload_config);

    cx.bind_keys([KeyBinding::new("secondary-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.bind_keys([KeyBinding::new("alt-right", Next, None)]);
    cx.bind_keys([KeyBinding::new("alt-left", Previous, None)]);
    cx.bind_keys([KeyBinding::new("space", PlayPause, None)]);

    cx.bind_keys([KeyBinding::new("secondary-alt-r", ReloadConfig, None)]);
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
