use gpui::{App, BorrowAppContext, KeyBinding, actions};
use tracing::{debug, info};

use crate::{
    data::config::Config,
    media::{playback::Playback, queue::Queue},
};

actions!(vleer, [Quit]);
actions!(player, [PlayPause, Next, Previous]);
actions!(config, [Reload]);

pub fn register_actions(cx: &mut App) {
    debug!("Registering Actions");
    cx.on_action(quit);
    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);
    cx.on_action(reload_config);

    cx.bind_keys([KeyBinding::new("secondary-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-right", Next, None)]);
    cx.bind_keys([KeyBinding::new("secondary-left", Previous, None)]);
    cx.bind_keys([KeyBinding::new("space", PlayPause, None)]);

    cx.bind_keys([KeyBinding::new("secondary-alt-r", Reload, None)]);
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
    cx.update_global::<Playback, _>(|playback, _cx| {
        playback.play_pause();
    });
}

fn previous(_: &Previous, cx: &mut App) {
    if let Err(e) = Queue::previous(cx) {
        tracing::error!("Failed to go to previous track: {}", e);
    }
}

fn next(_: &Next, cx: &mut App) {
    if let Err(e) = Queue::next(cx) {
        tracing::error!("Failed to go to next track: {}", e);
    }
}

fn reload_config(_: &Reload, cx: &mut App) {
    cx.update_global::<Config, _>(|config, _| {
        if let Err(e) = config.reload() {
            tracing::error!("Failed to save config: {}", e);
        }
    });
}
