use std::sync::Arc;

use anyhow::Result;
use gpui::{App, BorrowAppContext, Global};
use tracing::debug;

use crate::data::config::Config;
use crate::data::types::Song;
use crate::media::media_keys::MediaKeyHandler;
use crate::media::playback::Playback;

pub struct Queue {
    items: Vec<Arc<Song>>,
    current_index: Option<usize>,
    shuffle: bool,
    repeat: RepeatMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl Global for Queue {}

impl Queue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            current_index: None,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(Queue::new());
    }

    pub fn add(&mut self, song: Arc<Song>) {
        self.items.push(song);
        if self.current_index.is_none() && !self.items.is_empty() {
            self.current_index = Some(0);
        }
        debug!("Added item to queue. Queue size: {}", self.items.len());
    }

    pub fn add_many(&mut self, songs: Vec<Arc<Song>>) {
        let was_empty = self.items.is_empty();
        let count = songs.len();
        self.items.extend(songs);
        if was_empty && !self.items.is_empty() {
            self.current_index = Some(0);
        }
        debug!(
            "Added {} items to queue. Queue size: {}",
            count,
            self.items.len()
        );
    }

    pub fn clear_and_queue_songs(&mut self, songs: &[Arc<Song>]) {
        self.clear();
        self.add_many(songs.to_vec());
    }

    pub fn current(&self) -> Option<&Arc<Song>> {
        self.current_index.and_then(|idx| self.items.get(idx))
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn next(cx: &mut App) -> Result<()> {
        let next_song = cx.update_global::<Queue, _>(|queue, _cx| {
            if queue.items.is_empty() {
                return None;
            }

            match queue.repeat {
                RepeatMode::One => {
                    return queue.current().cloned();
                }
                RepeatMode::All => {
                    if let Some(idx) = queue.current_index {
                        queue.current_index = Some((idx + 1) % queue.items.len());
                    } else {
                        queue.current_index = Some(0);
                    }
                }
                RepeatMode::Off => {
                    if let Some(idx) = queue.current_index {
                        if idx + 1 < queue.items.len() {
                            queue.current_index = Some(idx + 1);
                        } else {
                            return None;
                        }
                    } else {
                        queue.current_index = Some(0);
                    }
                }
            }

            debug!("Moved to next track. Index: {:?}", queue.current_index);
            queue.current().cloned()
        });

        if let Some(song) = next_song {
            let config = cx.global::<Config>().clone();
            cx.update_global::<Playback, _>(|playback, cx| {
                playback
                    .open(&song.file_path, &config, song.track_lufs)
                    .ok();
                playback.play(cx);
                debug!("Playing next track");
            });

            MediaKeyHandler::update_playback(cx);
            Ok(())
        } else {
            debug!("No next track");
            Ok(())
        }
    }

    pub fn previous(cx: &mut App) -> Result<()> {
        let prev_song = cx.update_global::<Queue, _>(|queue, _cx| {
            if queue.items.is_empty() {
                return None;
            }

            match queue.repeat {
                RepeatMode::One => {
                    return queue.current().cloned();
                }
                RepeatMode::All => {
                    if let Some(idx) = queue.current_index {
                        if idx == 0 {
                            queue.current_index = Some(queue.items.len() - 1);
                        } else {
                            queue.current_index = Some(idx - 1);
                        }
                    } else {
                        queue.current_index = Some(0);
                    }
                }
                RepeatMode::Off => {
                    if let Some(idx) = queue.current_index {
                        if idx > 0 {
                            queue.current_index = Some(idx - 1);
                        } else {
                            return None;
                        }
                    } else {
                        queue.current_index = Some(0);
                    }
                }
            }

            debug!("Moved to previous track. Index: {:?}", queue.current_index);
            queue.current().cloned()
        });

        if let Some(song) = prev_song {
            let config = cx.global::<Config>().clone();
            cx.update_global::<Playback, _>(|playback, cx| {
                playback.open(&song.file_path, &config, song.track_lufs).ok();
                playback.play(cx);
                debug!("Playing previous track");
            });

            MediaKeyHandler::update_playback(cx);
            Ok(())
        } else {
            debug!("No previous track");
            Ok(())
        }
    }

    pub fn jump_to(&mut self, index: usize) -> Option<&Arc<Song>> {
        if index < self.items.len() {
            self.current_index = Some(index);
            debug!("Jumped to index {}", index);
            self.current()
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.current_index = None;
        debug!("Queue cleared");
    }

    pub fn remove(&mut self, index: usize) -> Option<Arc<Song>> {
        if index < self.items.len() {
            let item = self.items.remove(index);

            if let Some(current) = self.current_index {
                if current == index {
                    if self.items.is_empty() {
                        self.current_index = None;
                    } else if current >= self.items.len() {
                        self.current_index = Some(self.items.len() - 1);
                    }
                } else if current > index {
                    self.current_index = Some(current - 1);
                }
            }

            debug!(
                "Removed item at index {}. Queue size: {}",
                index,
                self.items.len()
            );
            Some(item)
        } else {
            None
        }
    }

    pub fn items(&self) -> &[Arc<Song>] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        debug!("Shuffle: {}", self.shuffle);
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        self.shuffle = shuffle;
        debug!("Shuffle set to: {}", shuffle);
    }

    pub fn is_shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        debug!("Repeat mode: {:?}", self.repeat);
    }

    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
        debug!("Repeat mode set to: {:?}", mode);
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.repeat
    }

    pub fn has_next(&self) -> bool {
        if self.items.is_empty() {
            return false;
        }

        match self.repeat {
            RepeatMode::One | RepeatMode::All => true,
            RepeatMode::Off => {
                if let Some(idx) = self.current_index {
                    idx + 1 < self.items.len()
                } else {
                    !self.items.is_empty()
                }
            }
        }
    }

    pub fn has_previous(&self) -> bool {
        if self.items.is_empty() {
            return false;
        }

        match self.repeat {
            RepeatMode::One | RepeatMode::All => true,
            RepeatMode::Off => {
                if let Some(idx) = self.current_index {
                    idx > 0
                } else {
                    false
                }
            }
        }
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}
