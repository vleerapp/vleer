use gpui::{App, BorrowAppContext, KeyBinding, actions};
use tracing::{debug, info};

use crate::{
    data::config::Config,
    media::{playback::Playback, queue::Queue},
    data::state::State,
};

actions!(vleer, [Quit, ReloadConfig, ReloadState]);
actions!(player, [PlayPause, Next, Previous]);

pub fn register_actions(cx: &mut App) {
    debug!("Registering Actions");
    cx.on_action(quit);
    cx.on_action(play_pause);
    cx.on_action(next);
    cx.on_action(previous);
    cx.on_action(reload_config);
    cx.on_action(reload_state);

    cx.bind_keys([KeyBinding::new("secondary-w", Quit, None)]);
    cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
    cx.bind_keys([KeyBinding::new("alt-right", Next, None)]);
    cx.bind_keys([KeyBinding::new("alt-left", Previous, None)]);
    cx.bind_keys([KeyBinding::new("space", PlayPause, Some("!TextInput"))]);

    cx.bind_keys([KeyBinding::new("secondary-alt-r", ReloadConfig, None)]);
    cx.bind_keys([KeyBinding::new("secondary-r", ReloadState, None)]);
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
    if let Err(e) = Queue::previous(cx) {
        tracing::error!("Failed to go to previous track: {}", e);
    }
}

fn next(_: &Next, cx: &mut App) {
    if let Err(e) = Queue::next(cx) {
        tracing::error!("Failed to go to next track: {}", e);
    }
}

fn reload_config(_: &ReloadConfig, cx: &mut App) {
    if let Err(e) = Config::reload(cx) {
        tracing::error!("Failed to reload config: {}", e);
    }
}

fn reload_state(_: &ReloadState, cx: &mut App) {
    State::prepare(cx)
}
